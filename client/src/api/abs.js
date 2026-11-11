// REST API calls go through our own backend proxy at /api/abs-proxy to avoid CORS issues.
// absBase is still passed for caller convenience but is unused here — it remains needed
// by components that build direct media URLs (cover art, audio streams).

const PROXY = '/api/abs-proxy'

// Ids go into URL paths. Encode them, so a stray "/" or "?" in one can't turn
// the call into a different ABS path. (Real ABS ids never need it.)
const seg = encodeURIComponent

function authHeaders(token) {
  return { Authorization: `Bearer ${token}` }
}

export async function login(username, password) {
  const resp = await fetch('/api/abs/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  })
  if (!resp.ok) throw new Error('Login failed')
  return resp.json()
}

export async function redeemSso(code) {
  const resp = await fetch('/api/auth/sso/redeem', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ code }),
  })
  if (!resp.ok) throw new Error('Sign-in expired')
  return resp.json()
}

// Deletes the ABS API key the server minted for this SSO sign-in. Best effort:
// the key expires on its own anyway.
export async function ssoLogout(token, keyId) {
  await fetch('/api/auth/sso/logout', {
    method: 'POST',
    headers: { ...authHeaders(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({ key_id: keyId }),
  })
}

// Asks the party server to swap this SSO key for a fresh one (see renew in
// server/src/saml.rs). Resolves to { status, grant }: 200 with a new grant,
// 204 not due yet, 401 the sign-in is dead, 404 not an SSO key, anything else
// means it couldn't check. Throws only when the request fails outright.
export async function renewSso(token, keyId, previousKeyId = '', signal = undefined) {
  const resp = await fetch('/api/auth/sso/renew', {
    method: 'POST',
    headers: { ...authHeaders(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({ key_id: keyId, previous_key_id: previousKeyId || null }),
    signal,
  })
  return { status: resp.status, grant: resp.status === 200 ? await resp.json() : null }
}

// The status ABS answers for this token (200: it works; 401: refused).
export async function tokenStatus(token, signal = undefined) {
  const resp = await fetch(`${PROXY}/api/me`, { headers: authHeaders(token), signal })
  return resp.status
}

export async function getMe(_absBase, token) {
  const resp = await fetch(`${PROXY}/api/me`, { headers: authHeaders(token) })
  if (!resp.ok) throw new Error('Failed to fetch user')
  return resp.json()
}

export async function getLibraries(_absBase, token) {
  const resp = await fetch(`${PROXY}/api/libraries`, { headers: authHeaders(token) })
  if (!resp.ok) throw new Error('Failed to fetch libraries')
  return resp.json()
}

// The items of a library: books in a book library, podcasts in a podcast
// library. (ABS picks by the library's own type and ignores a mediaType
// parameter, checked in LibraryController.getLibraryItems, ABS 2.37.2.)
export async function getLibraryItems(_absBase, token, libraryId, page = 0, sort = 'progress', desc = true) {
  const params = new URLSearchParams({ limit: 40, page })
  if (sort) { params.set('sort', sort); params.set('desc', desc ? 1 : 0) }
  const resp = await fetch(
    `${PROXY}/api/libraries/${seg(libraryId)}/items?${params}`,
    { headers: authHeaders(token) }
  )
  if (!resp.ok) throw new Error('Failed to fetch items')
  return resp.json()
}

export async function searchLibraryItems(_absBase, token, libraryId, query) {
  const params = new URLSearchParams({ q: query, limit: 40 })
  const resp = await fetch(`${PROXY}/api/libraries/${seg(libraryId)}/search?${params}`, { headers: authHeaders(token) })
  if (!resp.ok) throw new Error('Search failed')
  const data = await resp.json()
  // ABS answers `book` matches in a book library, `podcast` in a podcast one.
  return (data.book || data.podcast || []).map(b => b.libraryItem)
}

// Newest episodes across a podcast library, skipping ones this user finished.
// ABS returns no total, so a full page (`limit`) means there may be more.
export async function getRecentEpisodes(_absBase, token, libraryId, page = 0, limit = 40) {
  const params = new URLSearchParams({ limit, page })
  const resp = await fetch(
    `${PROXY}/api/libraries/${seg(libraryId)}/recent-episodes?${params}`,
    { headers: authHeaders(token) }
  )
  if (!resp.ok) throw new Error('Failed to fetch episodes')
  const data = await resp.json()
  return data.episodes || []
}

// A podcast's item comes back with all its episodes in media.episodes.
export async function getItem(_absBase, token, itemId) {
  const resp = await fetch(`${PROXY}/api/items/${seg(itemId)}?expanded=1`, { headers: authHeaders(token) })
  if (!resp.ok) throw new Error('Failed to fetch item')
  return resp.json()
}

// ABS keeps progress per book, and per episode in a podcast.
export async function getProgress(_absBase, token, libraryItemId, episodeId = null) {
  const path = episodeId ? `${seg(libraryItemId)}/${seg(episodeId)}` : seg(libraryItemId)
  const resp = await fetch(`${PROXY}/api/me/progress/${path}`, { headers: authHeaders(token) })
  if (resp.status === 404) return null
  if (!resp.ok) throw new Error('Failed to fetch progress')
  return resp.json()
}

export async function getBookmarks(_absBase, token, libraryItemId) {
  const me = await getMe(_absBase, token)
  return (me.bookmarks || []).filter(b => b.libraryItemId === libraryItemId)
}

export async function createBookmark(_absBase, token, libraryItemId, time, title) {
  const resp = await fetch(`${PROXY}/api/me/item/${seg(libraryItemId)}/bookmark`, {
    method: 'POST',
    headers: { ...authHeaders(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({ time, title }),
  })
  if (!resp.ok) throw new Error('Failed to create bookmark')
  return resp.json()
}

// A book plays as a whole item; a podcast plays one episode at a time
// (POST /api/items/:id/play/:episodeId; a podcast without one is a 404).
export async function startPlaybackSession(_absBase, token, libraryItemId, episodeId = null) {
  const path = episodeId ? `${seg(libraryItemId)}/play/${seg(episodeId)}` : `${seg(libraryItemId)}/play`
  const resp = await fetch(`${PROXY}/api/items/${path}`, {
    method: 'POST',
    headers: { ...authHeaders(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({
      mediaPlayer: 'html5',
      supportedMimeTypes: ['audio/mpeg', 'audio/mp4', 'audio/ogg', 'audio/webm', 'audio/flac'],
      deviceInfo: { deviceId: getDeviceId(), deviceName: 'ABS Party Web' },
      forceDirectPlay: false,
    }),
  })
  if (!resp.ok) throw new Error('Failed to start playback session')
  return resp.json()
}

export async function syncProgress(_absBase, token, sessionId, currentTime, timeListened) {
  await fetch(`${PROXY}/api/session/${sessionId}/sync`, {
    method: 'POST',
    headers: { ...authHeaders(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({ currentTime, timeListened }),
  })
}

export async function closeSession(_absBase, token, sessionId, currentTime, timeListened) {
  await fetch(`${PROXY}/api/session/${sessionId}/close`, {
    method: 'POST',
    headers: { ...authHeaders(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({ currentTime, timeListened }),
  })
}

function getDeviceId() {
  let id = localStorage.getItem('absPartyDeviceId')
  if (!id) {
    id = crypto.randomUUID()
    localStorage.setItem('absPartyDeviceId', id)
  }
  return id
}
