// Room.vue against a fake party server: the page's side of held seats,
// takeovers and waiting for the host. Audiobookshelf, audio and the router
// are stubbed; the WebSocket is a fake the test opens, drops and feeds.
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { ref, reactive } from 'vue'

const h = vi.hoisted(() => ({
  player: null,
  route: { params: { id: 'abcd1234' }, query: {}, fullPath: '/room/abcd1234' },
  push: null,
}))

vi.mock('vue-router', () => ({
  useRoute: () => h.route,
  useRouter: () => ({ push: h.push, replace: () => {} }),
}))
vi.mock('../composables/useAbsPlayer.js', () => ({ useAbsPlayer: () => h.player }))
vi.mock('../composables/useAudioCues.js', () => ({
  useAudioCues: () => ({ playJoinLeave: () => {}, playAlert: () => {} }),
}))
vi.mock('../api/abs.js', () => ({ getBookmarks: async () => [], createBookmark: async () => {} }))

import Room from './Room.vue'

class FakeWebSocket {
  static CONNECTING = 0
  static OPEN = 1
  static CLOSING = 2
  static CLOSED = 3
  static all = []
  constructor() {
    this.readyState = FakeWebSocket.CONNECTING
    this.sent = []
    FakeWebSocket.all.push(this)
  }
  send(data) {
    const msg = JSON.parse(data)
    this.sent.push(msg)
    // Answer pings like the real server, or the page's dead-connection check
    // (no pong for 15 s) would open a new socket mid-test.
    if (msg.type === 'ping') this.receive({ type: 'pong', sent_at: msg.sent_at })
  }
  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return
    this.readyState = FakeWebSocket.CLOSED
    this.onclose?.()
  }
  accept() { this.readyState = FakeWebSocket.OPEN; this.onopen?.() }
  drop() { this.readyState = FakeWebSocket.CLOSED; this.onclose?.() }
  receive(msg) { this.onmessage?.({ data: JSON.stringify(msg) }) }
  types() { return this.sent.map((m) => m.type).filter((t) => t !== 'ping') }
}
const last = () => FakeWebSocket.all[FakeWebSocket.all.length - 1]

function makePlayer() {
  return {
    open: vi.fn(async () => ({ currentTime: 42 })),
    play: vi.fn(),
    pause: vi.fn(),
    seekTo: vi.fn(),
    setSpeed: vi.fn(),
    setVolume: vi.fn(),
    close: vi.fn(async () => {}),
    currentTime: ref(0),
    duration: ref(600),
    volume: ref(1),
    isPlaying: ref(false),
    preJoinBookmarkSaved: ref(false),
    chapters: ref([]),
    audio: { addEventListener: vi.fn(), removeEventListener: vi.fn() },
  }
}

const item = { id: 'item1', library_id: 'lib1', episode_id: null, title: 'Book', author: 'Author' }
function state({ participants, position = 0, playing = false }) {
  return { type: 'room_state', room_id: 'abcd1234', item, position, playing, speed: 1, participants }
}
const joe = (extra = {}) => ({ name: 'joe', is_host: true, away: false, ...extra })
const rebecca = (extra = {}) => ({ name: 'rebecca', is_host: false, away: false, ...extra })

async function settle() {
  await flushPromises()
  await flushPromises()
}

async function mountRoom({ username = 'rebecca', query = {} } = {}) {
  h.route.query = query
  const wrapper = mount(Room, {
    global: {
      provide: {
        auth: reactive({ token: 'tok', username }),
        ABS_BASE: 'https://abs.test',
        signInExpired: () => {},
      },
    },
  })
  await settle()
  last().accept()
  vi.advanceTimersByTime(100) // the page polls for the connection every 100 ms
  await settle()
  return wrapper
}

