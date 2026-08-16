import { useCallback } from 'react'
import CodeMirror from '@uiw/react-codemirror'
import { sql as sqlLang } from '@codemirror/lang-sql'
import { EditorView } from '@codemirror/view'
import { foldEffect } from '@codemirror/language'
import { findSqlCommentRanges } from '../lib/sqlCommentRanges'

interface SqlViewerProps {
  sql: string
}

const extensions = [sqlLang(), EditorView.lineWrapping]

/**
 * Read-only, syntax-highlighted SQL display
 * (`docs/specs/ui_model_diagnostics.md` §Surface "UI page": "SQL … renders
 * in a read-only syntax-highlighted viewer, not a plain preformatted
 * block"). `--`/`/* … *\/` comment spans (`findSqlCommentRanges`) are
 * folded by default on load — the original text is untouched, so a
 * comment is one click on the gutter's unfold marker away, and there is
 * no risk of the leftover-blank-line artifact a text-deletion toggle
 * would leave behind. See §Semantics "Comment folding" and §Design "Why
 * comment folding, not comment stripping".
 */
export function SqlViewer({ sql }: SqlViewerProps) {
  const handleCreateEditor = useCallback(
    (view: EditorView) => {
      const ranges = findSqlCommentRanges(sql)
      if (ranges.length === 0) return
      view.dispatch({
        effects: ranges.map((range) => foldEffect.of(range)),
      })
    },
    [sql]
  )

  return (
    <div className="rounded border border-gray-200 overflow-hidden text-xs">
      <CodeMirror
        value={sql}
        extensions={extensions}
        readOnly
        editable={false}
        onCreateEditor={handleCreateEditor}
        basicSetup={{
          lineNumbers: true,
          foldGutter: true,
          highlightActiveLine: false,
          highlightActiveLineGutter: false,
        }}
      />
    </div>
  )
}
