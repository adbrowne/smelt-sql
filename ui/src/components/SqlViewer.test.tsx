import { describe, it, expect } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { SqlViewer } from './SqlViewer'

describe('SqlViewer', () => {
  it('renders_readonly_syntax_highlighted_sql', () => {
    const sql = 'SELECT id, name FROM users'
    const { container } = render(<SqlViewer sql={sql} />)

    // CodeMirror renders the given SQL into the document.
    expect(screen.getByText(/SELECT/)).toBeInTheDocument()

    // CodeMirror's read-only editor content is not a contenteditable
    // surface, so no keypress can mutate it — assert the CM6 content div
    // is not marked editable.
    const content = container.querySelector('.cm-content')
    expect(content).not.toBeNull()
    expect(content).toHaveAttribute('contenteditable', 'false')

    // Firing a keypress must not change the displayed text.
    if (content) {
      fireEvent.keyDown(content, { key: 'x' })
    }
    expect(screen.getByText(/SELECT/)).toBeInTheDocument()
  })

  it('folds_comments_by_default_and_can_be_unfolded', async () => {
    const sql = 'SELECT id -- trailing comment\nFROM users'
    const { container } = render(<SqlViewer sql={sql} />)

    await waitFor(() => {
      expect(container.querySelector('.cm-content')?.textContent).toContain('SELECT id')
    })

    // The comment's own text is not rendered while folded away — the
    // original document still has it (unlike the old strip-to-empty
    // toggle), it's just visually collapsed.
    expect(container.querySelector('.cm-content')?.textContent).not.toContain('trailing comment')

    const foldPlaceholder = container.querySelector('.cm-foldPlaceholder')
    expect(foldPlaceholder).not.toBeNull()

    if (foldPlaceholder) {
      fireEvent.click(foldPlaceholder)
    }

    await waitFor(() => {
      expect(container.querySelector('.cm-content')?.textContent).toContain('trailing comment')
    })
  })

  it('groups_consecutive_comment_lines_into_a_single_fold', async () => {
    const sql = '-- header line 1\n-- header line 2\n-- header line 3\nSELECT 1 FROM t'
    const { container } = render(<SqlViewer sql={sql} />)

    await waitFor(() => {
      expect(container.querySelector('.cm-content')?.textContent).toContain('SELECT 1')
    })

    expect(container.querySelector('.cm-content')?.textContent).not.toContain('header line')
    // Three consecutive comment lines fold as one unit, not one fold per line.
    expect(container.querySelectorAll('.cm-foldPlaceholder')).toHaveLength(1)
  })
})
