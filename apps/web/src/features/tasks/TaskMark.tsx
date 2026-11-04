import { CircleCheck, CircleDot, CircleStop, Loader } from 'lucide-react'
import { pileOf, type Task } from './status'

/** A task's state at a glance. The word beside it carries the meaning for assistive technology. */
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
