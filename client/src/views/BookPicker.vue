<template>
  <div class="page">
    <div class="picker">
      <header class="picker-header">
        <h2>{{ isPodcastLib ? 'Pick a podcast episode' : 'Pick an audiobook' }}</h2>
        <button class="btn-ghost" @click="router.back()">← Back</button>
      </header>

      <div v-if="pageLoading" class="status">Loading libraries…</div>
      <div v-else-if="error" class="error">{{ error }}</div>

      <template v-else>
        <div class="lib-tabs">
          <button
            v-for="lib in libraries"
            :key="lib.id"
            :class="['lib-tab', { active: selectedLibId === lib.id }]"
            @click="selectLib(lib.id)"
          >{{ lib.name }}</button>
        </div>

        <!-- A podcast library: browse its podcasts, or the newest episodes of all of them -->
        <div v-if="isPodcastLib && !openPodcast" class="lib-tabs">
          <button
            :class="['lib-tab', { active: podcastMode === 'podcasts' }]"
            @click="setPodcastMode('podcasts')"
          >All podcasts</button>
          <button
            :class="['lib-tab', { active: podcastMode === 'latest' }]"
            @click="setPodcastMode('latest')"
          >Latest episodes</button>
        </div>

        <!-- One podcast's episodes, newest first -->
        <template v-if="openPodcast">
          <div class="podcast-head">
            <button class="btn-ghost" @click="closePodcast">← All podcasts</button>
            <div>
              <div class="podcast-title">{{ openPodcast.media?.metadata?.title || 'Unknown' }}</div>
              <div class="book-author">{{ openPodcast.media?.metadata?.author || '' }}</div>
            </div>
          </div>

          <div class="toolbar">
            <input v-model="episodeFilter" class="search" placeholder="Filter episodes by title…" />
          </div>

          <ul class="episode-list">
            <li
              v-for="ep in shownEpisodes"
              :key="ep.id"
              class="episode-row"
              @click="pickEpisode(openPodcast.id, openPodcast.media?.metadata, ep)"
            >
              <div class="episode-title">{{ ep.title || 'Untitled episode' }}</div>
              <div class="episode-meta">
                <span v-if="formatDate(ep.publishedAt)">{{ formatDate(ep.publishedAt) }}</span>
                <span v-if="formatDuration(ep.duration)">{{ formatDuration(ep.duration) }}</span>
                <span v-if="progressLabel(episodeProgress[ep.id])" class="episode-progress">
                  {{ progressLabel(episodeProgress[ep.id]) }}
                </span>
              </div>
            </li>
            <li v-if="shownEpisodes.length === 0" class="status">No episodes found.</li>
          </ul>
        </template>

        <!-- The newest episodes across the library (ABS leaves out ones you finished) -->
        <template v-else-if="isPodcastLib && podcastMode === 'latest'">
          <ul :class="['episode-list', { 'grid--loading': gridLoading }]">
            <li
              v-for="ep in latestEpisodes"
              :key="ep.id"
              class="episode-row episode-row--cover"
              @click="pickEpisode(ep.libraryItemId, ep.podcast?.metadata, ep, ep.libraryId)"
            >
              <img
                v-if="ep.podcast?.coverPath"
                :src="`${absBase}/api/items/${ep.libraryItemId}/cover`"
                alt=""
                class="episode-cover"
              />
              <div v-else class="episode-cover episode-cover--empty">🎙</div>
              <div class="episode-text">
                <div class="episode-title">{{ ep.title || 'Untitled episode' }}</div>
                <div class="book-author">{{ ep.podcast?.metadata?.title || '' }}</div>
                <div class="episode-meta">
                  <span v-if="formatDate(ep.publishedAt)">{{ formatDate(ep.publishedAt) }}</span>
                  <span v-if="formatDuration(ep.duration)">{{ formatDuration(ep.duration) }}</span>
                </div>
              </div>
            </li>
            <li v-if="!gridLoading && latestEpisodes.length === 0" class="status">No unfinished episodes.</li>
          </ul>

          <!-- ABS gives no total here: a full page means there may be another -->
          <div v-if="page > 0 || latestHasMore" class="pagination">
            <button :disabled="page === 0" @click="changePage(page - 1)">‹</button>
            <span>{{ page + 1 }}</span>
            <button :disabled="!latestHasMore" @click="changePage(page + 1)">›</button>
          </div>
        </template>

        <!-- Books, or a podcast library's podcasts -->
        <template v-else>
          <div class="toolbar">
            <input
              v-model="search"
              class="search"
              :placeholder="isPodcastLib ? 'Search podcasts…' : 'Filter by title or author…'"
            />
            <select v-model="sort" class="sort-select" @change="onSortChange">
              <option v-for="o in sortOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
            </select>
          </div>

          <div :class="['grid', { 'grid--loading': gridLoading }]">
            <div
              v-for="item in items"
              :key="item.id"
              class="book-card"
              @click="onCardClick(item)"
            >
              <img
                v-if="item.media?.coverPath"
                :src="`${absBase}/api/items/${item.id}/cover`"
                :alt="item.media.metadata.title"
                class="cover"
                @load="e => e.target.classList.add('loaded')"
              />
              <div v-else class="cover cover-placeholder">🎵</div>
              <div class="book-info">
                <div class="book-title">{{ item.media?.metadata?.title || 'Unknown' }}</div>
                <!-- books have authorName, podcasts author -->
                <div class="book-author">{{ item.media?.metadata?.authorName || item.media?.metadata?.author || '' }}</div>
              </div>
            </div>
          </div>

          <div v-if="totalPages > 1 && !search" class="pagination">
            <button :disabled="page === 0" @click="changePage(page - 1)">‹</button>
            <span>{{ page + 1 }} / {{ totalPages }}</span>
            <button :disabled="page >= totalPages - 1" @click="changePage(page + 1)">›</button>
          </div>
        </template>
      </template>

      <div class="room-name-overlay" v-if="pickedItem">
        <div class="overlay-card">
          <h3>Create room for "{{ pickedItem.title }}"?</h3>
          <p v-if="pickedItem.episodeId && pickedItem.author" class="overlay-sub">{{ pickedItem.author }}</p>
          <p v-if="createError" class="error">{{ createError }}</p>
          <div class="overlay-actions">
            <button @click="pickedItem = null">Cancel</button>
            <button class="primary" @click="createRoom">Create Room</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, inject, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { getLibraries, getLibraryItems, searchLibraryItems, getRecentEpisodes, getItem, getMe } from '../api/abs.js'
