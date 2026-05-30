import { Handle, Position, type Node, type NodeProps } from '@xyflow/react'
import type { GraphNode } from '../types'

type ModelNodeData = GraphNode & Record<string, unknown>
type ModelNodeType = Node<ModelNodeData, 'model'>

export function ModelNode({ data, selected }: NodeProps<ModelNodeType>) {
  const isSource = data.node_type === 'source'
  const isTest = data.node_type === 'test'
  const hasErrors = data.has_errors

  let bgColor = 'bg-white'
  let borderColor = 'border-gray-300'
  let textColor = 'text-gray-900'

  if (selected) {
    borderColor = 'border-blue-500'
    bgColor = 'bg-blue-50'
  } else if (hasErrors) {
    borderColor = 'border-red-400'
    bgColor = 'bg-red-50'
  } else if (isTest) {
    borderColor = 'border-violet-400'
    bgColor = 'bg-violet-50'
    textColor = 'text-violet-900'
  } else if (isSource) {
    borderColor = 'border-emerald-400'
    bgColor = 'bg-emerald-50'
    textColor = 'text-emerald-900'
  }

  const matLabel = data.materialization
  const matBadgeColor = isSource
    ? 'bg-emerald-100 text-emerald-700'
    : isTest
      ? 'bg-violet-100 text-violet-700'
      : matLabel === 'table'
        ? 'bg-amber-100 text-amber-700'
        : 'bg-sky-100 text-sky-700'

  return (
    <div
      className={`${bgColor} ${borderColor} border rounded-lg shadow-sm px-3 py-2 min-w-[180px] cursor-pointer hover:shadow-md transition-shadow`}
    >
      <Handle type="target" position={Position.Top} className="!bg-gray-400 !w-2 !h-2" />

      <div className="flex items-center justify-between gap-2">
        <span className={`text-sm font-medium ${textColor} truncate`} title={data.label}>{data.label}</span>
        {matLabel && (
          <span className={`text-[10px] px-1.5 py-0.5 rounded font-medium ${matBadgeColor} shrink-0`}>
            {matLabel}
          </span>
        )}
      </div>

      {data.tags.length > 0 && (
        <div className="flex gap-1 mt-1 flex-wrap">
          {data.tags.slice(0, 3).map((tag) => (
            <span key={tag} className="text-[10px] bg-gray-100 text-gray-600 px-1 rounded">
              {tag}
            </span>
          ))}
          {data.tags.length > 3 && (
            <span className="text-[10px] text-gray-400">+{data.tags.length - 3}</span>
          )}
        </div>
      )}

      <Handle type="source" position={Position.Bottom} className="!bg-gray-400 !w-2 !h-2" />
    </div>
  )
}
