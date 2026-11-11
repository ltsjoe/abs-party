import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import {
  login,
  getMe,
  getProgress,
  createBookmark,
  startPlaybackSession,
  syncProgress,
  closeSession,
  getLibraryItems,
  searchLibraryItems,
  getRecentEpisodes,
  renewSso,
  tokenStatus,
} from './abs.js'

const ABS = 'https://abs.example'
const TOKEN = 'test-token-123'

function mockFetch(status, body) {
  return vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
  })
}

beforeEach(() => {
  // Stub localStorage for getDeviceId()
  vi.stubGlobal('localStorage', {
    getItem: vi.fn(() => 'device-uuid'),
    setItem: vi.fn(),
  })
  vi.stubGlobal('crypto', { randomUUID: vi.fn(() => 'device-uuid') })
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('login', () => {
  it('posts to /api/abs/login with credentials', async () => {
    const fetchMock = mockFetch(200, { user: { token: 'tok', username: 'alice', id: '1' } })
    vi.stubGlobal('fetch', fetchMock)

    const result = await login('alice', 'password')

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/abs/login',
      expect.objectContaining({ method: 'POST' })
    )
    const body = JSON.parse(fetchMock.mock.calls[0][1].body)
    expect(body.username).toBe('alice')
    expect(body.password).toBe('password')
    expect(result.user.token).toBe('tok')
  })

  it('throws on non-ok response', async () => {
    vi.stubGlobal('fetch', mockFetch(401, {}))
    await expect(login('bad', 'creds')).rejects.toThrow('Login failed')
  })
})

describe('getMe', () => {
  it('calls proxy /api/me with Bearer token', async () => {
    const fetchMock = mockFetch(200, { id: '1', username: 'alice' })
    vi.stubGlobal('fetch', fetchMock)

    await getMe(ABS, TOKEN)

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/abs-proxy/api/me',
      expect.objectContaining({ headers: { Authorization: `Bearer ${TOKEN}` } })
    )
  })
})

describe('getProgress', () => {
  it('returns null on 404', async () => {
    vi.stubGlobal('fetch', mockFetch(404, {}))
    const result = await getProgress(ABS, TOKEN, 'item-1')
    expect(result).toBeNull()
  })

  it('returns progress object on 200', async () => {
    const progress = { currentTime: 42.0, progress: 0.1 }
    vi.stubGlobal('fetch', mockFetch(200, progress))
    const result = await getProgress(ABS, TOKEN, 'item-1')
    expect(result.currentTime).toBe(42.0)
  })

  it('throws on other error status', async () => {
    vi.stubGlobal('fetch', mockFetch(500, {}))
    await expect(getProgress(ABS, TOKEN, 'item-1')).rejects.toThrow('Failed to fetch progress')
  })
})

describe('createBookmark', () => {
  it('posts to correct ABS endpoint with auth header', async () => {
    const fetchMock = mockFetch(200, { time: 60, title: 'pre_party_2026-06-21' })
    vi.stubGlobal('fetch', fetchMock)

    await createBookmark(ABS, TOKEN, 'item-1', 60, 'pre_party_2026-06-21')

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/abs-proxy/api/me/item/item-1/bookmark',
      expect.objectContaining({
        method: 'POST',
        headers: expect.objectContaining({ Authorization: `Bearer ${TOKEN}` }),
      })
    )
    const body = JSON.parse(fetchMock.mock.calls[0][1].body)
    expect(body.time).toBe(60)
    expect(body.title).toBe('pre_party_2026-06-21')
  })

  it('throws on failure', async () => {
    vi.stubGlobal('fetch', mockFetch(400, {}))
    await expect(createBookmark(ABS, TOKEN, 'item-1', 0, 'test')).rejects.toThrow(
      'Failed to create bookmark'
    )
  })
})

describe('startPlaybackSession', () => {
  it('posts to /api/items/:id/play', async () => {
    const session = { id: 'sess-1', audioTracks: [] }
    const fetchMock = mockFetch(200, session)
    vi.stubGlobal('fetch', fetchMock)

    const result = await startPlaybackSession(ABS, TOKEN, 'item-1')

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/abs-proxy/api/items/item-1/play',
      expect.objectContaining({ method: 'POST' })
    )
    expect(result.id).toBe('sess-1')
  })

  it('posts to /api/items/:id/play/:episodeId for a podcast episode', async () => {
    const fetchMock = mockFetch(200, { id: 'sess-2', audioTracks: [] })
    vi.stubGlobal('fetch', fetchMock)

    await startPlaybackSession(ABS, TOKEN, 'li_pod', 'ep_7')

    expect(fetchMock.mock.calls[0][0]).toBe('/api/abs-proxy/api/items/li_pod/play/ep_7')
  })

  it('encodes ids so they cannot add path segments', async () => {
    const fetchMock = mockFetch(200, { id: 's', audioTracks: [] })
    vi.stubGlobal('fetch', fetchMock)

    await startPlaybackSession(ABS, TOKEN, 'li_pod', '../../users')

    expect(fetchMock.mock.calls[0][0]).toBe('/api/abs-proxy/api/items/li_pod/play/..%2F..%2Fusers')
  })
})

describe('getProgress for an episode', () => {
  it('asks for /api/me/progress/:id/:episodeId', async () => {
    const fetchMock = mockFetch(200, { currentTime: 5 })
    vi.stubGlobal('fetch', fetchMock)

    await getProgress(ABS, TOKEN, 'li_pod', 'ep_7')

    expect(fetchMock.mock.calls[0][0]).toBe('/api/abs-proxy/api/me/progress/li_pod/ep_7')
  })
})

