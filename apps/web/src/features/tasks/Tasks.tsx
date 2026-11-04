import { Link, useNavigate } from '@tanstack/react-router'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, ok } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { keys, useAgents, useSession, useTasks } from '../../app/queries'
import { taskHover } from '../../components/agentHover'
import { Cube, moodOf } from '../../components/Cube'
import { Alert, Skeleton } from '../../components/primitives'
import { Composer } from '../chat/Composer'
import '../chat/chat.css'
import { ORCHESTRATOR_AVATAR } from './orchestrator'
import {
  doneCount,
  piles,
  sentenceOf,
  titleOf,
  type Pile,
  type Task,
} from './status'
import { TaskMark } from './TaskMark'
import './tasks.css'

const FINISHED_SHOWN = 8
const PILES: [Pile, string][] = [
  ['needs_you', 'Needs you'],
  ['working', 'Working'],
  ['finished', 'Finished'],
]

/** Say what you want done; the orchestrator takes it from there. */
export function Tasks() {
  const session = useSession()
  const agents = useAgents()
  const tasks = useTasks()
  const client = useQueryClient()
  const navigate = useNavigate()
  const crew = agents.data ?? []
  const noAgents = agents.isSuccess && crew.length === 0
  return (
    <div className="tasks">
      <div className="tasks__column">
        <header className="tasks__hello">
          <span className="tasks__stack" aria-hidden="true">
            <Cube avatar={ORCHESTRATOR_AVATAR} size={56} follow />
          </span>
          <h1>What do you want done?</h1>
          <p>
            Say it once. The orchestrator picks the right agents and gives each
            their part. They report back here as they work.
          </p>
        </header>
        <div className="tasks__ask">
          {session.data && (
            <Composer
              draftKey={`cubebot:${session.data.id}:new-task`}
              label="Start the task"
              placeholder="Describe the outcome you want"
              disabled={noAgents}
              note={<Crew agents={crew} />}
              send={async (request_id, request) => {
                const task = await ok(
                  api.POST('/api/tasks', { body: { request_id, request } }),
                )
                await client.invalidateQueries({ queryKey: keys.tasks })
                await navigate({
                  to: '/tasks/$taskId',
                  params: { taskId: task.id },
                })
              }}
            />
          )}
          {noAgents && (
            <Alert>
              There is no agent you can hand work to yet. An administrator can
              create one or give you access.
            </Alert>
          )}
        </div>
        {tasks.isPending && <Skeleton lines={3} />}
        {tasks.isError && (
          <Alert>Your tasks could not be read. Retry shortly.</Alert>
        )}
        {tasks.data && <TaskPiles tasks={tasks.data} agents={crew} />}
      </div>
    </div>
  )
}

/** Who the orchestrator can choose from, so the box says who is listening. */
function Crew({ agents }: { agents: AgentSummary[] }) {
  if (agents.length === 0) return null
  const shown = agents.slice(0, 6)
  return (
    <span className="tasks__crew">
      <span className="tasks__crew-cubes" aria-hidden="true">
        {shown.map((agent) => (
          <Cube
            key={agent.id}
            avatar={agent.avatar}
            mood={moodOf(agent)}
            size={18}
          />
        ))}
      </span>
      {agents.length === 1
        ? `${agents[0]?.name} is ready`
        : `${agents.length} agents to choose from`}
    </span>
  )
}

function TaskPiles({
  tasks,
  agents,
}: {
  tasks: Task[]
  agents: AgentSummary[]
}) {
  const grouped = piles(tasks)
  const [all, setAll] = useState(false)
  return (
    <>
      {PILES.map(([pile, label]) => {
        const here = grouped[pile]
        if (here.length === 0) return null
        const shown =
          pile === 'finished' && !all ? here.slice(0, FINISHED_SHOWN) : here
        return (
          <section className="tasks__pile" data-pile={pile} key={pile}>
            <h2 className="tasks__pile-head">
              {label}
              <span className="tabular">{here.length}</span>
            </h2>
            {shown.map((task) => (
              <TaskRow key={task.id} task={task} pile={pile} agents={agents} />
            ))}
            {shown.length < here.length && (
              <button
                type="button"
                className="link-btn small tasks__older"
                onClick={() => setAll(true)}
              >
                Show {here.length - shown.length} more
              </button>
            )}
          </section>
        )
      })}
    </>
  )
}

function TaskRow({
  task,
  pile,
  agents,
}: {
  task: Task
  pile: Pile
  agents: AgentSummary[]
}) {
  return (
    <Link
      to="/tasks/$taskId"
      params={{ taskId: task.id }}
      className="task-row"
      data-pile={pile}
      {...taskHover(task.id)}
    >
      <TaskMark task={task} />
      <span className="task-row__title">{titleOf(task)}</span>
      <span className="task-row__sentence">{sentenceOf(task)}</span>
      <span className="task-row__cubes" aria-hidden="true">
        {task.agents.slice(0, 3).map((member) => {
          const agent = agents.find((item) => item.id === member.id)
          return (
            <Cube
              key={member.id}
              avatar={agent?.avatar ?? null}
              mood={agent ? moodOf(agent) : 'idle'}
              size={20}
            />
          )
        })}
      </span>
      {/* Touch has no hover card, so the row itself says how far the plan got. */}
      {task.items.length > 0 && (
        <span className="plan__meter task-row__meter" aria-hidden="true">
          <span
            style={{ width: `${(doneCount(task) / task.items.length) * 100}%` }}
          />
        </span>
      )}
    </Link>
  )
}
