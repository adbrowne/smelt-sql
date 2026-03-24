import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { fetchGraph, fetchRunPlan, executeRun, cancelRun } from '../api'
import { useRunStatus } from '../hooks/useRunStatus'
import { RunProgress } from '../components/RunProgress'
import type { RunPlanResponse, PlanModel } from '../types'

function formatDate(d: Date): string {
  return d.toISOString().slice(0, 10)
}

function PlanPreview({ plan, onExecute, isExecuting }: {
  plan: RunPlanResponse
  onExecute: () => void
  isExecuting: boolean
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-4">
        <span className="text-sm text-gray-600">{plan.execution_order.length} model(s)</span>
        <span className="text-sm text-gray-600">{plan.total_batches} batch(es)</span>
        <button
          onClick={onExecute}
          disabled={isExecuting}
          className="ml-auto bg-green-600 text-white px-4 py-1.5 rounded text-sm font-medium hover:bg-green-700 disabled:opacity-50"
        >
          {isExecuting ? 'Starting...' : 'Execute Run'}
        </button>
      </div>

      <div className="border border-gray-200 rounded overflow-hidden">
        <table className="w-full text-sm">
          <thead>
            <tr className="bg-gray-50 border-b border-gray-200">
              <th className="text-left px-3 py-2 font-medium text-gray-600">Model</th>
              <th className="text-left px-3 py-2 font-medium text-gray-600">Type</th>
              <th className="text-left px-3 py-2 font-medium text-gray-600">Safety</th>
              <th className="text-right px-3 py-2 font-medium text-gray-600">Batches</th>
              <th className="text-left px-3 py-2 font-medium text-gray-600">Range</th>
            </tr>
          </thead>
          <tbody>
            {plan.models.map((model) => (
              <ModelPlanRow key={model.name} model={model} />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

function ModelPlanRow({ model }: { model: PlanModel }) {
  const [expanded, setExpanded] = useState(false)

  const safetyColors: Record<string, string> = {
    fully_batch_safe: 'text-green-700 bg-green-50',
    bounded_safe: 'text-yellow-700 bg-yellow-50',
    per_partition_only: 'text-red-700 bg-red-50',
  }

  const safetyLabels: Record<string, string> = {
    fully_batch_safe: 'Safe',
    bounded_safe: 'Bounded',
    per_partition_only: 'Per-Partition',
  }

  return (
    <>
      <tr
        className="border-b border-gray-100 last:border-0 cursor-pointer hover:bg-gray-50"
        onClick={() => model.batches.length > 0 && setExpanded(!expanded)}
      >
        <td className="px-3 py-2 font-mono text-gray-900">
          {model.batches.length > 0 && (
            <span className="text-gray-400 mr-1">{expanded ? '▾' : '▸'}</span>
          )}
          {model.name}
        </td>
        <td className="px-3 py-2 text-gray-600">
          {model.is_incremental ? 'incremental' : 'full refresh'}
        </td>
        <td className="px-3 py-2">
          {model.batch_safety && (
            <span className={`text-xs px-1.5 py-0.5 rounded ${safetyColors[model.batch_safety.level] ?? ''}`}>
              {safetyLabels[model.batch_safety.level] ?? model.batch_safety.level}
            </span>
          )}
        </td>
        <td className="px-3 py-2 text-right text-gray-600">
          {model.batches.length || '-'}
        </td>
        <td className="px-3 py-2 text-gray-500 text-xs font-mono">
          {model.partition_range
            ? `${model.partition_range.start} → ${model.partition_range.end}`
            : '-'}
        </td>
      </tr>
      {expanded && model.batches.map((batch, i) => (
        <tr key={i} className="bg-gray-50 border-b border-gray-100 text-xs">
          <td className="pl-10 pr-3 py-1 text-gray-500">Batch {i + 1}</td>
          <td colSpan={2} className="px-3 py-1 font-mono text-gray-500">
            write: {batch.partition_start} → {batch.partition_end}
          </td>
          <td colSpan={2} className="px-3 py-1 font-mono text-gray-500">
            read: {batch.filter_start} → {batch.filter_end}
          </td>
        </tr>
      ))}
    </>
  )
}

export function RunPlanner() {
  const today = formatDate(new Date())
  const thirtyDaysAgo = formatDate(new Date(Date.now() - 30 * 24 * 60 * 60 * 1000))

  const [start, setStart] = useState(thirtyDaysAgo)
  const [end, setEnd] = useState(today)
  const [batchSize, setBatchSize] = useState('')
  const [perPartition, setPerPartition] = useState(false)
  const [selectedModels, setSelectedModels] = useState<string[]>([])

  const { data: graphData } = useQuery({
    queryKey: ['graph'],
    queryFn: fetchGraph,
  })

  const { status, startRun } = useRunStatus()

  const planMutation = useMutation({
    mutationFn: fetchRunPlan,
  })

  const executeMutation = useMutation({
    mutationFn: executeRun,
    onSuccess: (data) => {
      startRun(data.run_id)
    },
  })

  const allModels = graphData?.nodes
    .filter(n => n.node_type === 'model')
    .map(n => n.id)
    .sort() ?? []

  function handlePreview() {
    planMutation.mutate({
      start,
      end,
      batch_size_days: batchSize ? parseInt(batchSize) : undefined,
      per_partition: perPartition,
      select: selectedModels.length > 0 ? selectedModels : undefined,
    })
  }

  function handleExecute() {
    executeMutation.mutate({
      start,
      end,
      batch_size_days: batchSize ? parseInt(batchSize) : undefined,
      per_partition: perPartition,
      select: selectedModels.length > 0 ? selectedModels : undefined,
    })
  }

  async function handleCancel() {
    try {
      await cancelRun()
    } catch {
      // Ignore cancel errors
    }
  }

  const isRunning = status.state === 'running'

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-4xl mx-auto p-6">
        <h1 className="text-xl font-semibold text-gray-900 mb-6">Run Planner</h1>

        {isRunning && <RunProgress status={status} onCancel={handleCancel} />}

        {status.error && (
          <div className="bg-red-50 border border-red-200 text-red-700 rounded p-3 mb-6 text-sm">
            Run failed: {status.error}
          </div>
        )}

        <div className="bg-white rounded-lg border border-gray-200 p-4 mb-6">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">Start Date</label>
              <input
                type="date"
                value={start}
                onChange={e => setStart(e.target.value)}
                disabled={isRunning}
                className="w-full border border-gray-300 rounded px-3 py-1.5 text-sm disabled:opacity-50"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">End Date</label>
              <input
                type="date"
                value={end}
                onChange={e => setEnd(e.target.value)}
                disabled={isRunning}
                className="w-full border border-gray-300 rounded px-3 py-1.5 text-sm disabled:opacity-50"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Batch Size (days)
                <span className="text-gray-400 font-normal ml-1">optional</span>
              </label>
              <input
                type="number"
                value={batchSize}
                onChange={e => setBatchSize(e.target.value)}
                placeholder="auto"
                min="1"
                disabled={isRunning}
                className="w-full border border-gray-300 rounded px-3 py-1.5 text-sm disabled:opacity-50"
              />
            </div>
            <div className="flex items-end">
              <label className="flex items-center gap-2 text-sm text-gray-700 pb-1.5">
                <input
                  type="checkbox"
                  checked={perPartition}
                  onChange={e => setPerPartition(e.target.checked)}
                  disabled={isRunning}
                  className="rounded border-gray-300"
                />
                Per-partition execution
              </label>
            </div>
          </div>

          {allModels.length > 0 && (
            <div className="mb-4">
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Models
                <span className="text-gray-400 font-normal ml-1">
                  {selectedModels.length === 0 ? 'all' : `${selectedModels.length} selected`}
                </span>
              </label>
              <div className="flex gap-1.5 flex-wrap">
                {allModels.map(name => (
                  <button
                    key={name}
                    onClick={() => {
                      setSelectedModels(prev =>
                        prev.includes(name)
                          ? prev.filter(n => n !== name)
                          : [...prev, name]
                      )
                    }}
                    disabled={isRunning}
                    className={`text-xs px-2 py-0.5 rounded border transition-colors disabled:opacity-50 ${
                      selectedModels.includes(name)
                        ? 'bg-blue-50 border-blue-300 text-blue-700'
                        : 'bg-white border-gray-200 text-gray-600 hover:border-gray-300'
                    }`}
                  >
                    {name}
                  </button>
                ))}
              </div>
            </div>
          )}

          <button
            onClick={handlePreview}
            disabled={planMutation.isPending || isRunning}
            className="bg-blue-600 text-white px-4 py-1.5 rounded text-sm font-medium hover:bg-blue-700 disabled:opacity-50"
          >
            {planMutation.isPending ? 'Computing...' : 'Preview Plan'}
          </button>
        </div>

        {planMutation.error && (
          <div className="bg-red-50 border border-red-200 text-red-700 rounded p-3 mb-6 text-sm">
            {(planMutation.error as Error).message}
          </div>
        )}

        {executeMutation.error && (
          <div className="bg-red-50 border border-red-200 text-red-700 rounded p-3 mb-6 text-sm">
            {(executeMutation.error as Error).message}
          </div>
        )}

        {planMutation.data && (
          <PlanPreview
            plan={planMutation.data}
            onExecute={handleExecute}
            isExecuting={executeMutation.isPending || isRunning}
          />
        )}
      </div>
    </div>
  )
}
