import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { fetchRuns } from '../api'
import type { RunHistoryEntry } from '../types'

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60000).toFixed(1)}m`
}

function formatTime(iso: string): string {
  const d = new Date(iso)
  return d.toLocaleString()
}

function RunRow({ run }: { run: RunHistoryEntry }) {
  const [expanded, setExpanded] = useState(false)

  const totalRows = Object.values(run.models).reduce((sum, m) => sum + m.row_count, 0)
  const totalDuration = Object.values(run.models).reduce((sum, m) => sum + m.duration_ms, 0)

  return (
    <>
      <tr
        className="border-b border-gray-100 last:border-0 cursor-pointer hover:bg-gray-50"
        onClick={() => setExpanded(!expanded)}
      >
        <td className="px-3 py-2 font-mono text-xs text-gray-700">
          <span className="text-gray-400 mr-1">{expanded ? '▾' : '▸'}</span>
          {run.run_id}
        </td>
        <td className="px-3 py-2 text-sm text-gray-600">
          {formatTime(run.started_at)}
        </td>
        <td className="px-3 py-2 text-sm text-gray-600 text-right">
          {run.model_count}
        </td>
        <td className="px-3 py-2 text-sm text-gray-600 text-right">
          {totalRows.toLocaleString()}
        </td>
        <td className="px-3 py-2 text-sm text-gray-600 text-right">
          {formatDuration(totalDuration)}
        </td>
      </tr>
      {expanded && Object.entries(run.models).map(([name, model]) => (
        <tr key={name} className="bg-gray-50 border-b border-gray-100 text-xs">
          <td className="pl-10 pr-3 py-1 font-mono text-gray-500">{name}</td>
          <td className="px-3 py-1 text-gray-500">
            {model.time_range ? `${model.time_range.start} → ${model.time_range.end}` : '-'}
          </td>
          <td className="px-3 py-1 text-gray-500">
            <span className="px-1.5 py-0.5 bg-gray-100 rounded">{model.strategy}</span>
          </td>
          <td className="px-3 py-1 text-gray-500 text-right">{model.row_count.toLocaleString()}</td>
          <td className="px-3 py-1 text-gray-500 text-right">{formatDuration(model.duration_ms)}</td>
        </tr>
      ))}
    </>
  )
}

export function RunHistory() {
  const { data: runs, isLoading, error } = useQuery({
    queryKey: ['runs'],
    queryFn: fetchRuns,
  })

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-5xl mx-auto p-6">
        <h1 className="text-xl font-semibold text-gray-900 mb-6">Run History</h1>

        {isLoading && (
          <p className="text-gray-500 text-sm">Loading run history...</p>
        )}

        {error && (
          <div className="bg-red-50 border border-red-200 text-red-700 rounded p-3 text-sm">
            {(error as Error).message}
          </div>
        )}

        {runs && runs.length === 0 && (
          <p className="text-gray-500 text-sm">No runs yet. Use the Run Planner to execute your first run.</p>
        )}

        {runs && runs.length > 0 && (
          <div className="border border-gray-200 rounded overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="bg-gray-50 border-b border-gray-200">
                  <th className="text-left px-3 py-2 font-medium text-gray-600">Run ID</th>
                  <th className="text-left px-3 py-2 font-medium text-gray-600">Started</th>
                  <th className="text-right px-3 py-2 font-medium text-gray-600">Models</th>
                  <th className="text-right px-3 py-2 font-medium text-gray-600">Rows</th>
                  <th className="text-right px-3 py-2 font-medium text-gray-600">Duration</th>
                </tr>
              </thead>
              <tbody>
                {runs.map(run => (
                  <RunRow key={run.run_id} run={run} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  )
}