beforeEach(() => {
  FakeWebSocket.all = []
  h.player = makePlayer()
  h.push = vi.fn()
  vi.useFakeTimers()
  vi.stubGlobal('WebSocket', FakeWebSocket)
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' })
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('Room page', () => {
  it('waits for the host instead of failing, then gets in', async () => {
    const wrapper = await mountRoom()
    expect(last().types()).toEqual(['join'])

    last().receive({ type: 'room_not_found', message: 'Room not found' })
    await settle()
    expect(wrapper.text()).toContain('Waiting for the host to open the room…')

    vi.advanceTimersByTime(5000)
    expect(last().types()).toEqual(['join', 'join'])

    last().receive(state({ participants: [joe(), rebecca()] }))
    await settle()
    expect(h.player.open).toHaveBeenCalled()
    expect(wrapper.text()).toContain('joe')
    vi.advanceTimersByTime(20000)
    expect(last().types()).toEqual(['join', 'join']) // no more asking once in
  })

  it('shows who is reconnecting, then that they are back', async () => {
    const wrapper = await mountRoom()
    last().receive(state({ participants: [joe(), rebecca()] }))
    await settle()

    last().receive({ type: 'participant_away', name: 'joe' })
    await settle()
    expect(wrapper.find('.away-badge').exists()).toBe(true)
    expect(wrapper.text()).toContain('joe is reconnecting…')

    last().receive({ type: 'participant_back', name: 'joe' })
    await settle()
    expect(wrapper.find('.away-badge').exists()).toBe(false)
    expect(wrapper.text()).toContain('joe is back')
  })

  it('a page another page took over goes quiet and stays out, until asked back', async () => {
    const wrapper = await mountRoom()
    const first = last()
    first.receive(state({ participants: [joe(), rebecca()] }))
    await settle()

    first.receive({ type: 'replaced', message: 'x' })
    await settle()
    expect(wrapper.text()).toContain('open on another page or device')
    expect(h.player.pause).toHaveBeenCalled()
    expect(first.readyState).toBe(FakeWebSocket.CLOSED)
    vi.advanceTimersByTime(60000)
    expect(FakeWebSocket.all).toHaveLength(1) // no reconnecting back in on its own

    const useHere = wrapper.findAll('button').find((b) => b.text() === 'Use this page')
    await useHere.trigger('click')
    last().accept()
    await settle()
    expect(FakeWebSocket.all).toHaveLength(2)
    expect(last().types()).toEqual(['join'])
  })

  it('an error screen does not rejoin in the background', async () => {
    const wrapper = await mountRoom()
    last().receive({ type: 'error', message: 'Audiobookshelf could not be reached.' })
    await settle()

    expect(wrapper.text()).toContain('Audiobookshelf could not be reached.')
    vi.advanceTimersByTime(60000)
    expect(FakeWebSocket.all).toHaveLength(1)
    expect(last().readyState).toBe(FakeWebSocket.CLOSED)
  })

  it('back after a drop with others there: follows the room', async () => {
    await mountRoom()
    last().receive(state({ participants: [joe(), rebecca()] }))
    await settle()

    last().drop()
    await settle()
    vi.advanceTimersByTime(3000)
    last().accept()
    await settle()
    expect(last().types()).toEqual(['join'])

    h.player.seekTo.mockClear()
    last().receive(state({ participants: [joe(), rebecca()], position: 300, playing: true }))
    await settle()
    expect(h.player.seekTo).toHaveBeenCalledWith(300)
    expect(h.player.play).toHaveBeenCalled()
  })

  it('the host back after a drop with nobody else there: the room takes its place', async () => {
    const query = { host: 'true', itemId: 'item1', title: 'Book', author: 'Author', libraryId: 'lib1' }
    await mountRoom({ username: 'joe', query })
    expect(last().types()).toEqual(['create_room'])
    last().receive(state({ participants: [joe()] }))
    await settle()
    // Opening the room: starts from the host's own Audiobookshelf progress.
    expect(last().sent.find((m) => m.type === 'seek').position).toBe(42)

    // Joe switches apps; his book keeps playing in the background.
    h.player.isPlaying.value = true
    h.player.currentTime.value = 120
    last().drop()
    await settle()
    vi.advanceTimersByTime(3000)
    last().accept()
    await settle()
    expect(last().types()).toEqual(['create_room'])

    last().receive(state({ participants: [joe()], position: 0, playing: false }))
    await settle()
    expect(last().sent.at(-1)).toEqual({ type: 'play_intent', position: 120 })
  })

  it('the host reopening a room others are in follows it instead of moving it', async () => {
    const query = { host: 'true', itemId: 'item1', title: 'Book', author: 'Author', libraryId: 'lib1' }
    await mountRoom({ username: 'joe', query })
    last().receive(state({ participants: [joe(), rebecca()], position: 500, playing: true }))
    await settle()

    expect(last().types()).toEqual(['create_room'])
    expect(h.player.seekTo).toHaveBeenCalledWith(500)
    expect(h.player.play).toHaveBeenCalled()
  })

  it('Leave gives the seat up and stops the connection', async () => {
    const wrapper = await mountRoom()
    last().receive(state({ participants: [joe(), rebecca()] }))
    await settle()

    await wrapper.find('.btn-leave').trigger('click')
    await settle()

    expect(last().types()).toContain('leave')
    expect(last().readyState).toBe(FakeWebSocket.CLOSED)
    expect(h.push).toHaveBeenCalledWith('/')
    vi.advanceTimersByTime(60000)
    expect(FakeWebSocket.all).toHaveLength(1)
  })
})
