import { useCallback, useEffect, useRef, useState } from 'preact/hooks'
import { createLatestRequestGate, runLatestRequest } from './latest-request'

import { connection, subscribe, type WsChannel, type WsEvent } from './api/ws'
import { mutateBusy, notify } from './toast'

export const DEFAULT_POLL_MS = 5000

const RECONCILE_MS = 60_000

const REFETCH_DEBOUNCE_MS = 150

const REFETCH_MIN_GAP_MS = 1000

export function usePoll<T>(
  fetcher: () => Promise<T>,
  intervalMs: number = DEFAULT_POLL_MS,
  deps: unknown[] = [],
) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  const alive = useRef(true)
  const requests = useRef(createLatestRequestGate())
  const fetcherRef = useRef(fetcher)
  fetcherRef.current = fetcher

  const reload = useCallback(async () => {
    await runLatestRequest(
      requests.current,
      () => fetcherRef.current(),
      (value) => {
        if (!alive.current) return
        setData(value)
        setError(null)
      },
      (cause) => {
        if (alive.current) setError(cause instanceof Error ? cause.message : String(cause))
      },
    )
  }, [])

  useEffect(() => {
    alive.current = true
    setData(null)
    setError(null)
    void reload()
    const onVisible = () => {
      if (!document.hidden) void reload()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      alive.current = false
      requests.current.invalidate()
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [reload, ...deps])

  useEffect(() => {
    const id = setInterval(() => {
      if (!document.hidden) void reload()
    }, intervalMs)
    return () => clearInterval(id)
  }, [intervalMs, reload])

  return { data, error, reload }
}

export function useLive<T>(
  fetcher: () => Promise<T>,
  channels: WsChannel[],
  options: {
    intervalMs?: number
    deps?: unknown[]
    onEvent?: (event: WsEvent) => boolean | void
  } = {},
) {
  const { intervalMs = DEFAULT_POLL_MS, deps = [], onEvent } = options
  const state = connection.value
  const live = state === 'live'
  const poll = usePoll(fetcher, live ? RECONCILE_MS : intervalMs, deps)
  const { reload } = poll
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastReload = useRef(0)
  const owed = useRef(false)
  const key = channels.join(',')

  const previousState = useRef(state)
  useEffect(() => {
    const before = previousState.current
    previousState.current = state
    if (state !== 'live' || before === 'live' || before === 'connecting') return
    if (document.hidden) {
      owed.current = true
      return
    }
    lastReload.current = Date.now()
    void reload()
  }, [state, reload])

  useEffect(() => {
    if (!channels.length) return
    const scheduleReload = () => {
      if (timer.current) return
      const since = Date.now() - lastReload.current
      const wait = Math.max(REFETCH_DEBOUNCE_MS, REFETCH_MIN_GAP_MS - since)
      timer.current = setTimeout(() => {
        timer.current = null
        lastReload.current = Date.now()
        void reload()
      }, wait)
    }
    const dispose = subscribe(channels, (event) => {
      if (onEventRef.current?.(event) === true) return
      if (document.hidden) {
        owed.current = true
        return
      }
      scheduleReload()
    })
    const onVisible = () => {
      if (document.hidden || !owed.current) return
      owed.current = false
      scheduleReload()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      dispose()
      document.removeEventListener('visibilitychange', onVisible)
      if (timer.current) clearTimeout(timer.current)
      timer.current = null
    }
  }, [key, reload])

  return poll
}

export interface SettingsDraft<T> {
  data: T | null
  current: T | null
  busy: boolean
  setDraft: (next: T | null) => void
  discard: () => void
  save: (title: string, send: () => Promise<unknown>) => Promise<void>
  withBusy: (fn: () => Promise<void>) => Promise<void>
  reload: () => void
}

export function useSettingsDraft<T>(fetcher: () => Promise<T>): SettingsDraft<T> {
  const { data, reload } = usePoll(fetcher)
  const [draft, setDraft] = useState<T | null>(null)
  const [busy, setBusy] = useState(false)

  const save = async (title: string, send: () => Promise<unknown>) => {
    await mutateBusy(title, setBusy, send, () => {
      setDraft(null)
      reload()
      notify(title, 'succeeded', 'Applied')
    })
  }

  const withBusy = async (fn: () => Promise<void>) => {
    setBusy(true)
    try {
      await fn()
    } finally {
      setBusy(false)
    }
  }

  return {
    data,
    current: draft ?? data,
    busy,
    setDraft,
    discard: () => setDraft(null),
    save,
    withBusy,
    reload,
  }
}

export function useSnapshotFrames<T>(eventType: string) {
  const [latest, setLatest] = useState<T | null>(null)
  const live = connection.value === 'live'

  useEffect(() => {
    if (!live) setLatest(null)
  }, [live])

  const consume = useCallback(
    (event: WsEvent): boolean => {
      if (event.type !== eventType) return false
      setLatest(event.payload as T)
      return true
    },
    [eventType],
  )

  return { latest: live ? latest : null, consume }
}
