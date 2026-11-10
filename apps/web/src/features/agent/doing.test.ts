import test from 'node:test'
import assert from 'node:assert/strict'
import { doingOf } from './doing'

const working = (current_tool: string | null, current_target?: string) => ({
  activity: 'working',
  current_tool,
  current_target: current_target ?? null,
})

test('a working agent is described by its current tool and target', () => {
  assert.equal(doingOf(working(null)), 'Thinking')
  assert.equal(doingOf(working('bash')), 'Running a command')
  assert.equal(
    doingOf(working('write_file', 'docs/poem.txt')),
    'Writing poem.txt',
  )
  assert.equal(doingOf(working('write_file')), 'Writing a file')
  assert.equal(
    doingOf(working('browser_navigate', 'https://www.example.com/a')),
    'Browsing example.com',
  )
  assert.equal(doingOf(working('browser_click')), 'Using the browser')
  assert.equal(
    doingOf(working('kb_search', 'deploys')),
    'Reading the knowledge base',
  )
  assert.equal(doingOf(working('kb_edit')), 'Writing in the knowledge base')
  assert.equal(doingOf(working('github__create_issue')), 'Using github')
  assert.equal(doingOf(working('something_new')), 'Using something new')
})

test('an agent not working says so, and a waiting login comes first', () => {
  assert.equal(doingOf({ activity: 'sleeping' }), 'Asleep, wakes when asked')
  assert.equal(doingOf({ activity: 'idle' }), 'Ready for work')
  assert.equal(
    doingOf({ activity: 'working', pending_login: true }),
    'Waiting for you to log in',
  )
  assert.equal(
    doingOf({ activity: 'idle', pending_creations: 2 }),
    'Waiting for your approval',
  )
})
