// ABS bookmarks belong to a library item and have no episode field
// ({ libraryItemId, time, title }), and a whole podcast is ONE library item.
// So in a podcast the episode's name goes in front of the bookmark's title,
// "[Episode title] note", and a room lists only its own episode's bookmarks.
// (ABS's own web player hides bookmarks for podcasts altogether.)

// What a room's bookmarks are named after: '' for a book, else the episode.
export function episodeLabel(item) {
  if (!item?.episode_id) return ''
  return item.title || item.episode_id
}

export function episodeBookmarkTitle(title, label) {
  return label ? `[${label}] ${title}` : title
}

// The bookmarks to show in a room: all of the item's for a book; for an
// episode only those named for it, with the "[Episode title] " part removed.
export function bookmarksForRoom(bookmarks, label) {
  if (!label) return bookmarks
  const prefix = `[${label}] `
  return bookmarks
    .filter(b => typeof b.title === 'string' && b.title.startsWith(prefix))
    .map(b => ({ ...b, title: b.title.slice(prefix.length) }))
}

// ABS treats two bookmarks on one item at the same second as the same
// bookmark and just renames the first. Every episode starts at 0:00, so an
// episode bookmark there would rename another episode's. Skip those.
export function canBookmarkAt(label, time) {
  return !label || time > 0
}
