;(function () {
  var root = new URL('./', document.currentScript.src)
  var themeListeners = []
  var pending = new Map()
  var counter = 0
  var current = null

  function notify(fn) {
    try {
      fn(current)
    } catch {
      return
    }
  }

  function applyTheme(theme) {
    if (theme !== 'light' && theme !== 'dark') return
    current = theme
    document.documentElement.dataset.theme = theme
    themeListeners.forEach(notify)
  }

  function readError(response) {
    return response
      .json()
      .catch(function () {
        return {}
      })
      .then(function (body) {
        throw new Error(
          body.error || 'The call failed (' + response.status + ')',
        )
      })
  }

  function call(tool, input) {
    return fetch(new URL('call', root), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ tool: tool, input: input || {} }),
    }).then(function (response) {
      if (!response.ok) return readError(response)
      return response.json().then(function (body) {
        if (body.ok === false || body.error) {
          throw new Error(body.error || 'The call failed')
        }
        return body.text
      })
    })
  }

  function settle(id, ok, error) {
    var entry = pending.get(id)
    if (!entry) return
    pending.delete(id)
    clearTimeout(entry.timer)
    if (ok) entry.resolve({ sent: true })
    else entry.reject(new Error(error || 'The message was not sent'))
  }

  function ask(text) {
    if (window.parent === window) return Promise.resolve({ sent: false })
    var id = ++counter
    return new Promise(function (resolve, reject) {
      var timer = setTimeout(function () {
        settle(id, false, 'The message timed out without an answer')
      }, 30000)
      pending.set(id, { resolve: resolve, reject: reject, timer: timer })
      window.parent.postMessage(
        { type: 'cubebot:ask', id: id, text: String(text) },
        '*',
      )
    })
  }

  function onTheme(fn) {
    themeListeners.push(fn)
    if (current) notify(fn)
    return function () {
      themeListeners = themeListeners.filter(function (item) {
        return item !== fn
      })
    }
  }

  function receive(event) {
    var data = event.data
    if (event.source !== window.parent || !data) return
    if (data.type === 'cubebot:theme') applyTheme(data.theme)
    if (data.type === 'cubebot:asked') settle(data.id, data.ok, data.error)
  }

  applyTheme(new URLSearchParams(location.search).get('theme'))
  window.addEventListener('message', receive)
  window.cubebot = {
    call: call,
    ask: ask,
    theme: function () {
      return current
    },
    onTheme: onTheme,
  }
  if (window.parent !== window) {
    window.parent.postMessage({ type: 'cubebot:ready' }, '*')
  }
})()
