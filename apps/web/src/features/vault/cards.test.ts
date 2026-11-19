// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import {
  brandOf,
  cardProblem,
  expired,
  groupedNumber,
  luhn,
  typedExpiry,
} from './cards'

describe('cards', () => {
  test('checks the number and names the brand', () => {
    expect(luhn('4242424242424242')).toBe(true)
    expect(luhn('4242424242424241')).toBe(false)
    expect(brandOf('4242')).toBe('Visa')
    expect(brandOf('2223003122003222')).toBe('Mastercard')
    expect(brandOf('378282246310005')).toBe('American Express')
    expect(brandOf('9999')).toBe(null)
  })

  test('formats as the person types', () => {
    expect(groupedNumber('4242424242424242')).toBe('4242 4242 4242 4242')
    expect(groupedNumber('378282246310005')).toBe('3782 822463 10005')
    expect(typedExpiry('0729')).toBe('07/29')
    expect(typedExpiry('07/2')).toBe('07/2')
  })

  test('explains what is wrong', () => {
    const card = {
      holder: 'Ada',
      number: '4242 4242 4242 4242',
      expiry: '07/29',
      cvc: '123',
    }
    expect(cardProblem(card)).toBe(null)
    expect(cardProblem({ ...card, number: '4242 4242 4242 4241' })).toContain(
      'typo',
    )
    expect(cardProblem({ ...card, expiry: '13/29' })).toContain('MM/YY')
    expect(cardProblem({ ...card, cvc: '12' })).toContain('3 or 4')
  })

  test('a card expires after its last month', () => {
    const november = new Date(2026, 10, 18)
    expect(expired('11/26', november)).toBe(false)
    expect(expired('10/26', november)).toBe(true)
  })
})
