import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import {
  ChevronLeft,
  ChevronRight,
  Circle,
  CircleCheck,
  CircleMinus,
  Loader,
} from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { Session } from '../../api/types'
import { keys, useAgents, useTask } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { Alert, Empty, Skeleton } from '../../components/primitives'
import { RoomView } from '../rooms/Room'
import { useIsPhone } from '../shell/viewport'
import {
  doneCount,
  pileOf,
  sentenceOf,
  titleOf,
  type Task,
  type TaskItem,
} from './status'
import { TaskMark } from './TaskMark'
import './tasks.css'

/** One task: its private room, with the plan beside it on a desk and above it on a phone. */
export function TaskView({
  taskId,
  session,
}: {
  taskId: string
  session: Session
}) {
  const task = useTask(taskId)
  const phone = useIsPhone()
  if (task.isPending)
    return (
      <div className="page__body">
        <Skeleton lines={3} width="50%" />
      </div>
    )
  if (!task.data)
    return (
      <Empty title="This task is unavailable.">
        It belongs to someone else, or it no longer exists.
      </Empty>
    )
  return (
    <section className="task" aria-label={titleOf(task.data)}>
      <TaskHead task={task.data} />
      <div className="task__body">
        <div className="task__room">
          <RoomView
            roomId={task.data.room_id}
            session={session}
            label="Task room"
            draftKey={`cubebot:${session.id}:task:${task.data.id}`}
            placeholder="Write to the room · @name reaches an agent"
            note="The orchestrator reads everything here; @name goes straight to that agent."
            empty="The orchestrator is on its way."
            task
          />
        </div>
        <Plan task={task.data} phone={phone} />
      </div>
    </section>
  )
}

function TaskHead({ task }: { task: Task }) {
  const client = useQueryClient()
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const running = pileOf(task) !== 'finished'
  const stop = async () => {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.POST('/api/tasks/{task}/stop', {
          params: { path: { task: task.id } },
        }),
      )
      await client.invalidateQueries({ queryKey: keys.tasks })
      await client.invalidateQueries({ queryKey: ['task', task.id] })
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <header className="task__head">
        <Link
          to="/tasks"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="All tasks"
          title="All tasks"
        >
          <ChevronLeft />
        </Link>
        <TaskMark task={task} />
        <h1>{titleOf(task)}</h1>
        <span className="task__state">{sentenceOf(task)}</span>
        {running && (
          <button
            type="button"
            className="btn btn--sm"
            disabled={busy}
            onClick={() => void stop()}
          >
            Stop
          </button>
        )}
      </header>
      <Alert>{error}</Alert>
    </>
  )
}

function ItemMark({ status }: { status: string }) {
  if (status === 'done') return <CircleCheck aria-label="Done" />
  if (status === 'doing')
    return <Loader className="spin" aria-label="In progress" />
  if (status === 'skipped') return <CircleMinus aria-label="Skipped" />
  return <Circle aria-label="Not started" />
}

function Plan({ task, phone }: { task: Task; phone: boolean }) {
  const done = doneCount(task)
  const agents = useAgents()
  const body = (
    <div className="plan__body">
      <h2 className="plan__title">
        Plan
        {task.items.length > 0 && (
          <span className="faint tabular">
            {done} of {task.items.length}
          </span>
        )}
      </h2>
      {task.items.length > 0 && (
        <div
          className="plan__meter"
          role="progressbar"
          aria-label="Items finished"
          aria-valuemin={0}
          aria-valuemax={task.items.length}
          aria-valuenow={done}
        >
          <span style={{ width: `${(done / task.items.length) * 100}%` }} />
        </div>
      )}
      {task.items.length === 0 && (
        <p className="plan__empty">
          The orchestrator is working out who should do what.
        </p>
      )}
      {task.agents.map((member) => {
        const agent = agents.data?.find((item) => item.id === member.id)
        const mine = task.items.filter((item) => item.agent_id === member.id)
        if (mine.length === 0) return null
        return (
          <div className="plan__agent" key={member.id}>
            <Link
              to="/agents/$agentId"
              params={{ agentId: member.id }}
              className="plan__who"
            >
              <Cube
                avatar={agent?.avatar ?? null}
                mood={agent ? moodOf(agent) : 'idle'}
                size={20}
              />
              {member.name}
            </Link>
            <ul className="plan__items">
              {mine.map((item) => (
                <Item key={item.id} item={item} />
              ))}
            </ul>
          </div>
        )
      })}
    </div>
  )
  if (!phone) return <aside className="task__plan">{body}</aside>
  return (
    <details className="task__plan">
      <summary>
        <ChevronRight aria-hidden="true" />
        Plan
        {task.items.length > 0 && (
          <span className="faint tabular">
            {done} of {task.items.length} done
          </span>
        )}
      </summary>
      {body}
    </details>
  )
}

function Item({ item }: { item: TaskItem }) {
  return (
    <li className="plan__item" data-status={item.status}>
      <ItemMark status={item.status} />
      <span>
        {item.title}
        {item.note && <span className="plan__note">{item.note}</span>}
      </span>
    </li>
  )
}