import {
  sortNewestFirst, formatDuration, formatDate, progressByEpisode, progressLabel, bookChoice, episodeChoice,
} from '../utils/episodes.js'

const router = useRouter()
const auth = inject('auth')
const absBase = inject('ABS_BASE')

const BOOK_SORTS = [
  { value: 'progress', label: 'Last listened' },
  { value: 'media.metadata.title', label: 'Title' },
  { value: 'addedAt', label: 'Date added' },
]
// ABS has no "last listened" order for podcasts
// (libraryItemsPodcastFilters.getOrder, ABS 2.37.2).
const PODCAST_SORTS = [
  { value: 'media.metadata.title', label: 'Title' },
  { value: 'addedAt', label: 'Date added' },
]

const libraries = ref([])
const selectedLibId = ref('')
const items = ref([])
const pageLoading = ref(true)
const gridLoading = ref(false)
const error = ref('')
const search = ref('')
const sort = ref('progress')
const page = ref(0)
const totalItems = ref(0)
const PAGE_SIZE = 40

const totalPages = computed(() => Math.ceil(totalItems.value / PAGE_SIZE))

const selectedLib = computed(() => libraries.value.find(l => l.id === selectedLibId.value))
const isPodcastLib = computed(() => selectedLib.value?.mediaType === 'podcast')
const sortOptions = computed(() => (isPodcastLib.value ? PODCAST_SORTS : BOOK_SORTS))

// Podcast libraries only.
const podcastMode = ref('podcasts')   // 'podcasts' (grid) or 'latest' (episode list)
const openPodcast = ref(null)         // the podcast whose episodes are listed, with media.episodes
const episodeProgress = ref({})       // episode id -> this user's ABS progress
const episodeFilter = ref('')
const latestEpisodes = ref([])
const latestHasMore = ref(false)

const shownEpisodes = computed(() => {
  const episodes = sortNewestFirst(openPodcast.value?.media?.episodes || [])
  const q = episodeFilter.value.trim().toLowerCase()
  return q ? episodes.filter(ep => (ep.title || '').toLowerCase().includes(q)) : episodes
})

let searchDebounce = null
watch(search, () => {
  clearTimeout(searchDebounce)
  searchDebounce = setTimeout(() => {
    page.value = 0
    loadResults()
  }, 350)
})

