import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { useWebSocket, RECONNECT_MS, PING_MS, PONG_TIMEOUT_MS } from './useWebSocket.js'

// A stand-in for the browser's WebSocket that the test opens, drops and feeds.
class FakeWebSocket {
  static CONNECTING = 0
  static OPEN = 1
  static CLOSING = 2
  static CLOSED = 3
  static all = []

  constructor(url) {
    this.url = url
    this.readyState = FakeWebSocket.CONNECTING
    this.sent = []
    FakeWebSocket.all.push(this)
  }
  send(data) { this.sent.push(JSON.parse(data)) }
  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return
    this.readyState = FakeWebSocket.CLOSED
    this.onclose?.()
  }
  // test side
  accept() { this.readyState = FakeWebSocket.OPEN; this.onopen?.() }
  drop() { this.readyState = FakeWebSocket.CLOSED; this.onclose?.() }
  receive(msg) { this.onmessage?.({ data: JSON.stringify(msg) }) }
}

let visibility = 'visible'
function setVisibility(v) {
  visibility = v
  document.dispatchEvent(new Event('visibilitychange'))
}

const sockets = () => FakeWebSocket.all
const last = () => FakeWebSocket.all[FakeWebSocket.all.length - 1]

beforeEach(() => {
  FakeWebSocket.all = []
  visibility = 'visible'
  vi.useFakeTimers()
  vi.stubGlobal('WebSocket', FakeWebSocket)
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility })
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('useWebSocket', () => {
  it('reconnects after the connection drops', () => {
    const { connect, connected } = useWebSocket(() => {})
    connect()
    last().accept()
    expect(connected.value).toBe(true)

    last().drop()
    expect(connected.value).toBe(false)
    vi.advanceTimersByTime(RECONNECT_MS - 1)
    expect(sockets()).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(sockets()).toHaveLength(2)
  })

  it('stops for good after disconnect, so no socket is left reconnecting', () => {
    const { connect, disconnect } = useWebSocket(() => {})
    connect()
    last().accept()

    disconnect()
    vi.advanceTimersByTime(10 * RECONNECT_MS)
    setVisibility('hidden')
    setVisibility('visible')

    expect(sockets()).toHaveLength(1)
    expect(sockets()[0].readyState).toBe(FakeWebSocket.CLOSED)
  })

  it('reconnects at once when the page is shown again', () => {
    const { connect } = useWebSocket(() => {})
    connect()
    last().accept()
    setVisibility('hidden')
    last().drop() // the phone closed it while the browser was in the background

    setVisibility('visible')

    expect(sockets()).toHaveLength(2)
    vi.advanceTimersByTime(RECONNECT_MS) // the pending timer doesn't open a third
    expect(sockets()).toHaveLength(2)
  })

  it('replaces a connection whose pongs stopped coming', () => {
    const onMessage = vi.fn()
    const { connect, connected } = useWebSocket(onMessage)
    connect()
    const dead = last()
    dead.accept()

    vi.advanceTimersByTime(PING_MS + PONG_TIMEOUT_MS + PING_MS)

    expect(sockets()).toHaveLength(2)
    expect(connected.value).toBe(false)
    // Anything the old socket still delivers is ignored.
    dead.receive({ type: 'replaced', message: 'x' })
    expect(onMessage).not.toHaveBeenCalled()
  })

  it('keeps a connection that answers its pings', () => {
    const { connect } = useWebSocket(() => {})
    connect()
    const sock = last()
    sock.accept()

    for (let i = 0; i < 10; i++) {
      vi.advanceTimersByTime(PING_MS)
      const ping = sock.sent[sock.sent.length - 1]
      expect(ping.type).toBe('ping')
      sock.receive({ type: 'pong', sent_at: ping.sent_at })
    }

    expect(sockets()).toHaveLength(1)
  })

  it('hands every other message to the page', () => {
    const onMessage = vi.fn()
    const { connect } = useWebSocket(onMessage)
    connect()
    last().accept()

    last().receive({ type: 'participant_away', name: 'Bob' })

    expect(onMessage).toHaveBeenCalledWith({ type: 'participant_away', name: 'Bob' })
  })
})
