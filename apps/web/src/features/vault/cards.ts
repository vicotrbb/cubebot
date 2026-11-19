export type CardInput = {
  holder: string
  number: string
  expiry: string
  cvc: string
}

export const digitsOf = (text: string) => text.replace(/\D/g, '')

export function luhn(number: string) {
  let sum = 0
  for (let at = 0; at < number.length; at += 1) {
    const digit = Number(number[number.length - 1 - at])
    const doubled = at % 2 === 1 ? digit * 2 : digit
    sum += doubled > 9 ? doubled - 9 : doubled
  }
  return number.length > 0 && sum % 10 === 0
}

export function brandOf(number: string) {
  const head = (length: number) => Number(number.slice(0, length))
  if (number.startsWith('4')) return 'Visa'
  if ([34, 37].includes(head(2))) return 'American Express'
  if ((head(2) >= 51 && head(2) <= 55) || (head(4) >= 2221 && head(4) <= 2720))
    return 'Mastercard'
  if (head(4) === 6011 || head(2) === 65) return 'Discover'
  return null
}

export function groupedNumber(text: string) {
  const digits = digitsOf(text).slice(0, 19)
  const amex = /^3[47]/.test(digits)
  const groups = amex ? [4, 6, 5] : [4, 4, 4, 4, 3]
  const parts: string[] = []
  let at = 0
  for (const size of groups) {
    if (at >= digits.length) break
    parts.push(digits.slice(at, at + size))
    at += size
  }
  return parts.join(' ')
}

export function typedExpiry(text: string) {
  const digits = digitsOf(text).slice(0, 4)
  return digits.length > 2 ? `${digits.slice(0, 2)}/${digits.slice(2)}` : digits
}

export function cardProblem(card: CardInput) {
  const number = digitsOf(card.number)
  if (!card.holder.trim()) return 'Enter the cardholder name as printed.'
  if (number.length < 12 || number.length > 19 || !luhn(number))
    return 'That card number does not check out; look for a typo.'
  const [month, year] = card.expiry.split('/')
  if (!/^(0[1-9]|1[0-2])$/.test(month ?? '') || !/^\d{2}$/.test(year ?? ''))
    return 'Enter the expiry as MM/YY.'
  if (!/^\d{3,4}$/.test(card.cvc.trim()))
    return 'The security code is the 3 or 4 digits on the card.'
  return null
}

export function expired(expires: string, now = new Date()) {
  const [month, year] = expires.split('/').map(Number)
  if (!month || year === undefined) return false
  return new Date(2000 + year, month, 1) <= now
}
