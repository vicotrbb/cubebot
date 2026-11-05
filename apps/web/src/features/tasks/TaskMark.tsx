import {
  Circle,
  CircleCheck,
  CircleDot,
  CircleMinus,
  CircleStop,
  Loader,
} from 'lucide-react'
import { pileOf, type Task } from './status'

export function ItemMark({ status }: { status: string }) {
  if (status === 'done') return <CircleCheck aria-label="Done" />
  if (status === 'doing')
    return <Loader className="spin" aria-label="In progress" />
  if (status === 'skipped') return <CircleMinus aria-label="Skipped" />
  return <Circle aria-label="Not started" />
}

/** The word beside it carries the meaning for assistive technology. */
export function TaskMark({ task }: { task: Task }) {
  const pile = pileOf(task)
  if (pile === 'needs_you')
    return (
      <CircleDot
        className="task-mark"
        data-state="needs_you"
        aria-hidden="true"
      />
    )
  if (pile === 'working')
    return (
      <Loader
        className="task-mark spin"
        data-state="working"
        aria-hidden="true"
      />
    )
  if (task.status === 'done')
    return (
      <CircleCheck className="task-mark" data-state="done" aria-hidden="true" />
    )
  return (
    <CircleStop className="task-mark" data-state="stopped" aria-hidden="true" />
  )
}
