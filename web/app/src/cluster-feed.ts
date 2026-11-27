import { signal } from '@preact/signals'
import { useEffect } from 'preact/hooks'
import { api } from './api/client'
import type { ClusterInfo, ClusterStatus } from './api/types'
import type { WsEvent } from './api/ws'
import { DEFAULT_POLL_MS, useLive, usePoll } from './hooks'

export const clusterStatus = signal<ClusterStatus | null>(null)

function takeStatus(event: WsEvent): boolean {
  if (event.type === 'ClusterStatus') {
    if (event.payload) clusterStatus.value = event.payload as ClusterStatus
    return event.payload !== null
  }
  return event.type === 'ClusterInfo'
}

export function useClusterStatus(intervalMs: number = DEFAULT_POLL_MS): {
  status: ClusterStatus | null
  reload: () => Promise<void>
} {
  const { data, reload } = useLive(() => api.clusterStatus(), ['cluster'], {
    intervalMs,
    onEvent: takeStatus,
  })
  useEffect(() => {
    if (data) clusterStatus.value = data
  }, [data])
  return { status: clusterStatus.value, reload }
}

export function useClusterInfo(): ClusterInfo | null {
  return usePoll(() => api.clusterInfo()).data
}
