<template>
  <div class="page">
    <div class="card">
      <h1>ABS Party</h1>
      <p class="sub">Listen to audiobooks together, in sync.</p>

      <template v-if="!auth.token">
        <h2>Sign in</h2>

        <template v-if="loginOptions.saml">
          <a class="sso-button" href="/auth/saml/login" @click="loading = true">
            {{ loginOptions.samlLabel || 'Sign in with single sign-on' }}
          </a>
          <p v-if="loginOptions.local" class="divider">or use your Audiobookshelf password</p>
        </template>
        <p v-if="error && !loginOptions.local" class="error">{{ error }}</p>

        <form v-if="loginOptions.local" @submit.prevent="doLogin">
          <label>Username
            <input v-model="form.username" required autocomplete="username" />
          </label>
          <label>Password
            <input v-model="form.password" type="password" required autocomplete="current-password" />
          </label>
          <p v-if="error" class="error">{{ error }}</p>
          <button type="submit" :disabled="loading">{{ loading ? 'Signing in…' : 'Sign in' }}</button>
        </form>
      </template>

      <template v-else>
        <div class="actions">
          <button @click="goCreate">Create a room</button>
        </div>
        <hr />
        <h2>Join a room</h2>
        <form @submit.prevent="doJoin">
          <label>Room code
            <input v-model="joinCode" placeholder="e.g. a1b2c3d4" required />
          </label>
          <p v-if="joinError" class="error">{{ joinError }}</p>
          <button type="submit">Join</button>
        </form>
      </template>
    </div>
  </div>
</template>

<script setup>
import { inject, ref, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { login, redeemSso } from '../api/abs.js'

const router = useRouter()
const route = useRoute()
const auth = inject('auth')
const setAuth = inject('setAuth')
const loginOptions = inject('loginOptions')

const SSO_ERRORS = {
  expired: 'That sign-in took too long or was already used. Please try again.',
  invalid: 'The sign-in response could not be verified. Please try again.',
  no_username: 'Single sign-on did not say who you are. Ask the admin to check the SAML username mapping.',
  no_account: 'You do not have an Audiobookshelf account here yet. Ask the admin to add one with the same username.',
  disabled: 'Your Audiobookshelf account is disabled.',
  root: 'The Audiobookshelf root account cannot use single sign-on. Use its password instead.',
  abs: 'Audiobookshelf could not be reached. Please try again later.',
}

const form = ref({ username: '', password: '' })
const loading = ref(false)
const error = ref('')

const joinCode = ref('')
const joinError = ref('')

async function doLogin() {
  error.value = ''
  loading.value = true
  try {
    const data = await login(form.value.username, form.value.password)
    setAuth(data.user.token, data.user.username, data.user.id)
    resumeAfterSignIn()
  } catch (e) {
    error.value = 'Login failed. Check your credentials.'
  } finally {
    loading.value = false
  }
}

// Room.vue remembers a room link opened while signed out; go back to it.
function resumeAfterSignIn() {
  let target = null
  try {
    target = sessionStorage.getItem('abs_party_return')
    sessionStorage.removeItem('abs_party_return')
  } catch { /* storage unavailable */ }
  if (target && target.startsWith('/room/')) router.replace(target)
}

onMounted(async () => {
  const { sso, sso_error: ssoError } = route.query
  if (!sso && !ssoError) return
  // Drop the code from the address bar (and history) straight away.
  router.replace('/')
  if (ssoError) {
    error.value = SSO_ERRORS[ssoError] || 'Single sign-on failed. Please try again.'
    return
  }
  loading.value = true
  try {
    const grant = await redeemSso(String(sso))
    setAuth(grant.token, grant.user.username, grant.user.id, grant.key_id)
    resumeAfterSignIn()
  } catch {
    error.value = SSO_ERRORS.expired
  } finally {
    loading.value = false
  }
})

function goCreate() {
  router.push('/pick')
}

function doJoin() {
  if (!joinCode.value.trim()) return
  router.push(`/room/${joinCode.value.trim()}`)
}
</script>

<style scoped>
.page { display: flex; justify-content: center; padding: 3rem 1rem; }
.card { background: #16213e; border-radius: 12px; padding: 2rem; width: 100%; max-width: 420px; }
h1 { margin: 0 0 0.25rem; color: #a29bfe; }
.sub { margin: 0 0 1.5rem; color: #888; }
h2 { margin: 1rem 0 0.75rem; font-size: 1rem; color: #ccc; }
form { display: flex; flex-direction: column; gap: 0.75rem; }
label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; color: #aaa; }
input { padding: 0.6rem 0.75rem; border: 1px solid #0f3460; border-radius: 6px; background: #0f3460; color: #fff; font-size: 1rem; }
button[type=submit] { padding: 0.7rem; background: #6c5ce7; border: none; border-radius: 6px; color: #fff; font-size: 1rem; cursor: pointer; }
button[type=submit]:disabled { opacity: 0.6; cursor: default; }
.sso-button { display: block; text-align: center; text-decoration: none; padding: 0.7rem; background: #6c5ce7; border-radius: 6px; color: #fff; font-size: 1rem; }
.divider { text-align: center; color: #888; font-size: 0.8rem; margin: 1rem 0 0.75rem; }
.error { color: #ff7675; font-size: 0.85rem; margin: 0; }
.actions button { width: 100%; padding: 0.7rem; background: #6c5ce7; border: none; border-radius: 6px; color: #fff; font-size: 1rem; cursor: pointer; }
hr { border: none; border-top: 1px solid #0f3460; margin: 1.5rem 0; }
</style>
