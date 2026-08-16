import type { ModelDetailResponse, BatchSafetyInfo, ColumnInfo } from '../types'

interface ModelDetailProps {
  model: ModelDetailResponse
  onClose: () => void
  onOpenDiagnostics?: (name: string) => void
}

/**
 * The columns table shared by `ModelDetail`'s side panel and the
 * `ModelDiagnostics` full-screen page's overview section (`docs/specs/
 * ui_model_diagnostics.md` §Surface "UI page": the page's overview is "a
 * superset of the existing side panel's fields").
 */
export function ColumnsTable({ columns }: { columns: ColumnInfo[] }) {
  return (
    <div className="border border-gray-200 rounded overflow-hidden">
      <table className="w-full text-xs">
        <thead>
          <tr className="bg-gray-50 border-b border-gray-200">
            <th className="text-left px-2 py-1 font-medium text-gray-600">Name</th>
            <th className="text-left px-2 py-1 font-medium text-gray-600">Type</th>
            <th className="text-center px-1 py-1 font-medium text-gray-600" title="Nullable">?</th>
            <th className="text-left px-2 py-1 font-medium text-gray-600">Source</th>
          </tr>
        </thead>
        <tbody>
          {columns.map((col) => (
            <tr key={col.name} className="border-b border-gray-100 last:border-0">
              <td className="px-2 py-1 font-mono text-gray-900">{col.name}</td>
              <td className="px-2 py-1 text-gray-600">
                {col.data_type ?? <span className="text-gray-400">?</span>}
              </td>
              <td className="px-1 py-1 text-center text-gray-400">
                {col.nullable === true && <span title="nullable">∅</span>}
                {col.nullable === false && <span className="text-gray-600" title="not null">!</span>}
              </td>
              <td className="px-2 py-1 text-gray-500">
                {col.source.type === 'from_model' && (
                  <span title={`${col.source.model}.${col.source.column}`}>
                    {col.source.model}
                  </span>
                )}
                {col.source.type === 'computed' && 'computed'}
                {col.source.type === 'external_table' && col.source.table}
                {col.source.type === 'unknown' && '-'}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function BatchSafetyBadge({ safety }: { safety: BatchSafetyInfo }) {
  const colors = {
    fully_batch_safe: 'bg-green-100 text-green-700',
    bounded_safe: 'bg-yellow-100 text-yellow-700',
    per_partition_only: 'bg-red-100 text-red-700',
  }
  const labels = {
    fully_batch_safe: 'Batch Safe',
    bounded_safe: `Bounded (${safety.max_chunk_days}d chunks, ${safety.context_days}d context)`,
    per_partition_only: 'Per-Partition Only',
  }
  const colorClass = colors[safety.level as keyof typeof colors] ?? 'bg-gray-100 text-gray-600'
  const label = labels[safety.level as keyof typeof labels] ?? safety.level

  return (
    <span className={`text-xs px-2 py-0.5 rounded font-medium ${colorClass}`} title={safety.reason ?? undefined}>
      {label}
    </span>
  )
}

export function ModelDetail({ model, onClose, onOpenDiagnostics }: ModelDetailProps) {
  const diagnostics = model.diagnostics ?? []
  const errors = diagnostics.filter(d => d.severity === 'error')
  const warnings = diagnostics.filter(d => d.severity === 'warning')

  return (
    <div className="w-96 bg-white border-l border-gray-200 overflow-y-auto shrink-0">
      <div className="p-4">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-base font-semibold text-gray-900">{model.name}</h2>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-gray-600 text-lg leading-none"
          >
            &times;
          </button>
        </div>

        <p className="text-xs text-gray-500 mb-3 font-mono">{model.path}</p>

        {onOpenDiagnostics && (
          <button
            onClick={() => onOpenDiagnostics(model.name)}
            className="text-xs bg-gray-900 text-white px-2 py-1 rounded font-medium hover:bg-gray-700 mb-3"
          >
            Open Diagnostics
          </button>
        )}

        {model.description && (
          <p className="text-sm text-gray-700 mb-3">{model.description}</p>
        )}

        <div className="flex gap-2 mb-3 flex-wrap">
          {model.materialization && (
            <span className="text-xs bg-amber-100 text-amber-700 px-2 py-0.5 rounded font-medium">
              {model.materialization}
            </span>
          )}
          {model.batch_safety && <BatchSafetyBadge safety={model.batch_safety} />}
          {model.tags.map((tag) => (
            <span key={tag} className="text-xs bg-gray-100 text-gray-600 px-2 py-0.5 rounded">
              {tag}
            </span>
          ))}
        </div>

        {model.owner && (
          <p className="text-xs text-gray-500 mb-3">Owner: {model.owner}</p>
        )}

        {/* Diagnostics */}
        {diagnostics.length > 0 && (
          <div className="mb-4">
            <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-1">
              Diagnostics
            </h3>
            <div className="space-y-1">
              {errors.map((d, i) => (
                <div key={`e${i}`} className="text-xs bg-red-50 text-red-700 px-2 py-1 rounded">
                  {d.line != null && <span className="font-mono text-red-500">L{d.line} </span>}
                  {d.message}
                </div>
              ))}
              {warnings.map((d, i) => (
                <div key={`w${i}`} className="text-xs bg-yellow-50 text-yellow-700 px-2 py-1 rounded">
                  {d.line != null && <span className="font-mono text-yellow-500">L{d.line} </span>}
                  {d.message}
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Incremental Config */}
        {model.incremental && (
          <div className="mb-4">
            <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-1">
              Incremental
            </h3>
            <div className="text-xs space-y-1 bg-gray-50 rounded p-2">
              <div className="flex justify-between">
                <span className="text-gray-500">Granularity</span>
                <span className="font-medium text-gray-700">{model.incremental.granularity}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500">Partition</span>
                <span className="font-mono text-gray-700">{model.incremental.partition_column}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500">Event Time</span>
                <span className="font-mono text-gray-700">{model.incremental.event_time_column}</span>
              </div>
              {model.incremental.unique_key.length > 0 && (
                <div className="flex justify-between">
                  <span className="text-gray-500">Unique Key</span>
                  <span className="font-mono text-gray-700">{model.incremental.unique_key.join(', ')}</span>
                </div>
              )}
            </div>
          </div>
        )}

        {model.refs.length > 0 && (
          <div className="mb-4">
            <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-1">
              Dependencies
            </h3>
            <div className="flex gap-1 flex-wrap">
              {model.refs.map((dep) => (
                <span key={dep} className="text-xs bg-blue-50 text-blue-700 px-2 py-0.5 rounded">
                  {dep}
                </span>
              ))}
            </div>
          </div>
        )}

        {model.columns.length > 0 && (
          <div className="mb-4">
            <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">
              Columns ({model.columns.length})
            </h3>
            <ColumnsTable columns={model.columns} />
          </div>
        )}

        {model.function_type && (
          <div className="mb-4">
            <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">
              Model Type
            </h3>
            <pre className="bg-gray-50 text-gray-700 rounded p-3 text-xs overflow-x-auto whitespace-pre-wrap font-mono border border-gray-200">
              {model.function_type}
            </pre>
          </div>
        )}

        <div>
          <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">
            SQL
          </h3>
          <pre className="bg-gray-900 text-gray-100 rounded p-3 text-xs overflow-x-auto whitespace-pre-wrap font-mono">
            {model.sql}
          </pre>
        </div>
      </div>
    </div>
  )
}
