import { signal } from '@preact/signals'
import { useEffect } from 'preact/hooks'
import { api } from './api/client'
import type { ClusterInfo, ClusterStatus } from './api/types'
import type { WsEvent } from './api/ws'
import { DEFAULT_POLL_MS, useLive } from './hooks'

export interface StatusRead {
  status: ClusterStatus
  at: number
}

export const clusterStatus = signal<StatusRead | null>(null)

let framesTaken = 0

function takeStatus(event: WsEvent): boolean {
  if (event.type === 'ClusterInfo') return true
  if (event.type !== 'ClusterStatus' || !event.payload || document.hidden) return false
  framesTaken += 1
  clusterStatus.value = { status: event.payload as ClusterStatus, at: Date.now() }
  return true
}

interface PolledStatus {
  status: ClusterStatus
  framesBefore: number
}

async function pollStatus(): Promise<PolledStatus> {
  const framesBefore = framesTaken
  return { status: await api.clusterStatus(), framesBefore }
}

export function useClusterStatus(intervalMs: number = DEFAULT_POLL_MS): {
  read: StatusRead | null
  reload: () => Promise<void>
} {
  const { data, reload } = useLive(pollStatus, ['cluster'], { intervalMs, onEvent: takeStatus })
  useEffect(() => {
    if (data && data.framesBefore === framesTaken) clusterStatus.value = { status: data.status, at: Date.now() }
  }, [data])
  return { read: clusterStatus.value, reload }
}

function statusOnly(event: WsEvent): boolean {
  return event.type === 'ClusterStatus'
}

export function useClusterInfo(): { info: ClusterInfo | null; reload: () => Promise<void> } {
  const { data, reload } = useLive(() => api.clusterInfo(), ['cluster'], { onEvent: statusOnly })
  return { info: data, reload }
}
