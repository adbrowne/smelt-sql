import { useMemo } from 'react'
import CodeMirror from '@uiw/react-codemirror'
import { sql as sqlLang } from '@codemirror/lang-sql'
import { EditorView } from '@codemirror/view'
import { stripSqlComments } from '../lib/stripSqlComments'

interface SqlViewerProps {
  sql: string
  removeComments: boolean
}

const extensions = [sqlLang(), EditorView.lineWrapping]

/**
 * Read-only, syntax-highlighted SQL display
 * (`docs/specs/ui_model_diagnostics.md` §Surface "UI page": "SQL … renders
 * in a read-only syntax-highlighted viewer, not a plain preformatted
 * block"). `removeComments` applies the page-wide "Remove comments" toggle
 * to this one viewer's content — see `stripSqlComments` for why the
 * stripping happens here, client-side, rather than in the API response.
 */
export function SqlViewer({ sql, removeComments }: SqlViewerProps) {
  const displayedSql = useMemo(
    () => (removeComments ? stripSqlComments(sql) : sql),
    [sql, removeComments]
  )

  return (
    <div className="rounded border border-gray-200 overflow-hidden text-xs">
      <CodeMirror
        value={displayedSql}
        extensions={extensions}
        readOnly
        editable={false}
        basicSetup={{
          lineNumbers: true,
          foldGutter: false,
          highlightActiveLine: false,
          highlightActiveLineGutter: false,
        }}
      />
    </div>
  )
}
