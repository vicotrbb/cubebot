import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import type { AgentSummary } from '../../api/types'
import { keys } from '../../app/queries'
import { Alert, Field, Skeleton, Status } from '../../components/primitives'
import { SaveBar, Section } from '../platform/Section'
import { PROVIDER_OPTIONS } from './call'
import { ProviderField, VoiceChoice } from './SpeechFields'
import './call.css'

type Provider = components['schemas']['Provider']
type Settings = components['schemas']['SpeechSettings']

const name = (provider: Provider) =>
  PROVIDER_OPTIONS.find(([id]) => id === provider)?.[1] ?? provider

const useSpeech = () =>
  useQuery({
    queryKey: ['speech'],
    queryFn: () => ok(api.GET('/api/speech')),
  })

export function PlatformSpeech() {
  const speech = useSpeech()
  if (speech.isPending) return <Skeleton lines={4} width="62ch" />
  if (speech.isError)
    return <Alert>The speech settings could not be read. Retry shortly.</Alert>
  return <SpeechForm settings={speech.data} />
}

function SpeechForm({ settings }: { settings: Settings }) {
  const client = useQueryClient()
  const [stt, setStt] = useState(settings.stt_provider)
  const [tts, setTts] = useState(settings.tts_provider)
  const [voice, setVoice] = useState(settings.voice)
  const [key, setKey] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const save = async (forget: boolean) => {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await ok(
        api.PUT('/api/speech', {
          body: {
            stt_provider: stt,
            tts_provider: tts,
            voice,
            elevenlabs_key: key || null,
            forget_elevenlabs_key: forget,
          },
        }),
      )
      setKey('')
      setNotice('Saved. New calls use these settings.')
      await client.invalidateQueries({ queryKey: ['speech'] })
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  const needsKey =
    (stt === 'elevenlabs' || tts === 'elevenlabs') &&
    !settings.elevenlabs_configured &&
    !key
  return (
    <form
      className="settings"
      onSubmit={(event) => {
        event.preventDefault()
        void save(false)
      }}
    >
      <Section
        title="Hearing and speaking"
        description={
          <>
            <p>
              How every agent hears and speaks on a call, unless its About tab
              says otherwise.
            </p>
            <p>
              Embedded runs on this installation and works offline. OpenAI uses
              the key under AI providers.
            </p>
          </>
        }
      >
        <div className="settings__pair">
          <ProviderField
            label="Speech to text"
            value={stt}
            onChange={(provider) => provider && setStt(provider)}
          />
          <ProviderField
            label="Text to speech"
            value={tts}
            onChange={(provider) => provider && setTts(provider)}
          />
        </div>
      </Section>
      <Section
        title="How agents sound"
        description="The voice agents answer in. Each agent can pick its own."
      >
        <VoiceChoice
          value={voice}
          onChange={(chosen) => chosen && setVoice(chosen)}
        />
      </Section>
      <Section
        title="ElevenLabs"
        description="Only needed when ElevenLabs hears or speaks for agents. The key is encrypted on the server and never shown again."
      >
        <Field
          label="API key"
          hint={settings.elevenlabs_configured ? 'A key is saved' : undefined}
        >
          <input
            type="password"
            autoComplete="off"
            placeholder={
              settings.elevenlabs_configured
                ? 'Paste a new key to replace it'
                : ''
            }
            value={key}
            onChange={(event) => setKey(event.target.value)}
          />
        </Field>
        {needsKey && (
          <p className="small faint">
            ElevenLabs needs a key before a call can use it.
          </p>
        )}
      </Section>
      <SaveBar>
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
        <div className="actions">
          <button className="btn btn--primary" disabled={busy || needsKey}>
            {busy ? 'Saving…' : 'Save speech settings'}
          </button>
          {settings.elevenlabs_configured && (
            <button
              type="button"
              className="btn btn--ghost"
              disabled={busy}
              onClick={() => void save(true)}
            >
              Forget ElevenLabs key
            </button>
          )}
        </div>
      </SaveBar>
    </form>
  )
}

export function AgentSpeechSection({ agent }: { agent: AgentSummary }) {
  const client = useQueryClient()
  const speech = useSpeech()
  const [stt, setStt] = useState(agent.stt_provider ?? null)
  const [tts, setTts] = useState(agent.tts_provider ?? null)
  const [voice, setVoice] = useState(agent.voice ?? null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const platform = speech.data
  return (
    <section className="panel__section">
      <h2>Speech</h2>
      <p className="panel__lead">
        How {agent.name} hears and speaks on a call. Applies from the next call.
      </p>
      <form
        className="form"
        onSubmit={async (event) => {
          event.preventDefault()
          setBusy(true)
          setError('')
          setNotice('')
          try {
            await ok(
              api.PUT('/api/agents/{id}/speech', {
                params: { path: { id: agent.id } },
                body: { stt_provider: stt, tts_provider: tts, voice },
              }),
            )
            await client.invalidateQueries({ queryKey: keys.agents })
            setNotice('Saved.')
          } catch (caught) {
            setError(failure(caught))
          } finally {
            setBusy(false)
          }
        }}
      >
        <ProviderField
          label="Speech to text"
          value={stt}
          onChange={setStt}
          inherit={`Platform (${platform ? name(platform.stt_provider) : '…'})`}
        />
        <ProviderField
          label="Text to speech"
          value={tts}
          onChange={setTts}
          inherit={`Platform (${platform ? name(platform.tts_provider) : '…'})`}
        />
        <VoiceChoice
          value={voice}
          onChange={setVoice}
          inherit={`Platform (${platform?.voice === 'male' ? 'Male' : 'Female'})`}
        />
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
        <div className="actions">
          <button className="btn btn--primary btn--sm" disabled={busy}>
            {busy ? 'Saving…' : 'Save speech'}
          </button>
        </div>
      </form>
    </section>
  )
}
