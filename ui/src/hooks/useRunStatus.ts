import { useEffect, useRef, useState, useCallback } from 'react'
import type { RunProgressEvent } from '../types'

export interface RunStatus {
  state: 'idle' | 'running'
  runId?: string
  currentModel?: string
  modelsCompleted: number
  modelsTotal: number
  batchesCompleted: number
  batchesTotal: number
  events: RunProgressEvent[]
  error?: string
}

const initialStatus: RunStatus = {
  state: 'idle',
  modelsCompleted: 0,
  modelsTotal: 0,
  batchesCompleted: 0,
  batchesTotal: 0,
  events: [],
}

export function useRunStatus() {
  const [status, setStatus] = useState<RunStatus>(initialStatus)
  const statusRef = useRef(status)
  statusRef.current = status

  const handleEvent = useCallback((event: RunProgressEvent) => {
    setStatus(prev => {
      const events = [...prev.events, event]
      switch (event.type) {
        case 'run_started':
          return {
            state: 'running',
            runId: event.run_id,
            modelsCompleted: 0,
            modelsTotal: event.models.length,
            batchesCompleted: 0,
            batchesTotal: event.total_batches,
            events,
          }
        case 'model_started':
          return { ...prev, currentModel: event.model, events }
        case 'batch_completed':
          return {
            ...prev,
            batchesCompleted: prev.batchesCompleted + 1,
            events,
          }
        case 'model_completed':
          return {
            ...prev,
            modelsCompleted: prev.modelsCompleted + 1,
            currentModel: undefined,
            events,
          }
        case 'run_completed':
          return { ...initialStatus, events }
        case 'run_failed':
          return { ...initialStatus, error: event.error, events }
        case 'run_cancelled':
          return { ...initialStatus, events }
        default:
          return { ...prev, events }
      }
    })
  }, [])

  const startRun = useCallback((runId: string) => {
    setStatus(prev => ({
      ...prev,
      state: 'running',
      runId,
      events: [],
      error: undefined,
    }))
  }, [])

  const clearEvents = useCallback(() => {
    setStatus(prev => ({ ...prev, events: [], error: undefined }))
  }, [])

  // Register with the global WebSocket event handler
  useEffect(() => {
    runStatusHandlers.add(handleEvent)
    return () => { runStatusHandlers.delete(handleEvent) }
  }, [handleEvent])

  return { status, startRun, clearEvents }
}

// Global set of handlers that the WebSocket hook dispatches to
export const runStatusHandlers = new Set<(event: RunProgressEvent) => void>()
