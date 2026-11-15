import assert from 'node:assert/strict'
import test from 'node:test'
import { browserLogins, csvRows } from './browserLogins'

test('quoted cells keep commas, quotes and line breaks', () => {
  assert.deepEqual(csvRows('a,"b, ""c""",d\r\n"x\ny",,z\n'), [
    ['a', 'b, "c"', 'd'],
    ['x\ny', '', 'z'],
  ])
})

test('a Chrome export becomes logins named by the browser, keyed by origin', () => {
  const file =
    '﻿name,url,username,password,note\n' +
    'GitHub,https://github.com/login?return=/x,octocat,pa,ss,\n' +
    ',https://www.example.com/a,me,secret,\n'
  const found = browserLogins(file.replace('pa,ss', '"pa,ss"'))
  assert.deepEqual(found?.logins, [
    {
      name: 'GitHub',
      site: 'https://github.com',
      username: 'octocat',
      secret: 'pa,ss',
    },
    {
      name: 'example.com',
      site: 'https://www.example.com',
      username: 'me',
      secret: 'secret',
    },
  ])
})

test('a Firefox export skips what agents cannot fill and says why', () => {
  const file = [
    '"url","username","password","httpRealm","formActionOrigin","guid"',
    '"chrome://FirefoxAccounts","me","x","","",""',
    '"https://a.test","","x","","",""',
    '"https://b.test","me","","","",""',
    '"https://c.test","me","pässword","","",""',
    `"https://d.test","me","${'x'.repeat(5000)}","","",""`,
    '"https://e.test","me","fine","","",""',
  ].join('\n')
  const found = browserLogins(file)
  assert.deepEqual(
    found?.logins.map((login) => login.name),
    ['e.test'],
  )
  assert.deepEqual(found?.skipped, {
    not_web: 1,
    no_username: 1,
    no_password: 1,
    untypable: 1,
    too_long: 1,
  })
})

test('a file without login columns is not a password export', () => {
  assert.equal(browserLogins('title,body\na,b'), null)
})
