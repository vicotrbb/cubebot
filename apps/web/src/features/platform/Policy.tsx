import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { admin, type Policy } from '../../api/admin'
import {
  Alert,
  Field,
  Skeleton,
  Status,
  Switch,
} from '../../components/primitives'
import { useAdminAction } from './useAdminAction'

type Flag = Exclude<keyof Policy, 'member_agent_limit'>

const FLAGS: [Flag, string, string][] = [
  [
    'members_create_agents',
    'Create agents',
    'Whoever creates an agent owns it and decides who it is shared with.',
  ],
  [
    'members_create_rooms',
    'Create rooms',
    'Rooms where people and agents work together.',
  ],
  [
    'members_create_teams',
    'Create teams',
    'Teams of agents with a lead and an org chart.',
  ],
  [
    'members_create_connections',
    'Add their own connections',
    'Personal connections to apps such as GitHub, signed in with their own account.',
  ],
  [
    'members_bring_providers',
    'Bring their own AI provider',
    'Their own keys and plans, used only by agents they own.',
  ],
]

export function MemberPolicy({ writer }: { writer: boolean }) {
  const policy = useQuery({ queryKey: ['policy'], queryFn: admin.policy })
  return (
    <section className="policy" aria-labelledby="policy-title">
      <h2 id="policy-title" className="platform__h2">
        What members can do
      </h2>
      <p className="platform__lead">
        Administrators are never limited. Auditors and guests never create
        anything.
      </p>
      {policy.isPending && <Skeleton lines={4} width="62ch" />}
      {policy.isError && <Alert>The rules could not be read.</Alert>}
      {policy.data && <PolicyForm policy={policy.data} writer={writer} />}
    </section>
  )
}

function PolicyForm({ policy, writer }: { policy: Policy; writer: boolean }) {
  const [draft, setDraft] = useState(policy)
  const action = useAdminAction(['policy', 'standing'])
  const set = (patch: Partial<Policy>) => setDraft({ ...draft, ...patch })
  const limit = draft.member_agent_limit ?? null
  return (
    <form
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        void action.run(() => admin.savePolicy(draft), 'Saved. It applies now.')
      }}
    >
      {FLAGS.map(([flag, name, about]) => (
        <div key={flag} className="policy__row">
          <div>
            <p className="policy__name">{name}</p>
            <p className="faint small">{about}</p>
          </div>
          <Switch
            checked={draft[flag]}
            label={name}
            disabled={!writer}
            onChange={(on) => set({ [flag]: on })}
          />
        </div>
      ))}
      {draft.members_create_agents && (
        <Field
          label="Agents each member may own"
          hint="Empty for no limit; retired agents do not count"
        >
          <input
            type="number"
            inputMode="numeric"
            min={0}
            max={10000}
            placeholder="No limit"
            disabled={!writer}
            value={limit ?? ''}
            onChange={(event) =>
              set({
                member_agent_limit:
                  event.target.value === ''
                    ? null
                    : Math.max(0, Math.floor(event.target.valueAsNumber)),
              })
            }
          />
        </Field>
      )}
      {writer && (
        <>
          <Alert>{action.error}</Alert>
          <Status>{action.notice}</Status>
          <div className="actions">
            <button className="btn btn--primary btn--sm" disabled={action.busy}>
              {action.busy ? 'Saving…' : 'Save rules'}
            </button>
          </div>
        </>
      )}
    </form>
  )
}
