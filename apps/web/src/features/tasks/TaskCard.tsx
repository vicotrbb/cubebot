import { useAgents, useTasks } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { when } from '../../components/primitives'
import { ORCHESTRATOR_AVATAR } from './orchestrator'
import { doneCount, headlineOf, pileOf, planWindow, titleOf } from './status'
import { ItemMark, TaskMark } from './TaskMark'
import './tasks.css'

export function TaskCard({
  id,
  ref,
}: {
  id: string
  ref: React.Ref<HTMLDivElement>
}) {
  const tasks = useTasks()
  const agents = useAgents()
  const task = tasks.data?.find((item) => item.id === id)
  if (!task) return null
  const done = doneCount(task)
  const total = task.items.length
  const { shown, before, after } = planWindow(task.items)
  const pile = pileOf(task)
  const cube = (agentId: string, size: number) => {
    const agent = agents.data?.find((item) => item.id === agentId)
    return (
      <Cube
        key={agentId}
        avatar={agent?.avatar ?? null}
        mood={agent ? moodOf(agent) : 'idle'}
        size={size}
      />
    )
  }
  return (
    <div
      ref={ref}
      className="agent-card task-card"
      data-pile={pile}
      role="tooltip"
    >
      <div className="task-card__head">
        <TaskMark task={task} />
        <div className="task-card__what">
          <strong>{titleOf(task)}</strong>
          <span>{headlineOf(task)}</span>
        </div>
        {total > 0 && (
          <span className="task-card__count tabular">
            {done}/{total}
          </span>
        )}
      </div>
      {total > 0 && (
        <div className="plan__meter task-card__meter" aria-hidden="true">
          <span style={{ width: `${(done / total) * 100}%` }} />
        </div>
      )}
      {total === 0 && pile === 'working' && (
        <div className="task-card__planning">
          <Cube avatar={ORCHESTRATOR_AVATAR} size={24} />
          <span aria-hidden="true" className="task-card__dots">
            <i />
            <i />
            <i />
          </span>
        </div>
      )}
      {total > 0 && (
        <ul className="task-card__items">
          {before > 0 && (
            <li className="task-card__more tabular">{before} finished above</li>
          )}
          {shown.map((item, index) => (
            <li
              key={item.id}
              className="task-card__item"
              data-status={item.status}
              style={{ '--i': index } as React.CSSProperties}
            >
              <ItemMark status={item.status} />
              <span className="task-card__label">{item.title}</span>
              {task.agents.length > 1 && cube(item.agent_id, 16)}
            </li>
          ))}
          {after > 0 && (
            <li className="task-card__more tabular">{after} more to go</li>
          )}
        </ul>
      )}
      {task.agents.length > 0 && (
        <div className="task-card__foot">
          <span className="task-card__crew" aria-hidden="true">
            {task.agents.slice(0, 4).map((member) => cube(member.id, 18))}
          </span>
          <span className="task-card__names">
            {task.agents.map((member) => member.name).join(', ')}
          </span>
          <time dateTime={task.created_at}>{when(task.created_at)}</time>
        </div>
      )}
    </div>
  )
}
