import { useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { runStatusHandlers } from './useRunStatus'

type ConnectionStatus = 'connecting' | 'connected' | 'disconnected'

const RUN_EVENT_TYPES = new Set([
  'run_started', 'model_started', 'batch_completed',
  'model_completed', 'run_completed', 'run_failed', 'run_cancelled',
])

export function useWebSocket() {
  const queryClient = useQueryClient()
  const wsRef = useRef<WebSocket | null>(null)
  const [status, setStatus] = useState<ConnectionStatus>('disconnected')
  const reconnectTimeout = useRef<ReturnType<typeof setTimeout>>(null)

  useEffect(() => {
    function connect() {
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      const wsUrl = `${protocol}//${window.location.host}/ws`

      setStatus('connecting')
      const ws = new WebSocket(wsUrl)
      wsRef.current = ws

      ws.onopen = () => {
        setStatus('connected')
      }

      ws.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data)
          if (data.type === 'models_updated') {
            queryClient.invalidateQueries()
          } else if (RUN_EVENT_TYPES.has(data.type)) {
            for (const handler of runStatusHandlers) {
              handler(data)
            }
            // Refetch run status and history when a run completes/fails
            if (data.type === 'run_completed' || data.type === 'run_failed' || data.type === 'run_cancelled') {
              queryClient.invalidateQueries({ queryKey: ['runs'] })
              queryClient.invalidateQueries({ queryKey: ['runStatus'] })
            }
          }
        } catch {
          // Ignore malformed messages
        }
      }

      ws.onclose = () => {
        setStatus('disconnected')
        wsRef.current = null
        reconnectTimeout.current = setTimeout(connect, 2000)
      }

      ws.onerror = () => {
        ws.close()
      }
    }

    connect()

    return () => {
      if (reconnectTimeout.current) {
        clearTimeout(reconnectTimeout.current)
      }
      if (wsRef.current) {
        wsRef.current.close()
      }
    }
  }, [queryClient])

  return status
}
