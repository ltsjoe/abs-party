import { describe, it, expect } from 'vitest'
import {
  sortNewestFirst, formatDuration, formatDate, progressByEpisode, progressLabel, bookChoice, episodeChoice,
} from './episodes.js'

describe('sortNewestFirst', () => {
  it('sorts by publish date, falling back to the date ABS added it', () => {
    const eps = [
      { id: 'old', publishedAt: 1000 },
      { id: 'new', publishedAt: 3000 },
      { id: 'undated', publishedAt: null, addedAt: 2000 },
    ]
    expect(sortNewestFirst(eps).map(e => e.id)).toEqual(['new', 'undated', 'old'])
  })

  it('does not change the list it was given', () => {
    const eps = [{ id: 'a', publishedAt: 1 }, { id: 'b', publishedAt: 2 }]
    sortNewestFirst(eps)
    expect(eps[0].id).toBe('a')
  })
})

describe('formatDuration', () => {
  it('shows minutes, and hours past 60', () => {
    expect(formatDuration(42 * 60)).toBe('42 min')
    expect(formatDuration(65 * 60)).toBe('1 h 05 min')
    expect(formatDuration(20)).toBe('1 min')
  })

  it('is empty when unknown', () => {
    expect(formatDuration(0)).toBe('')
    expect(formatDuration(undefined)).toBe('')
    expect(formatDuration(NaN)).toBe('')
  })
})

describe('formatDate', () => {
  it('is empty when ABS has no date', () => {
    expect(formatDate(null)).toBe('')
  })

  it('formats a timestamp', () => {
    expect(formatDate(Date.UTC(2026, 10, 10))).not.toBe('')
  })
})

describe('progressByEpisode / progressLabel', () => {
  const me = {
    mediaProgress: [
      { libraryItemId: 'li_pod', episodeId: 'ep_1', progress: 1, isFinished: true },
      { libraryItemId: 'li_pod', episodeId: 'ep_2', progress: 0.356, isFinished: false },
      { libraryItemId: 'li_other', episodeId: 'ep_3', progress: 0.5, isFinished: false },
      { libraryItemId: 'li_book', episodeId: null, progress: 0.5, isFinished: false },
    ],
  }

  it("maps one podcast's episodes to their progress", () => {
    expect(Object.keys(progressByEpisode(me, 'li_pod')).sort()).toEqual(['ep_1', 'ep_2'])
    expect(progressByEpisode(null, 'li_pod')).toEqual({})
  })

  it('labels finished, partly played and new episodes', () => {
    const map = progressByEpisode(me, 'li_pod')
    expect(progressLabel(map.ep_1)).toBe('Finished')
    expect(progressLabel(map.ep_2)).toBe('35%')
    expect(progressLabel(undefined)).toBe('')
    expect(progressLabel({ progress: 0, isFinished: false })).toBe('')
  })
})

describe('room choices', () => {
  it('describes a book like the picker always did', () => {
    const item = { id: 'li_b', media: { metadata: { title: 'Dune', authorName: 'Frank Herbert' } } }
    expect(bookChoice(item, 'lib_1')).toEqual({
      itemId: 'li_b', episodeId: null, title: 'Dune', author: 'Frank Herbert', libraryId: 'lib_1',
    })
  })

  it('describes an episode: its title, with the podcast as author', () => {
    const ep = { id: 'ep_7', title: 'Episode 7' }
    expect(episodeChoice('li_pod', { title: 'Some Show', author: 'Host' }, ep, 'lib_pods')).toEqual({
      itemId: 'li_pod', episodeId: 'ep_7', title: 'Episode 7', author: 'Some Show', libraryId: 'lib_pods',
    })
  })
})
