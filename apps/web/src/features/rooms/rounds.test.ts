// @ts-expect-error bun provides this at test time
import { expect, test } from 'bun:test'
import type { RoomMessage } from '../../api/client'
import { rounds } from './rounds'

const message = (
  id: string,
  deliveries: string[] = [],
  agent: string | null = null,
) =>
  ({
    id,
    author_agent_id: agent,
    deliveries: deliveries.map((turn) => ({ turn })),
  }) as unknown as RoomMessage

const shape = (messages: RoomMessage[]) =>
  rounds(messages).map(({ message, deliveries }) => [
    message.id,
    deliveries.map((item) => item.turn),
  ])

test('a reply comes after what its agent posted while working', () => {
  expect(
    shape([
      message('ask'),
      message('handover', ['ayla']),
      message('progress', [], 'ayla'),
      message('almost', [], 'ayla'),
    ]),
  ).toEqual([
    ['ask', []],
    ['handover', []],
    ['progress', []],
    ['almost', ['ayla']],
  ])
})

test('a person speaking closes the round before it', () => {
  expect(
    shape([
      message('handover', ['ayla']),
      message('progress', [], 'ayla'),
      message('person'),
      message('next', ['bee']),
    ]),
  ).toEqual([
    ['handover', []],
    ['progress', ['ayla']],
    ['person', []],
    ['next', ['bee']],
  ])
})

test('a turn an agent started for another stays in the round', () => {
  expect(
    shape([message('handover', ['ayla']), message('ask bee', ['bee'], 'ayla')]),
  ).toEqual([
    ['handover', []],
    ['ask bee', ['ayla', 'bee']],
  ])
})