// What the "Create room" box is about: see bookChoice/episodeChoice.
const pickedItem = ref(null)
const createError = ref('')

onMounted(async () => {
  try {
    const data = await getLibraries(absBase, auth.token)
    libraries.value = data.libraries || []
    if (libraries.value.length > 0) {
      await selectLib(libraries.value[0].id)
    }
  } catch (e) {
    error.value = 'Failed to load libraries. Are you still signed in?'
  } finally {
    pageLoading.value = false
  }
})

async function selectLib(libId) {
  selectedLibId.value = libId
  page.value = 0
  openPodcast.value = null
  podcastMode.value = 'podcasts'
  // Book and podcast libraries sort differently; fall back to the first order.
  if (!sortOptions.value.some(o => o.value === sort.value)) {
    sort.value = sortOptions.value[0].value
  }
  await loadResults()
}

async function loadResults() {
  gridLoading.value = true
  try {
    const q = search.value.trim()
    if (q) {
      items.value = await searchLibraryItems(absBase, auth.token, selectedLibId.value, q)
      totalItems.value = 0
    } else {
      const desc = sort.value !== 'media.metadata.title'
      const data = await getLibraryItems(absBase, auth.token, selectedLibId.value, page.value, sort.value, desc)
      items.value = data.results || []
      totalItems.value = data.total || 0
    }
    if (sort.value === 'media.metadata.title') {
      items.value = [...items.value].sort((a, b) => {
        const ta = a.media?.metadata?.title?.toLowerCase() ?? ''
        const tb = b.media?.metadata?.title?.toLowerCase() ?? ''
        return ta.localeCompare(tb)
      })
    }
  } catch (e) {
    error.value = 'Failed to load.'
  } finally {
    gridLoading.value = false
  }
}

async function loadLatest() {
  gridLoading.value = true
  try {
    const episodes = await getRecentEpisodes(absBase, auth.token, selectedLibId.value, page.value, PAGE_SIZE)
    latestEpisodes.value = episodes
    latestHasMore.value = episodes.length === PAGE_SIZE
  } catch (e) {
    error.value = 'Failed to load episodes.'
  } finally {
    gridLoading.value = false
  }
}

async function setPodcastMode(mode) {
  podcastMode.value = mode
  page.value = 0
  if (mode === 'latest') await loadLatest()
  else await loadResults()
}

async function changePage(p) {
  page.value = p
  if (isPodcastLib.value && podcastMode.value === 'latest') await loadLatest()
  else await loadResults()
}

async function onSortChange() {
  clearTimeout(searchDebounce)
  page.value = 0
  await loadResults()
}

function onCardClick(item) {
  if (isPodcastLib.value) showPodcast(item)
  else pick(bookChoice(item, selectedLibId.value))
}

// The library list has no episodes in it, so fetch the podcast's full item,
// plus this user's progress (from /api/me) to mark finished episodes.
async function showPodcast(item) {
  gridLoading.value = true
  try {
    const [full, me] = await Promise.all([
      getItem(absBase, auth.token, item.id),
      getMe(absBase, auth.token).catch(() => null), // the marks are a nice-to-have
    ])
    episodeProgress.value = progressByEpisode(me, full.id)
    episodeFilter.value = ''
    openPodcast.value = full
  } catch (e) {
    error.value = 'Failed to load episodes.'
  } finally {
    gridLoading.value = false
  }
}

function closePodcast() {
  openPodcast.value = null
}

function pickEpisode(libraryItemId, podcastMeta, episode, libraryId) {
  pick(episodeChoice(libraryItemId, podcastMeta, episode, libraryId || selectedLibId.value))
}

function pick(choice) {
  pickedItem.value = choice
  createError.value = ''
}

function createRoom() {
  const choice = pickedItem.value
  const roomId = crypto.randomUUID().replace(/-/g, '').slice(0, 8)
  const query = {
    host: 'true',
    itemId: choice.itemId,
    libraryId: choice.libraryId,
    title: choice.title,
    author: choice.author,
  }
  // A podcast room plays one episode; Room.vue sends it as episode_id.
  if (choice.episodeId) query.episodeId = choice.episodeId
  router.push({ path: `/room/${roomId}`, query })
}
</script>

