import { describe, it, expect } from 'vitest'
import { normalizeChapters, chapterIndexAt, prevChapterStart, nextChapterStart } from './chapters.js'

const CH = normalizeChapters([
  { id: 0, start: 0, end: 300, title: 'Opening' },
  { id: 1, start: 300, end: 650, title: 'The Middle' },
  { id: 2, start: 650, end: 1000, title: 'The End' },
])

describe('normalizeChapters', () => {
  it('sorts, drops broken entries and fills in missing titles', () => {
    const out = normalizeChapters([
      { id: 2, start: 650, end: 1000, title: '' },
      { id: 0, start: 0, end: 300, title: ' Opening ' },
      { id: 9, start: 'nope', end: 1 },
      null,
      { id: 1, start: '300', end: 650 },
    ])
    expect(out.map(c => [c.start, c.title])).toEqual([[0, 'Opening'], [300, 'Chapter 2'], [650, 'Chapter 3']])
  })

  it('gives an empty list for anything that is not a list', () => {
    expect(normalizeChapters(undefined)).toEqual([])
    expect(normalizeChapters({})).toEqual([])
  })
})

describe('chapterIndexAt', () => {
  it('finds the chapter playing', () => {
    expect(chapterIndexAt(CH, 0)).toBe(0)
    expect(chapterIndexAt(CH, 299)).toBe(0)
    expect(chapterIndexAt(CH, 300)).toBe(1)
    expect(chapterIndexAt(CH, 999)).toBe(2)
  })

  it('counts a time a hair before a start as that chapter (after a seek)', () => {
    expect(chapterIndexAt(CH, 299.8)).toBe(1)
  })

  it('is -1 with no chapters', () => {
    expect(chapterIndexAt([], 10)).toBe(-1)
  })
})

describe('prevChapterStart', () => {
  it('goes back to the start of this chapter when well into it', () => {
    expect(prevChapterStart(CH, 400)).toBe(300)
  })

  it('goes to the chapter before when just past a start', () => {
    expect(prevChapterStart(CH, 302)).toBe(0)
    expect(prevChapterStart(CH, 299.9)).toBe(0)
  })

  it('stays at the first chapter', () => {
    expect(prevChapterStart(CH, 1)).toBe(0)
    expect(prevChapterStart(CH, 100)).toBe(0)
  })
})

describe('nextChapterStart', () => {
  it('goes to the next chapter', () => {
    expect(nextChapterStart(CH, 0)).toBe(300)
    expect(nextChapterStart(CH, 400)).toBe(650)
  })

  it('is null in the last chapter', () => {
    expect(nextChapterStart(CH, 700)).toBeNull()
  })
})
