import { describe, it, expect } from 'vitest'
import { othersHere, shouldLead, applyParticipantEvent } from './roomSync.js'

describe('othersHere', () => {
  it('counts other people who are connected', () => {
    const list = [
      { name: 'joe', is_host: true, away: false },
      { name: 'rebecca', is_host: false, away: true },
      { name: 'sam', is_host: false, away: false },
    ]
    expect(othersHere(list, 'joe')).toBe(1)
    expect(othersHere(list, 'sam')).toBe(1)
    expect(othersHere([{ name: 'joe', away: false }], 'joe')).toBe(0)
  })
})

describe('shouldLead', () => {
  it('follows whenever someone else stayed connected', () => {
    expect(shouldLead({ firstOpen: true, isHostMode: true, others: 1 })).toBe(false)
    expect(shouldLead({ firstOpen: false, isHostMode: true, others: 2 })).toBe(false)
    expect(shouldLead({ firstOpen: false, isHostMode: false, others: 1 })).toBe(false)
  })

  it('lets only the host lead on first open of an empty room', () => {
    expect(shouldLead({ firstOpen: true, isHostMode: true, others: 0 })).toBe(true)
    expect(shouldLead({ firstOpen: true, isHostMode: false, others: 0 })).toBe(false)
  })

  it('leads after a reconnect with nobody else here, host or not', () => {
    expect(shouldLead({ firstOpen: false, isHostMode: true, others: 0 })).toBe(true)
    expect(shouldLead({ firstOpen: false, isHostMode: false, others: 0 })).toBe(true)
  })
})

describe('applyParticipantEvent', () => {
  const start = [
    { name: 'joe', is_host: true, away: false },
    { name: 'rebecca', is_host: false, away: false },
  ]

  it('marks someone away and back without moving them', () => {
    const away = applyParticipantEvent(start, { type: 'participant_away', name: 'joe' })
    expect(away).toEqual([
      { name: 'joe', is_host: true, away: true },
      { name: 'rebecca', is_host: false, away: false },
    ])
    expect(applyParticipantEvent(away, { type: 'participant_back', name: 'joe' })).toEqual(start)
  })

  it('adds and removes people, never listing a name twice', () => {
    const joined = applyParticipantEvent(start, { type: 'participant_joined', name: 'rebecca' })
    expect(joined.filter((p) => p.name === 'rebecca')).toHaveLength(1)
    const left = applyParticipantEvent(start, { type: 'participant_left', name: 'rebecca' })
    expect(left.map((p) => p.name)).toEqual(['joe'])
  })

  it('leaves the list alone for other messages', () => {
    expect(applyParticipantEvent(start, { type: 'pong' })).toBe(start)
  })
})