<style scoped>
.page { height: 100%; overflow-y: auto; padding: 1.5rem; box-sizing: border-box; }
.picker { max-width: 900px; margin: 0 auto; }
.picker-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 1rem; }
h2 { margin: 0; }
.btn-ghost { background: none; border: 1px solid #444; color: #ccc; padding: 0.4rem 0.9rem; border-radius: 6px; cursor: pointer; }
.lib-tabs { display: flex; gap: 0.5rem; flex-wrap: wrap; margin-bottom: 0.75rem; }
.lib-tab { background: #16213e; border: 1px solid #0f3460; color: #ccc; padding: 0.35rem 0.9rem; border-radius: 20px; cursor: pointer; font-size: 0.85rem; }
.lib-tab.active { background: #6c5ce7; border-color: #6c5ce7; color: #fff; }
.toolbar { display: flex; gap: 0.75rem; margin-bottom: 1rem; }
.search { flex: 1; padding: 0.6rem 0.75rem; border: 1px solid #0f3460; border-radius: 6px; background: #16213e; color: #fff; font-size: 1rem; }
.sort-select { padding: 0.6rem 0.75rem; border: 1px solid #0f3460; border-radius: 6px; background: #16213e; color: #ccc; font-size: 0.85rem; cursor: pointer; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 1rem; transition: opacity 0.15s; }
.grid--loading { opacity: 0.4; pointer-events: none; }
.book-card { background: #16213e; border-radius: 8px; overflow: hidden; cursor: pointer; transition: transform 0.15s; }
.book-card:hover { transform: translateY(-3px); }
.cover { width: 100%; aspect-ratio: 1; object-fit: cover; background: #0f3460; filter: blur(8px); opacity: 0; transition: filter 0.35s ease, opacity 0.35s ease; }
.cover.loaded { filter: blur(0); opacity: 1; }
.cover-placeholder { width: 100%; aspect-ratio: 1; background: #0f3460; display: flex; align-items: center; justify-content: center; font-size: 2.5rem; }
.book-info { padding: 0.5rem; }
.book-title { font-size: 0.8rem; font-weight: 600; color: #e0e0e0; line-height: 1.3; }
.book-author { font-size: 0.75rem; color: #888; margin-top: 0.2rem; }
.podcast-head { display: flex; align-items: center; gap: 0.75rem; margin-bottom: 0.75rem; flex-wrap: wrap; }
.podcast-title { font-weight: 600; color: #e0e0e0; }
.episode-list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.5rem; transition: opacity 0.15s; }
.episode-row { background: #16213e; border-radius: 8px; padding: 0.65rem 0.8rem; cursor: pointer; transition: background 0.15s; }
.episode-row:hover { background: #1c2a50; }
.episode-row--cover { display: flex; gap: 0.75rem; align-items: center; }
.episode-cover { width: 56px; height: 56px; border-radius: 6px; object-fit: cover; flex-shrink: 0; background: #0f3460; }
.episode-cover--empty { display: flex; align-items: center; justify-content: center; font-size: 1.5rem; }
.episode-text { min-width: 0; }
.episode-title { font-size: 0.9rem; font-weight: 600; color: #e0e0e0; line-height: 1.3; }
.episode-meta { display: flex; gap: 0.75rem; flex-wrap: wrap; font-size: 0.75rem; color: #888; margin-top: 0.25rem; }
.episode-progress { color: #a29bfe; }
.pagination { display: flex; align-items: center; gap: 1rem; justify-content: center; margin-top: 1.5rem; }
.pagination button { background: #16213e; border: 1px solid #0f3460; color: #ccc; padding: 0.4rem 0.9rem; border-radius: 6px; cursor: pointer; }
.pagination button:disabled { opacity: 0.4; cursor: default; }
.status { text-align: center; color: #888; padding: 3rem; }
.error { color: #ff7675; }
.room-name-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.7); display: flex; align-items: center; justify-content: center; z-index: 100; }
.overlay-card { background: #16213e; border-radius: 12px; padding: 2rem; width: 100%; max-width: 380px; }
.overlay-card h3 { margin: 0 0 1rem; }
.overlay-sub { margin: -0.5rem 0 1rem; color: #888; font-size: 0.85rem; }
.overlay-card label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; color: #aaa; margin-bottom: 1rem; }
.overlay-card input { padding: 0.6rem 0.75rem; border: 1px solid #0f3460; border-radius: 6px; background: #0f3460; color: #fff; font-size: 1rem; }
.overlay-actions { display: flex; gap: 0.75rem; }
.overlay-actions button { flex: 1; padding: 0.65rem; border: none; border-radius: 6px; cursor: pointer; background: #2d3436; color: #ccc; }
.overlay-actions .primary { background: #6c5ce7; color: #fff; }
</style>
