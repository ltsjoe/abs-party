// Chapter maths for the room's chapter list and the ⏮ / ⏭ buttons.
//
// ABS sends a book's (or an episode's) chapters with each playback session:
// [{ id, start, end, title }], times in seconds from the start of the item.
// See PlaybackSession.chapters in ABS (server/objects/PlaybackSession.js).

// Small slack for "are we at this chapter yet?": after seeking to a chapter's
// start, the player can report a time a hair before it.
const SLACK = 0.5

// Pressing ⏮ more than this many seconds into a chapter goes back to its start,
// like a music player; pressing it sooner goes to the chapter before.
export const RESTART_GRACE = 3

// Cleans up what ABS sent: drops entries without usable times, sorts by start,
// and fills in a title where one is missing.
export function normalizeChapters(raw) {
  if (!Array.isArray(raw)) return []
  return raw
    .filter(c => c && Number.isFinite(Number(c.start)) && Number(c.start) >= 0)
    .map(c => ({ id: c.id, start: Number(c.start), end: Number(c.end), title: String(c.title ?? '').trim() }))
    .sort((a, b) => a.start - b.start)
    .map((c, i) => ({ ...c, title: c.title || `Chapter ${i + 1}` }))
}

// Index of the chapter playing at `time`, or -1 before the first one.
export function chapterIndexAt(chapters, time) {
  let idx = -1
  for (let i = 0; i < chapters.length; i++) {
    if (chapters[i].start <= time + SLACK) idx = i
    else break
  }
  return idx
}

// Where ⏮ goes from `time`.
export function prevChapterStart(chapters, time, grace = RESTART_GRACE) {
  const i = chapterIndexAt(chapters, time)
  if (i < 0) return 0
  if (time - chapters[i].start > grace || i === 0) return chapters[i].start
  return chapters[i - 1].start
}

// Where ⏭ goes from `time`, or null in the last chapter.
export function nextChapterStart(chapters, time) {
  const next = chapters[chapterIndexAt(chapters, time) + 1]
  return next ? next.start : null
}
