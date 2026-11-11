import { describe, it, expect } from 'vitest'
import { episodeLabel, episodeBookmarkTitle, bookmarksForRoom, canBookmarkAt } from './bookmarks.js'

describe('episodeLabel', () => {
  it('is empty for a book room', () => {
    expect(episodeLabel({ id: 'li_1', title: 'Book', episode_id: null })).toBe('')
    expect(episodeLabel(null)).toBe('')
  })

  it('is the episode title in a podcast room, else its id', () => {
    expect(episodeLabel({ id: 'li_1', title: 'Ep 7', episode_id: 'ep_7' })).toBe('Ep 7')
    expect(episodeLabel({ id: 'li_1', title: '', episode_id: 'ep_7' })).toBe('ep_7')
  })
})

describe('episodeBookmarkTitle', () => {
  it('leaves book bookmarks as they are', () => {
    expect(episodeBookmarkTitle('pre_party_x', '')).toBe('pre_party_x')
  })

  it('puts the episode in front for a podcast', () => {
    expect(episodeBookmarkTitle('pre_party_x', 'Ep 7')).toBe('[Ep 7] pre_party_x')
  })
})

describe('bookmarksForRoom', () => {
  const all = [
    { libraryItemId: 'li_1', time: 10, title: '[Ep 7] good bit' },
    { libraryItemId: 'li_1', time: 20, title: '[Ep 8] other episode' },
    { libraryItemId: 'li_1', time: 30, title: 'no episode' },
    { libraryItemId: 'li_1', time: 40, title: '[Ep 7 extended] not this one' },
  ]

  it('keeps everything in a book room', () => {
    expect(bookmarksForRoom(all, '')).toHaveLength(4)
  })

  it("keeps only this episode's, with the prefix removed", () => {
    expect(bookmarksForRoom(all, 'Ep 7')).toEqual([
      { libraryItemId: 'li_1', time: 10, title: 'good bit' },
    ])
  })

  it('copes with titles ABS left empty', () => {
    expect(bookmarksForRoom([{ time: 1 }], 'Ep 7')).toEqual([])
  })
})

describe('canBookmarkAt', () => {
  it('always lets a book bookmark', () => {
    expect(canBookmarkAt('', 0)).toBe(true)
  })

  it('refuses an episode bookmark at 0:00 only', () => {
    expect(canBookmarkAt('Ep 7', 0)).toBe(false)
    expect(canBookmarkAt('Ep 7', 0.5)).toBe(true)
  })
})
