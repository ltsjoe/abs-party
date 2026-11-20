import { ref, getCurrentInstance, onUnmounted } from 'vue'

// One WebSocket to the party server that comes back by itself.
//
// Phones close it whenever the browser goes to the background (switching to
// Messages), so it reconnects after a short wait, at once when the page is
// shown again, and when pongs stop coming back (a connection the browser
// still thinks is open after the phone changed networks). The server holds
// the seat meanwhile and gives it back on the rejoin.
//
// disconnect() ends it for good: after Leave, when another page took the
// seat, on a fatal error, and when the page goes away. Before, a closed
// socket always reconnected, so every visit to a room left a socket behind
// that kept reconnecting until the tab closed.

export const RECONNECT_MS = 3000
export const PING_MS = 5000
// No pong for this long: treat the connection as dead and open a new one.
export const PONG_TIMEOUT_MS = 15000

export function useWebSocket(onMessage) {
  const connected = ref(false)
  const rtt = ref(0)
  let ws = null
  let pingInterval = null
  let reconnectTimer = null
  let stopped = true
  let pendingPings = {} // sent_at -> sent_at, for pings with no pong yet

  function socketUrl() {
    const protocol = location.protocol === 'https:' ? 'wss' : 'ws'
    return `${protocol}://${location.host}/ws`
  }

  function connect() {
    if (stopped) document.addEventListener('visibilitychange', onVisibility)
    stopped = false
    open()
  }

  function open() {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
    if (stopped) return
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return

    const sock = new WebSocket(socketUrl())
    ws = sock

    sock.onopen = () => {
      if (ws !== sock) return
      pendingPings = {}
      connected.value = true
      clearInterval(pingInterval)
      pingInterval = setInterval(sendPing, PING_MS)
    }

    sock.onclose = () => {
      if (ws !== sock) return
      lost()
      if (!stopped) reconnectTimer = setTimeout(open, RECONNECT_MS)
    }

    sock.onerror = () => sock.close()

    sock.onmessage = (evt) => {
      if (ws !== sock) return
      let msg
      try { msg = JSON.parse(evt.data) } catch { return }

      if (msg.type === 'pong') {
        const sent = pendingPings[msg.sent_at]
        if (sent) rtt.value = Date.now() - sent
        // An answer means the connection works: older pings count as answered too.
        for (const k of Object.keys(pendingPings)) {
          if (Number(k) <= msg.sent_at) delete pendingPings[k]
        }
        return
      }

      onMessage(msg)
    }
  }

  function lost() {
    connected.value = false
    clearInterval(pingInterval)
    pingInterval = null
  }

  // Let go of the current socket without waiting for its close event, and
  // ignore anything it still delivers.
  function abandon() {
    const sock = ws
    ws = null
    if (sock) {
      sock.onopen = sock.onclose = sock.onmessage = sock.onerror = null
      try { sock.close() } catch { /* already closed */ }
    }
    lost()
  }

  function sendPing() {
    const now = Date.now()
    const waiting = Object.values(pendingPings)
    if (waiting.length && now - Math.min(...waiting) > PONG_TIMEOUT_MS) {
      abandon()
      open()
      return
    }
    pendingPings[now] = now
    send({ type: 'ping', sent_at: now })
  }

  // Back on the page (e.g. from Messages): reconnect now rather than after
  // the timer, or check that the connection still answers.
  function onVisibility() {
    if (document.visibilityState !== 'visible' || stopped) return
    if (!ws || ws.readyState === WebSocket.CLOSING || ws.readyState === WebSocket.CLOSED) open()
    else if (ws.readyState === WebSocket.OPEN) sendPing()
  }

  function send(obj) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(obj))
      return true
    }
    return false
  }

  function disconnect() {
    stopped = true
    clearTimeout(reconnectTimer)
    reconnectTimer = null
    document.removeEventListener('visibilitychange', onVisibility)
    abandon()
  }

  if (getCurrentInstance()) onUnmounted(disconnect)

  return { connect, send, disconnect, connected, rtt }
}
