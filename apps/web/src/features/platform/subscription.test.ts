import test from 'node:test'
import assert from 'node:assert/strict'
import { planProblem, signInProblem } from './subscription'

const plan = (over: object) => ({
  provider: 'chatgpt' as const,
  account: 'person@example.com',
  owner: 'admin',
  mine: true,
  expires_at: 1_800_000_000,
  problem: null,
  waiting: 0,
  ...over,
})

test('a refused renewal outranks waiting turns', () => {
  const problem = planProblem(
    'chatgpt',
    plan({ problem: 'sign_in_again', waiting: 2 }),
  )
  assert.equal(problem?.severe, true)
  assert.match(problem?.text ?? '', /sign in again/)
})

test('usage-limit waits are counted and point to retrying', () => {
  assert.match(
    planProblem('supergrok', plan({ waiting: 1 }))?.text ?? '',
    /^1 turn waits/,
  )
  assert.match(
    planProblem('chatgpt', plan({ waiting: 3 }))?.text ?? '',
    /Retry them/,
  )
})

test('a working plan has nothing to say, a failing renewal is not severe', () => {
  assert.equal(planProblem('chatgpt', plan({})), null)
  assert.equal(
    planProblem('chatgpt', plan({ problem: 'refresh_failing' }))?.severe,
    false,
  )
})

test('sign-in problems name the recovery', () => {
  assert.match(
    signInProblem('chatgpt', 422, 'plan_not_included'),
    /allow Cubebot to use your plan/,
  )
  assert.match(signInProblem('chatgpt', 400, undefined), /127\.0\.0\.1:1456/)
  assert.match(signInProblem('supergrok', 409, undefined), /Grok plan/)
  assert.match(
    signInProblem('supergrok', undefined, undefined),
    /could not be reached/,
  )
})
