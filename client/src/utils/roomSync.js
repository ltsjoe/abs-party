// Small rules Room.vue uses to stay in step when connections drop and come
// back. The server holds a dropped seat ("reconnecting…") and gives it back
// when the same person rejoins; these decide what the page does then.

/** How many OTHER people are connected right now (away seats don't count). */
export function othersHere(participants, myName) {
  return participants.filter((p) => p.name !== myName && !p.away).length
}

/**
 * Should this page set the room's position (lead) or take the room's (follow)?
 * Whoever stayed connected is the truth, so with anyone else here, follow.
 * Alone after a reconnect, this page is the truth (it may have kept playing in
 * the background), so lead. On first open only the host leads, starting from
 * its own Audiobookshelf progress; a guest alone in a room takes the room's.
 */
export function shouldLead({ firstOpen, isHostMode, others }) {
  if (others > 0) return false
  return firstOpen ? isHostMode : true
}

/** The participant list after a joined / left / away / back message. */
export function applyParticipantEvent(list, msg) {
  switch (msg.type) {
    case 'participant_joined':
      return [...list.filter((p) => p.name !== msg.name), { name: msg.name, is_host: false, away: false }]
    case 'participant_left':
      return list.filter((p) => p.name !== msg.name)
    case 'participant_away':
      return list.map((p) => (p.name === msg.name ? { ...p, away: true } : p))
    case 'participant_back':
      return list.map((p) => (p.name === msg.name ? { ...p, away: false } : p))
    default:
      return list
  }
}
