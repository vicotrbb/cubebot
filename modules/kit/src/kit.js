;(function () {
  var root = new URL('./', document.currentScript.src)
  var themeListeners = []
  var asking = []
  var current = null

  function applyTheme(theme) {
    if (theme !== 'light' && theme !== 'dark') return
    current = theme
    document.documentElement.dataset.theme = theme
    themeListeners.forEach(function (fn) {
      fn(theme)
    })
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
        if (body.error) throw new Error(body.error)
        return body.text
      })
    })
  }

  function ask(text) {
    if (window.parent === window) return Promise.resolve({ sent: false })
    return new Promise(function (resolve) {
      asking.push(resolve)
      window.parent.postMessage(
        { type: 'cubebot:ask', text: String(text) },
        '*',
      )
    })
  }

  function onTheme(fn) {
    themeListeners.push(fn)
    if (current) fn(current)
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
    if (data.type === 'cubebot:asked' && asking.length) {
      asking.shift()({ sent: true })
    }
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
