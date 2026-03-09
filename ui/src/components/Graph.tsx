import { useCallback, useEffect, useMemo } from 'react'
import {
  ReactFlow,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type NodeMouseHandler,
  Background,
  Controls,
  BackgroundVariant,
} from '@xyflow/react'
import dagre from '@dagrejs/dagre'
import '@xyflow/react/dist/style.css'
import type { GraphResponse } from '../types'
import { ModelNode } from './ModelNode'

const nodeWidth = 200
const nodeHeight = 60

function layoutGraph(data: GraphResponse): { nodes: Node[]; edges: Edge[] } {
  const g = new dagre.graphlib.Graph()
  g.setDefaultEdgeLabel(() => ({}))
  g.setGraph({ rankdir: 'TB', nodesep: 40, ranksep: 60 })

  for (const node of data.nodes) {
    g.setNode(node.id, { width: nodeWidth, height: nodeHeight })
  }

  for (const edge of data.edges) {
    g.setEdge(edge.source, edge.target)
  }

  dagre.layout(g)

  const nodes: Node[] = data.nodes.map((node) => {
    const pos = g.node(node.id)
    return {
      id: node.id,
      type: 'model' as const,
      position: { x: pos.x - nodeWidth / 2, y: pos.y - nodeHeight / 2 },
      data: { ...node } as Record<string, unknown>,
    }
  })

  const edges: Edge[] = data.edges.map((edge, i) => ({
    id: `e-${i}`,
    source: edge.source,
    target: edge.target,
    animated: false,
    style: { stroke: '#94a3b8' },
  }))

  return { nodes, edges }
}

const nodeTypes = { model: ModelNode }

interface GraphProps {
  data: GraphResponse
  selectedModel: string | null
  onSelectModel: (name: string | null) => void
}

export function Graph({ data, selectedModel, onSelectModel }: GraphProps) {
  const layout = useMemo(() => layoutGraph(data), [data])
  const [nodes, setNodes, onNodesChange] = useNodesState(layout.nodes)
  const [edges, setEdges, onEdgesChange] = useEdgesState(layout.edges)

  useEffect(() => {
    setNodes(layout.nodes)
    setEdges(layout.edges)
  }, [layout, setNodes, setEdges])

  // Highlight selected node via ReactFlow's built-in selected property
  useEffect(() => {
    setNodes((nds) =>
      nds.map((n) => ({
        ...n,
        selected: n.id === selectedModel,
      }))
    )
  }, [selectedModel, setNodes])

  const onNodeClick: NodeMouseHandler = useCallback(
    (_event, node) => {
      onSelectModel(node.id === selectedModel ? null : node.id)
    },
    [onSelectModel, selectedModel]
  )

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      onNodeClick={onNodeClick}
      nodeTypes={nodeTypes}
      fitView
      minZoom={0.1}
      maxZoom={2}
    >
      <Background variant={BackgroundVariant.Dots} gap={16} size={1} />
      <Controls />
    </ReactFlow>
  )
}