describe('getLibraryItems', () => {
  it('sends no mediaType (ABS uses the library type)', async () => {
    const fetchMock = mockFetch(200, { results: [], total: 0 })
    vi.stubGlobal('fetch', fetchMock)

    await getLibraryItems(ABS, TOKEN, 'lib_1', 0, 'addedAt', true)

    const url = fetchMock.mock.calls[0][0]
    expect(url.startsWith('/api/abs-proxy/api/libraries/lib_1/items?')).toBe(true)
    expect(url).not.toContain('mediaType')
  })
})

describe('searchLibraryItems', () => {
  it('returns book matches in a book library', async () => {
    vi.stubGlobal('fetch', mockFetch(200, { book: [{ libraryItem: { id: 'b1' } }], authors: [] }))
    const result = await searchLibraryItems(ABS, TOKEN, 'lib_books', 'x')
    expect(result.map(i => i.id)).toEqual(['b1'])
  })

  it('returns podcast matches in a podcast library', async () => {
    vi.stubGlobal('fetch', mockFetch(200, { podcast: [{ libraryItem: { id: 'p1' } }], episodes: [], tags: [] }))
    const result = await searchLibraryItems(ABS, TOKEN, 'lib_pods', 'x')
    expect(result.map(i => i.id)).toEqual(['p1'])
  })
})

describe('getRecentEpisodes', () => {
  it('asks the recent-episodes path and returns the episodes', async () => {
    const fetchMock = mockFetch(200, { episodes: [{ id: 'ep_1' }], limit: 40, page: 2 })
    vi.stubGlobal('fetch', fetchMock)

    const result = await getRecentEpisodes(ABS, TOKEN, 'lib_pods', 2)

    expect(fetchMock.mock.calls[0][0]).toBe('/api/abs-proxy/api/libraries/lib_pods/recent-episodes?limit=40&page=2')
    expect(result).toEqual([{ id: 'ep_1' }])
  })

  it('throws on failure', async () => {
    vi.stubGlobal('fetch', mockFetch(403, {}))
    await expect(getRecentEpisodes(ABS, TOKEN, 'lib_pods')).rejects.toThrow('Failed to fetch episodes')
  })
})

describe('syncProgress', () => {
  it('posts to /api/session/:id/sync', async () => {
    const fetchMock = mockFetch(200, {})
    vi.stubGlobal('fetch', fetchMock)

    await syncProgress(ABS, TOKEN, 'sess-1', 120.5, 10)

    const body = JSON.parse(fetchMock.mock.calls[0][1].body)
    expect(fetchMock.mock.calls[0][0]).toBe('/api/abs-proxy/api/session/sess-1/sync')
    expect(body.currentTime).toBe(120.5)
    expect(body.timeListened).toBe(10)
  })
})

describe('closeSession', () => {
  it('posts to /api/session/:id/close', async () => {
    const fetchMock = mockFetch(200, {})
    vi.stubGlobal('fetch', fetchMock)

    await closeSession(ABS, TOKEN, 'sess-1', 200.0, 5)

    expect(fetchMock.mock.calls[0][0]).toBe('/api/abs-proxy/api/session/sess-1/close')
  })
})

describe('validate-key proxy (integration shape)', () => {
  it('posts api_key to /api/abs/validate-key', async () => {
    const fetchMock = mockFetch(200, {
      user: { id: '1', username: 'alice', token: 'the-key' },
    })
    vi.stubGlobal('fetch', fetchMock)

    const resp = await fetch('/api/abs/validate-key', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ api_key: 'the-key' }),
    })
    const data = await resp.json()

    expect(fetchMock).toHaveBeenCalledWith('/api/abs/validate-key', expect.objectContaining({ method: 'POST' }))
    const body = JSON.parse(fetchMock.mock.calls[0][1].body)
    expect(body.api_key).toBe('the-key')
    expect(data.user.username).toBe('alice')
  })
})

describe('renewSso', () => {
  it('sends the key, the previous key and the bearer token', async () => {
    const grant = { token: 'new', key_id: 'k2', user: { id: 'u', username: 'alice' } }
    const fetchMock = mockFetch(200, grant)
    vi.stubGlobal('fetch', fetchMock)
    const res = await renewSso(TOKEN, 'k1', 'k0')
    expect(fetchMock.mock.calls[0][0]).toBe('/api/auth/sso/renew')
    expect(fetchMock.mock.calls[0][1].method).toBe('POST')
    expect(fetchMock.mock.calls[0][1].headers.Authorization).toBe(`Bearer ${TOKEN}`)
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ key_id: 'k1', previous_key_id: 'k0' })
    expect(res).toEqual({ status: 200, grant })
  })

  it('sends null when there is no previous key, and no grant unless 200', async () => {
    const fetchMock = mockFetch(204, null)
    vi.stubGlobal('fetch', fetchMock)
    const res = await renewSso(TOKEN, 'k1')
    expect(JSON.parse(fetchMock.mock.calls[0][1].body).previous_key_id).toBeNull()
    expect(res).toEqual({ status: 204, grant: null })
  })
})

describe('tokenStatus', () => {
  it("returns ABS's answer for /api/me through the relay", async () => {
    const fetchMock = mockFetch(401, {})
    vi.stubGlobal('fetch', fetchMock)
    expect(await tokenStatus(TOKEN)).toBe(401)
    expect(fetchMock.mock.calls[0][0]).toBe('/api/abs-proxy/api/me')
  })
})
