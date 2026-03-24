import type { RunStatus } from '../hooks/useRunStatus'

export function RunProgress({ status, onCancel }: { status: RunStatus; onCancel: () => void }) {
  if (status.state !== 'running') return null

  const modelPercent = status.modelsTotal > 0
    ? Math.round((status.modelsCompleted / status.modelsTotal) * 100)
    : 0
  const batchPercent = status.batchesTotal > 0
    ? Math.round((status.batchesCompleted / status.batchesTotal) * 100)
    : 0

  return (
    <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mb-6">
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <span className="inline-block w-2 h-2 rounded-full bg-blue-500 animate-pulse" />
          <span className="text-sm font-medium text-blue-900">
            Running{status.runId ? ` (${status.runId})` : ''}
          </span>
        </div>
        <button
          onClick={onCancel}
          className="text-xs text-red-600 hover:text-red-800 px-2 py-1 rounded hover:bg-red-50"
        >
          Cancel
        </button>
      </div>

      {status.currentModel && (
        <p className="text-sm text-blue-800 mb-2">
          Current: <span className="font-mono">{status.currentModel}</span>
        </p>
      )}

      <div className="space-y-2">
        <div>
          <div className="flex justify-between text-xs text-blue-700 mb-1">
            <span>Models</span>
            <span>{status.modelsCompleted}/{status.modelsTotal}</span>
          </div>
          <div className="w-full bg-blue-100 rounded-full h-2">
            <div
              className="bg-blue-500 h-2 rounded-full transition-all duration-300"
              style={{ width: `${modelPercent}%` }}
            />
          </div>
        </div>

        {status.batchesTotal > 0 && (
          <div>
            <div className="flex justify-between text-xs text-blue-700 mb-1">
              <span>Batches</span>
              <span>{status.batchesCompleted}/{status.batchesTotal}</span>
            </div>
            <div className="w-full bg-blue-100 rounded-full h-2">
              <div
                className="bg-blue-500 h-2 rounded-full transition-all duration-300"
                style={{ width: `${batchPercent}%` }}
              />
            </div>
          </div>
        )}
      </div>

      {/* Recent events log */}
      {status.events.length > 0 && (
        <div className="mt-3 max-h-32 overflow-y-auto">
          {status.events.slice(-8).map((event, i) => (
            <EventLine key={i} event={event} />
          ))}
        </div>
      )}
    </div>
  )
}

function EventLine({ event }: { event: import('../types').RunProgressEvent }) {
  switch (event.type) {
    case 'model_started':
      return <p className="text-xs text-blue-600">Starting {event.model}...</p>
    case 'batch_completed':
      return (
        <p className="text-xs text-blue-600">
          {event.model} batch {event.batch_index + 1}/{event.batches_total} ({event.row_count} rows, {event.duration_ms}ms)
        </p>
      )
    case 'model_completed':
      return (
        <p className="text-xs text-green-600">
          {event.model} completed ({event.row_count} rows, {event.duration_ms}ms)
        </p>
      )
    case 'run_failed':
      return <p className="text-xs text-red-600">Failed: {event.error}</p>
    default:
      return null
  }
}
