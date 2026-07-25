import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { ModelDiagnostics } from './ModelDiagnostics'
import * as api from '../api'
import type { ModelDetailResponse, ModelDiagnosticsResponse } from '../types'

const modelDetail: ModelDetailResponse = {
  name: 'daily_events',
  path: 'models/daily_events.sql',
  sql: 'SELECT id FROM events -- model comment',
  materialization: 'incremental',
  tags: [],
  owner: null,
  description: null,
  refs: [],
  columns: [],
}

const diagnostics: ModelDiagnosticsResponse = {
  model: 'daily_events',
  properties: {
    columns: [],
    grain: { keys: [] },
    functional_dependencies: [],
    determinism: [],
    comparability: [],
    discriminants: [],
    literal_columns: [],
    has_set_op_barrier: false,
    has_fan_out_join: false,
    row_identity: { identity: 'WholeRow', proven_mismatch: null },
    source_bounds: {},
  },
  contract: {},
  inbound_edges: [],
  cells: [
    {
      group: 'main',
      trigger: 'Always',
      corner: 'None',
      admitted_technique: 'DeleteInsert',
      row_identity: 'WholeRow',
      technique_previews: [
        {
          technique: 'DeleteInsert',
          transactional: true,
          statements: [{ sql: 'DELETE FROM t WHERE 1=1 -- delete comment' }],
          admissibility: { verdict: 'admitted' },
        },
        {
          technique: 'KeyedFold',
          transactional: false,
          statements: [{ sql: 'MERGE INTO t -- merge comment' }],
          admissibility: { verdict: 'not_applicable', reason: 'no unique key declared' },
        },
      ],
    },
  ],
}

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={queryClient}>
      <ModelDiagnostics model="daily_events" onClose={() => {}} />
    </QueryClientProvider>
  )
}

/** CodeMirror's syntax highlighter splits SQL text across multiple `<span>`
 * text nodes, so `getByText` regex matching against a single node doesn't
 * work — read each `.cm-content` viewer's flattened text content instead.
 */
function viewerTexts(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll('.cm-content')).map((el) => el.textContent ?? '')
}

describe('ModelDiagnostics', () => {
  beforeEach(() => {
    vi.spyOn(api, 'fetchModel').mockResolvedValue(modelDetail)
    vi.spyOn(api, 'fetchModelDiagnostics').mockResolvedValue(diagnostics)
  })

  it('technique_picker_swaps_sql_and_badge', async () => {
    const { container } = renderPage()

    await waitFor(() => {
      expect(viewerTexts(container).some((t) => t.includes('DELETE FROM t'))).toBe(true)
    })
    expect(screen.getByText('Admitted')).toBeInTheDocument()

    const keyedFoldTab = await screen.findByRole('tab', { name: /Keyed Fold/ })
    fireEvent.click(keyedFoldTab)

    await waitFor(() => {
      expect(viewerTexts(container).some((t) => t.includes('MERGE INTO t'))).toBe(true)
    })
    expect(screen.getByText(/Not Applicable/)).toBeInTheDocument()
    expect(viewerTexts(container).some((t) => t.includes('DELETE FROM t'))).toBe(false)
  })

  it('remove_comments_toggle_applies_to_every_viewer', async () => {
    const { container } = renderPage()

    await waitFor(() => {
      expect(viewerTexts(container).some((t) => t.includes('DELETE FROM t'))).toBe(true)
    })
    // Model SQL viewer's comment is present before toggling.
    expect(viewerTexts(container).some((t) => t.includes('model comment'))).toBe(true)
    // Technique-preview viewer's comment is present before toggling.
    expect(viewerTexts(container).some((t) => t.includes('delete comment'))).toBe(true)

    const toggle = screen.getByLabelText('Remove comments')
    fireEvent.click(toggle)

    await waitFor(() => {
      expect(viewerTexts(container).some((t) => t.includes('model comment'))).toBe(false)
    })
    expect(viewerTexts(container).some((t) => t.includes('delete comment'))).toBe(false)
    // Non-comment SQL text remains in every viewer.
    expect(viewerTexts(container).some((t) => t.includes('SELECT id FROM events'))).toBe(true)
    expect(viewerTexts(container).some((t) => t.includes('DELETE FROM t WHERE 1=1'))).toBe(true)
  })
})
