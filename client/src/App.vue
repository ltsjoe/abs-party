<template>
  <div class="app-root">
    <nav class="nav">
      <span class="nav-brand">🎧 ABS Party</span>
      <span v-if="auth.username" class="nav-user">
        {{ auth.username }}
        <button class="btn-sm" @click="logout">Logout</button>
      </span>
    </nav>
    <div class="app-content">
      <router-view v-if="ready" />
      <p v-else class="checking">Checking your sign-in…</p>
    </div>
  </div>
</template>

<script setup>
import { reactive, provide, ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { ssoLogout, renewSso, tokenStatus } from './api/abs.js'
import { checkSignIn, rememberRoom } from './utils/session.js'

const props = defineProps({
  absBaseUrl: { type: String, default: '' },
  loginOptions: { type: Object, default: () => ({ local: true, saml: false, samlLabel: '' }) },
})

const router = useRouter()

// What localStorage holds for a sign-in. abs_sso_prev_key_id is the SSO key the
// current one replaced at the last renewal: it stays valid for a while so other
// open tabs keep working, and is deleted at the next renewal or at logout.
const KEYS = {
  token: 'abs_token',
  username: 'abs_username',
  userId: 'abs_user_id',
  ssoKeyId: 'abs_sso_key_id',
  prevKeyId: 'abs_sso_prev_key_id',
}

function stored(name) {
  try { return localStorage.getItem(KEYS[name]) || '' } catch { return '' }
}

const auth = reactive({
  token: stored('token'),
  username: stored('username'),
  userId: stored('userId'),
  // Set when signed in through SAML: the ABS API key the server minted for us.
  ssoKeyId: stored('ssoKeyId'),
})

// Nothing renders until the stored sign-in has been checked (and renewed if due),
// so no page starts out with a token that ABS is about to refuse.
const ready = ref(false)

function setAuth(token, username, userId, ssoKeyId = '', prevKeyId = '') {
  auth.token = token
  auth.username = username
  auth.userId = userId
  auth.ssoKeyId = ssoKeyId
  try {
    localStorage.setItem(KEYS.username, username)
    localStorage.setItem(KEYS.userId, userId)
    if (ssoKeyId) localStorage.setItem(KEYS.ssoKeyId, ssoKeyId)
    else localStorage.removeItem(KEYS.ssoKeyId)
    if (prevKeyId) localStorage.setItem(KEYS.prevKeyId, prevKeyId)
    else localStorage.removeItem(KEYS.prevKeyId)
    // Last, because other tabs watch this one (see onStorage).
    localStorage.setItem(KEYS.token, token)
  } catch { /* storage unavailable: signed in for this tab only */ }
}

function clearAuth() {
  auth.token = ''
  auth.username = ''
  auth.userId = ''
  auth.ssoKeyId = ''
  try { for (const k of Object.values(KEYS)) localStorage.removeItem(k) } catch { /* nothing to clear */ }
}

// Take over whatever sign-in another tab stored.
function adoptStoredAuth() {
  auth.token = stored('token')
  auth.username = stored('username')
  auth.userId = stored('userId')
  auth.ssoKeyId = stored('ssoKeyId')
}

async function logout() {
  if (auth.ssoKeyId) {
    // Delete both keys this browser may still hold. Best effort: they run out anyway.
    for (const keyId of [stored('prevKeyId'), auth.ssoKeyId]) {
      if (!keyId) continue
      try { await ssoLogout(auth.token, keyId) } catch { /* key expires anyway */ }
    }
  }
  clearAuth()
  router.push('/')
}

// ABS refused our token. Sign out here, remember the room we were in, and send
// the person to the sign-in page with a note saying why. Resolves once there.
async function signInExpired() {
  // Another tab may have renewed the key in the meantime: then use its sign-in.
  const fresh = stored('token')
  if (fresh && fresh !== auth.token) {
    location.reload()
    return
  }
  rememberRoom(router.currentRoute.value.fullPath)
  clearAuth()
  await router.replace({ path: '/', query: { signin: 'expired' } })
}

async function checkStoredSignIn() {
  // Give up after 8 s and carry on: a slow ABS must not leave the page blank.
  const ctl = new AbortController()
  const timer = setTimeout(() => ctl.abort(), 8000)
  const run = async () => {
    adoptStoredAuth() // another tab may have renewed while this one waited
    const result = await checkSignIn(
      { token: auth.token, ssoKeyId: auth.ssoKeyId, previousKeyId: stored('prevKeyId') },
      {
        renewSso: (t, k, p) => renewSso(t, k, p, ctl.signal),
        tokenStatus: t => tokenStatus(t, ctl.signal),
      },
    )
    if (result.state === 'renewed') {
      const g = result.grant
      setAuth(g.token, g.user.username, g.user.id, g.key_id, auth.ssoKeyId)
    } else if (result.state === 'expired') {
      // Wait for the move to the sign-in page, or the page we were heading
      // for would show first (and a room page, seeing no sign-in, would
      // replace the note with its own).
      await signInExpired()
    }
  }
  try {
    await router.isReady()
    // One tab at a time, so two tabs opening together don't both renew.
    if (navigator.locks?.request) await navigator.locks.request('abs-party-sign-in', run)
    else await run()
  } catch { /* couldn't check: keep the sign-in as it is */ }
  clearTimeout(timer)
  ready.value = true
}

// Keep tabs in step: a renewal or sign-in in one tab updates the others, and a
// logout in one signs them all out (its keys are gone).
function onStorage(e) {
  if (e.key !== KEYS.token) return
  if (e.newValue) {
    adoptStoredAuth()
  } else if (auth.token) {
    auth.token = ''
    auth.username = ''
    auth.userId = ''
    auth.ssoKeyId = ''
    router.push('/')
  }
}

onMounted(() => {
  window.addEventListener('storage', onStorage)
  checkStoredSignIn()
})
onUnmounted(() => window.removeEventListener('storage', onStorage))

provide('auth', auth)
provide('setAuth', setAuth)
provide('signInExpired', signInExpired)
provide('ABS_BASE', props.absBaseUrl)
provide('loginOptions', props.loginOptions)
</script>

<style scoped>
.nav {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.75rem 1.5rem;
  background: #16213e;
  border-bottom: 1px solid #0f3460;
}
.app-root { display: flex; flex-direction: column; height: 100vh; overflow: hidden; }
.app-content { flex: 1; overflow: hidden; min-height: 0; }
.nav-brand { font-size: 1.2rem; font-weight: 700; color: #a29bfe; }
.nav-user { display: flex; align-items: center; gap: 0.75rem; font-size: 0.9rem; }
.checking { text-align: center; color: #888; margin-top: 3rem; }
.btn-sm { padding: 0.25rem 0.75rem; background: #6c5ce7; border: none; border-radius: 4px; color: #fff; cursor: pointer; font-size: 0.8rem; }
</style>
