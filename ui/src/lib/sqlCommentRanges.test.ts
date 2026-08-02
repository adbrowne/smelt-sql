import { describe, it, expect } from 'vitest'
import { findSqlCommentRanges } from './sqlCommentRanges'

function slices(sql: string) {
  return findSqlCommentRanges(sql).map((r) => sql.slice(r.from, r.to))
}

describe('findSqlCommentRanges', () => {
  it('single_line_comment', () => {
    const sql = 'SELECT 1 -- trailing\nFROM t'
    expect(slices(sql)).toEqual(['-- trailing'])
  })

  it('groups_consecutive_line_comments_into_one_range', () => {
    const sql = '-- header line 1\n-- header line 2\n-- header line 3\nSELECT 1 FROM t'
    const ranges = findSqlCommentRanges(sql)
    expect(ranges).toHaveLength(1)
    expect(sql.slice(ranges[0].from, ranges[0].to)).toBe(
      '-- header line 1\n-- header line 2\n-- header line 3'
    )
  })

  it('does_not_group_comments_separated_by_sql', () => {
    const sql = '-- header\nSELECT 1 -- trailing\nFROM t'
    expect(slices(sql)).toEqual(['-- header', '-- trailing'])
  })

  it('groups_block_comment_followed_by_line_comments', () => {
    const sql = '/* header\n   block */\n-- and a note\nSELECT 1 FROM t'
    const ranges = findSqlCommentRanges(sql)
    expect(ranges).toHaveLength(1)
    expect(sql.slice(ranges[0].from, ranges[0].to)).toBe(
      '/* header\n   block */\n-- and a note'
    )
  })

  it('no_comments_returns_empty', () => {
    expect(findSqlCommentRanges('SELECT 1 FROM t')).toEqual([])
  })
})
