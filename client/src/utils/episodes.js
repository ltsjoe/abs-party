// Small helpers for listing podcast episodes in the picker.
// Episode fields are ABS's (PodcastEpisode.toOldJSONExpanded, ABS 2.37):
// id, title, publishedAt (ms or null), addedAt (ms), duration (seconds).

// Newest first: by publish date, else by when ABS added it.
export function sortNewestFirst(episodes) {
  const when = ep => ep.publishedAt ?? ep.addedAt ?? 0
  return [...episodes].sort((a, b) => when(b) - when(a))
}

// "1 h 05 min", "42 min", or '' when ABS doesn't know.
export function formatDuration(seconds) {
  if (!seconds || !isFinite(seconds) || seconds <= 0) return ''
  const totalMin = Math.round(seconds / 60)
  if (totalMin < 60) return `${Math.max(1, totalMin)} min`
  const h = Math.floor(totalMin / 60)
  const m = totalMin % 60
  return `${h} h ${String(m).padStart(2, '0')} min`
}

export function formatDate(ms) {
  if (!ms) return ''
  const d = new Date(ms)
  return isNaN(d) ? '' : d.toLocaleDateString()
}

// episodeId -> this user's ABS progress, for one podcast, from GET /api/me.
export function progressByEpisode(me, libraryItemId) {
  const map = {}
  for (const mp of me?.mediaProgress || []) {
    if (mp.libraryItemId === libraryItemId && mp.episodeId) map[mp.episodeId] = mp
  }
  return map
}

// "Finished", "35%", or '' for an episode not started.
export function progressLabel(mp) {
  if (!mp) return ''
  if (mp.isFinished) return 'Finished'
  const pct = Math.floor((mp.progress || 0) * 100)
  return pct > 0 ? `${pct}%` : ''
}

// What a room needs to know about the thing being played.
export function bookChoice(item, libraryId) {
  const meta = item.media?.metadata || {}
  return {
    itemId: item.id,
    episodeId: null,
    title: meta.title || 'Untitled',
    author: meta.authorName || '',
    libraryId,
  }
}

// `podcastMeta` is the podcast's metadata ({ title, author, ... }): at
// item.media.metadata on a library item, at ep.podcast.metadata on a
// recent-episodes entry.
export function episodeChoice(libraryItemId, podcastMeta, episode, libraryId) {
  return {
    itemId: libraryItemId,
    episodeId: episode.id,
    title: episode.title || 'Untitled episode',
    author: podcastMeta?.title || '',
    libraryId,
  }
}
