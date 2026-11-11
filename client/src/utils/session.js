// Keeping a sign-in alive, and recovering cleanly when it has died.
//
// A single sign-on gets an ABS API key that lasts SSO_KEY_TTL_HOURS (90 days by
// default). Each time the page loads it asks the party server to swap that key
// for a fresh one (POST /api/auth/sso/renew, at most once a day), so a sign-in
// that keeps being used never runs out. A password sign-in's token is checked
// against ABS instead (GET /api/me through the relay).

// What the page should do with the sign-in it has stored:
//   none     nothing stored
//   ok       it works, keep it
//   renewed  swap in `grant` (same shape as the /redeem reply)
//   expired  ABS refused it: sign out and ask for a fresh sign-in
//   unknown  couldn't check (offline, ABS down): keep it, it may well be fine
//
// `api` is { renewSso(token, keyId, previousKeyId), tokenStatus(token) }, both
// resolving to an HTTP status (renewSso to { status, grant }).
export async function checkSignIn({ token, ssoKeyId, previousKeyId }, api) {
  if (!token) return { state: 'none' }
  try {
    if (ssoKeyId) {
      const { status, grant } = await api.renewSso(token, ssoKeyId, previousKeyId)
      if (status === 200 && grant?.token && grant?.key_id) return { state: 'renewed', grant }
      if (status === 204) return { state: 'ok' }
      if (status === 401) return { state: 'expired' }
      // 404: not one of our SSO keys (or SSO is off now). Check the token itself.
      if (status !== 404) return { state: 'unknown' }
    }
    const status = await api.tokenStatus(token)
    if (status >= 200 && status < 300) return { state: 'ok' }
    if (status === 401 || status === 403) return { state: 'expired' }
    return { state: 'unknown' }
  } catch {
    return { state: 'unknown' }
  }
}

// A room link opened while signed out (or when the sign-in had died) is kept in
// sessionStorage, so the person lands back in that room after signing in. It
// survives the trip to the SSO provider because that happens in the same tab.
const RETURN_KEY = 'abs_party_return'

export function isRoomPath(path) {
  return typeof path === 'string' && /^\/room\/[^/]/.test(path)
}

export function rememberRoom(path, storage = globalThis.sessionStorage) {
  if (!isRoomPath(path)) return
  try { storage.setItem(RETURN_KEY, path) } catch { /* storage unavailable */ }
}

// Returns the remembered room path once, or null.
export function takeRememberedRoom(storage = globalThis.sessionStorage) {
  let path = null
  try {
    path = storage.getItem(RETURN_KEY)
    storage.removeItem(RETURN_KEY)
  } catch { /* storage unavailable */ }
  return isRoomPath(path) ? path : null
}

export function hasRememberedRoom(storage = globalThis.sessionStorage) {
  try { return isRoomPath(storage.getItem(RETURN_KEY)) } catch { return false }
}
