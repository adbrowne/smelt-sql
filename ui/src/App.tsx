import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { fetchGraph, fetchProject, fetchModel } from './api'
import { ErrorBoundary } from './components/ErrorBoundary'
import { Graph } from './components/Graph'
import { ModelDetail } from './components/ModelDetail'
import { RunPlanner } from './pages/RunPlanner'
import { RunHistory } from './pages/RunHistory'
import { useWebSocket } from './hooks/useWebSocket'

type Page = 'graph' | 'planner' | 'history'

function App() {
  const [page, setPage] = useState<Page>('graph')
  const [selectedModel, setSelectedModel] = useState<string | null>(null)
  const wsStatus = useWebSocket()

  const { data: project } = useQuery({
    queryKey: ['project'],
    queryFn: fetchProject,
  })

  const { data: graphData, isLoading, error } = useQuery({
    queryKey: ['graph'],
    queryFn: fetchGraph,
  })

  const { data: modelDetail } = useQuery({
    queryKey: ['model', selectedModel],
    queryFn: () => fetchModel(selectedModel!),
    enabled: !!selectedModel,
  })

  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center bg-gray-50">
        <p className="text-gray-500">Loading model graph...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center bg-gray-50">
        <p className="text-red-500">Error: {(error as Error).message}</p>
      </div>
    )
  }

  const navItems: { key: Page; label: string }[] = [
    { key: 'graph', label: 'Graph' },
    { key: 'planner', label: 'Run Planner' },
    { key: 'history', label: 'History' },
  ]

  return (
    <ErrorBoundary>
      <div className="h-full flex flex-col bg-gray-50">
        <header className="bg-white border-b border-gray-200 px-4 py-2 flex items-center gap-4 shrink-0">
          <h1 className="text-lg font-semibold text-gray-900">smelt</h1>
          {project && (
            <span className="text-sm text-gray-500">
              {project.name} &middot; {project.model_count} models
              {project.source_count > 0 && ` \u00b7 ${project.source_count} sources`}
            </span>
          )}

          <nav className="ml-4 flex gap-1">
            {navItems.map(({ key, label }) => (
              <button
                key={key}
                onClick={() => setPage(key)}
                className={`text-sm px-3 py-1 rounded ${
                  page === key
                    ? 'bg-gray-900 text-white'
                    : 'text-gray-600 hover:bg-gray-100'
                }`}
              >
                {label}
              </button>
            ))}
          </nav>

          <div className="ml-auto flex items-center gap-2">
            <span
              className={`inline-block w-2 h-2 rounded-full ${
                wsStatus === 'connected'
                  ? 'bg-green-500'
                  : wsStatus === 'connecting'
                    ? 'bg-yellow-500'
                    : 'bg-gray-400'
              }`}
              title={`Live updates: ${wsStatus}`}
            />
            <span className="text-xs text-gray-400">
              {wsStatus === 'connected' ? 'live' : wsStatus}
            </span>
          </div>
        </header>

        <div className="flex-1 flex min-h-0">
          {page === 'graph' && (
            <>
              <div className="flex-1">
                {graphData && (
                  <Graph
                    data={graphData}
                    selectedModel={selectedModel}
                    onSelectModel={setSelectedModel}
                  />
                )}
              </div>

              {selectedModel && modelDetail && (
                <ModelDetail
                  model={modelDetail}
                  onClose={() => setSelectedModel(null)}
                />
              )}
            </>
          )}

          {page === 'planner' && <RunPlanner />}
          {page === 'history' && <RunHistory />}
        </div>
      </div>
    </ErrorBoundary>
  )
}

export default App
