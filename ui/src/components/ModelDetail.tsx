import type { ModelDetailResponse } from '../types'

interface ModelDetailProps {
  model: ModelDetailResponse
  onClose: () => void
}

export function ModelDetail({ model, onClose }: ModelDetailProps) {
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

        {model.description && (
          <p className="text-sm text-gray-700 mb-3">{model.description}</p>
        )}

        <div className="flex gap-2 mb-3 flex-wrap">
          {model.materialization && (
            <span className="text-xs bg-amber-100 text-amber-700 px-2 py-0.5 rounded font-medium">
              {model.materialization}
            </span>
          )}
          {model.tags.map((tag) => (
            <span key={tag} className="text-xs bg-gray-100 text-gray-600 px-2 py-0.5 rounded">
              {tag}
            </span>
          ))}
        </div>

        {model.owner && (
          <p className="text-xs text-gray-500 mb-3">Owner: {model.owner}</p>
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
            <div className="border border-gray-200 rounded overflow-hidden">
              <table className="w-full text-xs">
                <thead>
                  <tr className="bg-gray-50 border-b border-gray-200">
                    <th className="text-left px-2 py-1 font-medium text-gray-600">Name</th>
                    <th className="text-left px-2 py-1 font-medium text-gray-600">Type</th>
                    <th className="text-left px-2 py-1 font-medium text-gray-600">Source</th>
                  </tr>
                </thead>
                <tbody>
                  {model.columns.map((col) => (
                    <tr key={col.name} className="border-b border-gray-100 last:border-0">
                      <td className="px-2 py-1 font-mono text-gray-900">{col.name}</td>
                      <td className="px-2 py-1 text-gray-600">
                        {col.data_type ?? <span className="text-gray-400">?</span>}
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
