import { useState, useMemo } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { fetchGraph, fetchRunPlan, executeRun, cancelRun, resolveSelectors } from '../api'
import { useRunStatus } from '../hooks/useRunStatus'
import { RunProgress } from '../components/RunProgress'
import type { RunPlanResponse, PlanModel } from '../types'

function formatDate(d: Date): string {
  return d.toISOString().slice(0, 10)
}

/** Parse a space-separated selector string into tokens. */
function parseTokens(text: string): string[] {
  return text.trim().split(/\s+/).filter(Boolean)
}

/** Add a token to a space-separated string (if not already present). */
function addToken(text: string, token: string): string {
  const tokens = parseTokens(text)
  if (tokens.includes(token)) return text
  return [...tokens, token].join(' ')
}

/** Remove a token from a space-separated string. */
function removeToken(text: string, token: string): string {
  return parseTokens(text).filter(t => t !== token).join(' ')
}

function PlanPreview({ plan, onExecute, isExecuting }: {
  plan: RunPlanResponse
  onExecute: () => void
  isExecuting: boolean
}) {
  const [copied, setCopied] = useState(false)

  function handleCopy() {
    navigator.clipboard.writeText(plan.cli_command).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    })
  }

  return (
    <div className="space-y-4">
      {/* CLI Command */}
      <div className="bg-gray-900 rounded-lg p-3 font-mono text-sm text-gray-100 flex items-start gap-2">
        <code className="flex-1 break-all">{plan.cli_command}</code>
        <button
          onClick={handleCopy}
          className="shrink-0 text-gray-400 hover:text-white px-2 py-0.5 rounded text-xs border border-gray-700 hover:border-gray-500 transition-colors"
        >
          {copied ? 'Copied!' : 'Copy'}
        </button>
      </div>

      {/* Resolved models */}
      <div>
        <span className="text-xs font-medium text-gray-500 uppercase tracking-wide">
          Resolved models ({plan.execution_order.length})
        </span>
        <div className="flex gap-1.5 flex-wrap mt-1">
          {plan.execution_order.map(name => (
            <span
              key={name}
              className="text-xs px-2 py-0.5 rounded bg-blue-50 border border-blue-200 text-blue-700 font-mono"
            >
              {name}
            </span>
          ))}
        </div>
      </div>

      {/* Summary + Execute */}
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

function SelectorInput({ label, value, onChange, placeholder, disabled }: {
  label: string
  value: string
  onChange: (v: string) => void
  placeholder: string
  disabled: boolean
}) {
  return (
    <div>
      <label className="block text-sm font-medium text-gray-700 mb-1">{label}</label>
      <input
        type="text"
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        className="w-full border border-gray-300 rounded px-3 py-1.5 text-sm font-mono disabled:opacity-50 placeholder:text-gray-400"
      />
      <p className="text-xs text-gray-400 mt-0.5">
        Space-separated. Supports: model_name, tag:X, +model (upstream), model+ (downstream)
      </p>
    </div>
  )
}

function ModelPills({ models, selectedModels, excludedModels, selectText, excludeText, onSelectChange, onExcludeChange, disabled }: {
  models: string[]
  selectedModels: Set<string>
  excludedModels: Set<string>
  selectText: string
  excludeText: string
  onSelectChange: (v: string) => void
  onExcludeChange: (v: string) => void
  disabled: boolean
}) {
  return (
    <div>
      <label className="block text-sm font-medium text-gray-700 mb-1">
        Models
        <span className="text-gray-400 font-normal ml-1">click to select, shift+click to exclude</span>
      </label>
      <div className="flex gap-1.5 flex-wrap">
        {models.map(name => {
          const isExcluded = excludedModels.has(name)
          const isSelected = selectedModels.has(name)

          let className = 'text-xs px-2 py-0.5 rounded border transition-colors disabled:opacity-50 '
          if (isExcluded) {
            className += 'bg-red-50 border-red-300 text-red-700 line-through'
          } else if (isSelected) {
            className += 'bg-blue-50 border-blue-300 text-blue-700'
          } else {
            className += 'bg-white border-gray-200 text-gray-600 hover:border-gray-300'
          }

          return (
            <button
              key={name}
              onClick={(e) => {
                if (e.shiftKey) {
                  if (isExcluded) {
                    onExcludeChange(removeToken(excludeText, name))
                  } else {
                    onExcludeChange(addToken(excludeText, name))
                    onSelectChange(removeToken(selectText, name))
                  }
                } else {
                  if (isSelected) {
                    onSelectChange(removeToken(selectText, name))
                  } else {
                    onSelectChange(addToken(selectText, name))
                    onExcludeChange(removeToken(excludeText, name))
                  }
                }
              }}
              disabled={disabled}
              className={className}
            >
              {name}
            </button>
          )
        })}
      </div>
    </div>
  )
}

export function RunPlanner() {
  const today = formatDate(new Date())
  const thirtyDaysAgo = formatDate(new Date(Date.now() - 30 * 24 * 60 * 60 * 1000))

  const [start, setStart] = useState(thirtyDaysAgo)
  const [end, setEnd] = useState(today)
  const [batchSize, setBatchSize] = useState('')
  const [perPartition, setPerPartition] = useState(false)
  const [selectText, setSelectText] = useState('')
  const [excludeText, setExcludeText] = useState('')

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

  const selectTokens = useMemo(() => parseTokens(selectText), [selectText])
  const excludeTokens = useMemo(() => parseTokens(excludeText), [excludeText])

  // Resolve selectors via backend API for pill highlighting
  const hasSelectors = selectTokens.length > 0 || excludeTokens.length > 0
  const { data: resolved } = useQuery({
    queryKey: ['resolve', selectTokens, excludeTokens],
    queryFn: () => resolveSelectors({
      select: selectTokens.length > 0 ? selectTokens : undefined,
      exclude: excludeTokens.length > 0 ? excludeTokens : undefined,
    }),
    enabled: hasSelectors,
    placeholderData: (prev) => prev,
  })

  const selectedModels = useMemo(
    () => new Set(resolved?.selected ?? []),
    [resolved?.selected]
  )
  const excludedModels = useMemo(
    () => new Set(resolved?.excluded ?? []),
    [resolved?.excluded]
  )

  function handlePreview() {
    planMutation.mutate({
      start,
      end,
      batch_size_days: batchSize ? parseInt(batchSize) : undefined,
      per_partition: perPartition,
      select: selectTokens.length > 0 ? selectTokens : undefined,
      exclude: excludeTokens.length > 0 ? excludeTokens : undefined,
    })
  }

  function handleExecute() {
    executeMutation.mutate({
      start,
      end,
      batch_size_days: batchSize ? parseInt(batchSize) : undefined,
      per_partition: perPartition,
      select: selectTokens.length > 0 ? selectTokens : undefined,
      exclude: excludeTokens.length > 0 ? excludeTokens : undefined,
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

        <div className="bg-white rounded-lg border border-gray-200 p-4 mb-6 space-y-4">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
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

          {/* Select / Exclude inputs */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <SelectorInput
              label={`Select${selectedModels.size > 0 ? ` (${selectedModels.size} models)` : ''}`}
              value={selectText}
              onChange={setSelectText}
              placeholder="e.g. +tag:revenue daily_revenue"
              disabled={isRunning}
            />
            <SelectorInput
              label={`Exclude${excludedModels.size > 0 ? ` (${excludedModels.size} models)` : ''}`}
              value={excludeText}
              onChange={setExcludeText}
              placeholder="e.g. staging_model tag:debug"
              disabled={isRunning}
            />
          </div>

          {/* Model pills — click to toggle in the text inputs */}
          {allModels.length > 0 && (
            <ModelPills
              models={allModels}
              selectedModels={selectedModels}
              excludedModels={excludedModels}
              selectText={selectText}
              excludeText={excludeText}
              onSelectChange={setSelectText}
              onExcludeChange={setExcludeText}
              disabled={isRunning}
            />
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
