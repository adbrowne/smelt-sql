import { describe, it, expect } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { SqlViewer } from './SqlViewer'

describe('SqlViewer', () => {
  it('renders_readonly_syntax_highlighted_sql', () => {
    const sql = 'SELECT id, name FROM users'
    const { container } = render(<SqlViewer sql={sql} removeComments={false} />)

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
})
