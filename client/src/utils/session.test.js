import { describe, it, expect, vi } from 'vitest'
import { checkSignIn, isRoomPath, rememberRoom, takeRememberedRoom, hasRememberedRoom } from './session.js'

const GRANT = { token: 'new-tok', key_id: 'k2', user: { id: 'u1', username: 'alice' } }

function api({ renew = { status: 204, grant: null }, me = 200 } = {}) {
  return {
    renewSso: vi.fn(async () => (renew instanceof Error ? Promise.reject(renew) : renew)),
    tokenStatus: vi.fn(async () => (me instanceof Error ? Promise.reject(me) : me)),
  }
}

describe('checkSignIn', () => {
  it('has nothing to check without a token', async () => {
    const a = api()
    expect(await checkSignIn({ token: '' }, a)).toEqual({ state: 'none' })
    expect(a.renewSso).not.toHaveBeenCalled()
    expect(a.tokenStatus).not.toHaveBeenCalled()
  })

  it('renews an SSO key, passing the previous key along', async () => {
    const a = api({ renew: { status: 200, grant: GRANT } })
    const res = await checkSignIn({ token: 't', ssoKeyId: 'k1', previousKeyId: 'k0' }, a)
    expect(res).toEqual({ state: 'renewed', grant: GRANT })
    expect(a.renewSso).toHaveBeenCalledWith('t', 'k1', 'k0')
    expect(a.tokenStatus).not.toHaveBeenCalled()
  })

  it('keeps an SSO key that is not due', async () => {
    expect((await checkSignIn({ token: 't', ssoKeyId: 'k1' }, api())).state).toBe('ok')
  })

  it('reports a refused SSO key as expired', async () => {
    const a = api({ renew: { status: 401, grant: null } })
    expect((await checkSignIn({ token: 't', ssoKeyId: 'k1' }, a)).state).toBe('expired')
  })

  it('keeps the sign-in when the server or ABS is down', async () => {
    for (const renew of [{ status: 502, grant: null }, { status: 500, grant: null }, new Error('offline')]) {
      expect((await checkSignIn({ token: 't', ssoKeyId: 'k1' }, api({ renew }))).state).toBe('unknown')
    }
  })

  it('falls back to checking the token when the key is not an SSO key', async () => {
    const ok = api({ renew: { status: 404, grant: null }, me: 200 })
    expect((await checkSignIn({ token: 't', ssoKeyId: 'k1' }, ok)).state).toBe('ok')
    const dead = api({ renew: { status: 404, grant: null }, me: 401 })
    expect((await checkSignIn({ token: 't', ssoKeyId: 'k1' }, dead)).state).toBe('expired')
  })

  it('ignores a 200 without a usable grant', async () => {
    const a = api({ renew: { status: 200, grant: { token: '' } } })
    expect((await checkSignIn({ token: 't', ssoKeyId: 'k1' }, a)).state).toBe('unknown')
  })

  it('checks a password sign-in against ABS', async () => {
    expect((await checkSignIn({ token: 't' }, api({ me: 200 }))).state).toBe('ok')
    expect((await checkSignIn({ token: 't' }, api({ me: 401 }))).state).toBe('expired')
    expect((await checkSignIn({ token: 't' }, api({ me: 403 }))).state).toBe('expired')
    expect((await checkSignIn({ token: 't' }, api({ me: 502 }))).state).toBe('unknown')
    expect((await checkSignIn({ token: 't' }, api({ me: new Error('offline') }))).state).toBe('unknown')
  })
})

function memoryStorage() {
  const m = new Map()
  return {
    getItem: k => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => m.set(k, String(v)),
    removeItem: k => m.delete(k),
  }
}

describe('remembered room', () => {
  it('only takes room paths', () => {
    expect(isRoomPath('/room/abc12345')).toBe(true)
    expect(isRoomPath('/room/abc12345?episodeId=ep_1')).toBe(true)
    for (const p of ['/', '/pick', '/room/', '//evil.example/room/x', 'https://evil.example/room/x', null]) {
      expect(isRoomPath(p)).toBe(false)
    }
  })

  it('is handed back once', () => {
    const s = memoryStorage()
    rememberRoom('/room/abc12345?episodeId=ep_1', s)
    expect(hasRememberedRoom(s)).toBe(true)
    expect(takeRememberedRoom(s)).toBe('/room/abc12345?episodeId=ep_1')
    expect(takeRememberedRoom(s)).toBeNull()
    expect(hasRememberedRoom(s)).toBe(false)
  })

  it('does not remember other pages', () => {
    const s = memoryStorage()
    rememberRoom('/pick', s)
    expect(takeRememberedRoom(s)).toBeNull()
  })

  it('copes with storage that throws', () => {
    const broken = { getItem() { throw new Error('denied') }, setItem() { throw new Error('denied') }, removeItem() {} }
    expect(() => rememberRoom('/room/abc12345', broken)).not.toThrow()
    expect(takeRememberedRoom(broken)).toBeNull()
    expect(hasRememberedRoom(broken)).toBe(false)
  })
})
