import { Phone, PhoneOff } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { AgentSummary } from '../../api/types'
import { Cube, type Mood } from '../../components/Cube'
import { clock, type CallState, type Line, type Phase } from './call'
import { useCall } from './useCall'
import './call.css'

const SAYS: Record<Phase, string> = {
  connecting: 'Calling…',
  listening: 'Listening',
  thinking: 'Thinking…',
  speaking: 'Speaking',
}
const MOODS: Record<Phase, Mood> = {
  connecting: 'waiting',
  listening: 'idle',
  thinking: 'working',
  speaking: 'idle',
}

export function CallButton({ agent }: { agent: AgentSummary }) {
  const [context, setContext] = useState<AudioContext | null>(null)
  return (
    <>
      <button
        type="button"
        className="btn btn--sm"
        aria-label="Call"
        onClick={() => {
          const audio = new AudioContext()
          void audio.resume()
          setContext(audio)
        }}
      >
        <Phone />
        <span className="agent-head__word">Call</span>
      </button>
      {context && (
        <CallScreen
          agent={agent}
          context={context}
          onHangUp={() => {
            void context.close()
            setContext(null)
          }}
        />
      )}
    </>
  )
}

function Elapsed({ since }: { since: number }) {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  return (
    <span className="call-screen__clock">
      {clock(Math.max(0, Math.floor((now - since) / 1000)))}
    </span>
  )
}

function Words({ line, agent }: { line: Line; agent: string }) {
  return (
    <li className={`call-line call-line--${line.who}`}>
      <span className="call-line__who">
        {line.who === 'you' ? 'You' : agent}
      </span>
      {line.text && <p>{line.text}</p>}
      {line.note && <p className="call-line__note">{line.note}</p>}
    </li>
  )
}

function saying(state: CallState, hearing: boolean, phase: Phase) {
  if (state.preparing) return 'Getting speech ready…'
  if (state.ready && !hearing) return 'Waiting for the microphone…'
  return SAYS[phase]
}

function Preparing({ share }: { share: number }) {
  return (
    <div className="call-screen__prepare">
      <progress max={1} value={share} aria-label="Downloading speech models" />
      <p className="small faint">
        The first call downloads the speech models, once. Later calls start at
        once and work offline.
      </p>
    </div>
  )
}

function CallScreen({
  agent,
  context,
  onHangUp,
}: {
  agent: AgentSummary
  context: AudioContext
  onHangUp: () => void
}) {
  const stage = useRef<HTMLDivElement>(null)
  const words = useRef<HTMLOListElement>(null)
  const hangUp = useRef<HTMLButtonElement>(null)
  const dialog = useRef<HTMLDialogElement>(null)
  const { state, phase, hearing, ended, started } = useCall(
    agent.id,
    context,
    stage,
  )
  // Modal, so the console behind it is inert; a call only ends by hanging up.
  useEffect(() => {
    dialog.current?.showModal()
    hangUp.current?.focus()
  }, [])
  useEffect(() => {
    const list = words.current
    if (list) list.scrollTop = list.scrollHeight
  }, [state.lines, state.partial])
  const said = ended || state.notice
  return (
    <dialog
      ref={dialog}
      className="call-screen"
      aria-label={`Call with ${agent.name}`}
      onCancel={(event) => event.preventDefault()}
      data-phase={ended ? 'ended' : phase}
    >
      <div className="call-screen__stage" ref={stage}>
        <div className="call-screen__cube">
          <Cube avatar={agent.avatar} mood={MOODS[phase]} size={148} follow />
        </div>
        <h1>{agent.name}</h1>
        <p className="call-screen__state" role="status">
          {ended ? 'Call ended' : saying(state, hearing, phase)}
          {state.ready && !ended && <Elapsed since={started.current} />}
        </p>
        {state.preparing && <Preparing share={state.preparing.share} />}
      </div>
      <ol
        className="call-screen__words"
        ref={words}
        aria-label="Transcript so far"
      >
        {state.lines.map((line) => (
          <Words key={line.id} line={line} agent={agent.name} />
        ))}
        {state.partial && (
          <Words
            line={{ id: 'partial', who: 'you', text: state.partial }}
            agent={agent.name}
          />
        )}
      </ol>
      {said && (
        <p className="call-screen__problem" role="alert">
          {said}
        </p>
      )}
      <div className="call-screen__actions">
        <button
          ref={hangUp}
          type="button"
          className={ended ? 'btn' : 'call-screen__hangup'}
          onClick={onHangUp}
        >
          {ended ? (
            'Back to the conversation'
          ) : (
            <>
              <PhoneOff aria-hidden="true" />
              Hang up
            </>
          )}
        </button>
      </div>
    </dialog>
  )
}
