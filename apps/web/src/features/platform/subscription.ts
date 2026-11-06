import type { components } from '../../api/schema'

type Subscription = components['schemas']['SubscriptionSummary']
type Plan = 'chatgpt' | 'supergrok' | 'minimax_plan' | 'minimax_plan_cn'

const NAME: Record<Plan, string> = {
  chatgpt: 'ChatGPT',
  supergrok: 'Grok',
  minimax_plan: 'MiniMax',
  minimax_plan_cn: 'MiniMax',
}

/** What a signed-in plan needs a person to know, most urgent first. */
export function planProblem(plan: Plan, subscription: Subscription) {
  const name = NAME[plan]
  if (subscription.problem === 'sign_in_again')
    return {
      severe: true,
      text: `${name} refused to renew this sign-in, so agents cannot use it. Sign out, then sign in again.`,
    }
  if (subscription.waiting > 0)
    return {
      severe: true,
      text: `${subscription.waiting} ${subscription.waiting === 1 ? 'turn waits' : 'turns wait'}: the plan reached its usage limit. Retry ${subscription.waiting === 1 ? 'it' : 'them'} from the conversation once your ${name} usage resets.`,
    }
  if (subscription.problem === 'refresh_failing')
    return {
      severe: false,
      text: `Renewing the sign-in keeps failing; Cubebot retries every half minute. Agents keep working until ${new Date(subscription.expires_at * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}.`,
    }
  return null
}

const REFUSED: Record<string, string> = {
  sign_in_expired: 'This sign-in expired. Start again.',
  sign_in_denied: 'Access was declined. Start again to retry.',
  sign_in_refused: 'The sign-in was refused. Start again.',
  sign_in_mismatch:
    'That sign-in belongs to a different Cubebot registration. Start again.',
  plan_not_included:
    'The sign-in did not allow plan usage. Start again and allow Cubebot to use your plan, or use an API key.',
}

/** Why a sign-in stopped, from the orchestrator's status and error code. */
export function signInProblem(
  plan: Plan,
  status: number | undefined,
  code: string | undefined,
) {
  const name = NAME[plan]
  if (status === 422 && code && REFUSED[code]) return REFUSED[code]
  if (status === 400 && plan.startsWith('minimax'))
    return 'That key has characters a key cannot hold.'
  if (status === 400)
    return 'That is not this sign-in’s address. Copy the whole address of the page that failed to load; it starts with http://127.0.0.1:1456/auth/callback.'
  if (status === 409)
    return `Someone else’s ${name} plan is signed in. They or an administrator must sign it out first.`
  if (status === 502 || status === undefined)
    return `${name} could not be reached. Retry shortly.`
  if (status === 403) return 'Only administrators can sign in a plan.'
  return 'The sign-in failed. Start again shortly.'
}
