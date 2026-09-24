import { signal } from '@preact/signals'

import { CLOSE_TRY_AGAIN_LATER, nextReconnectDelay } from './ws-backoff'

export type WsChannel =
  | 'adapters'
  | 'physical_devices'
  | 'virtual_lamps'
  | 'groups'
  | 'scenes'
  | 'operations'
  | 'stats'
  | 'diagnostics'
  | 'sniffer'
  | 'input'
  | 'rules'
  | 'logs'

export interface WsEvent {
  type: string
  channel: WsChannel
  ts_ms: number
  payload: unknown
}

export type ConnectionState = 'connecting' | 'live' | 'reconnecting' | 'down' | 'refused'

export const connection = signal<ConnectionState>('connecting')
export const connectionReason = signal<string>('connecting…')

const DOWN_AFTER_MS = 60_000

const TERMINAL_ERROR_REASONS: Record<string, string> = {
  ws_origin_rejected:
    'refused: this page was not served by the controller, so the push channel is closed to it — screens poll instead',
}

type Listener = (event: WsEvent) => void

const listeners = new Map<WsChannel, Set<Listener>>()
const wanted = new Map<WsChannel, number>()

let socket: WebSocket | null = null
let attempt = 0
let refusedForCapacity = false
let capacityThisSocket = false
let downSince: number | null = null
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let greeted = false
let refused = false

function wsUrl(): string {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${location.host}/api/v1/ws`
}

function activeChannels(): WsChannel[] {
  return [...wanted.entries()].filter(([, n]) => n > 0).map(([c]) => c)
}

let logLevel: LogLevel = 'info'

export type LogLevel = 'error' | 'warn' | 'info' | 'debug' | 'trace'

function send(op: 'subscribe' | 'unsubscribe', channels: WsChannel[]) {
  if (!channels.length || !greeted || socket?.readyState !== WebSocket.OPEN) return
  const frame: Record<string, unknown> = { op, channels }
  if (op === 'subscribe' && channels.includes('logs')) {
    frame.logs = { min_level: logLevel }
  }
  socket.send(JSON.stringify(frame))
}

export function setLogLevel(level: LogLevel) {
  logLevel = level
  if ((wanted.get('logs') ?? 0) > 0) send('subscribe', ['logs'])
}

function setState(state: ConnectionState, reason: string) {
  connection.value = state
  connectionReason.value = reason
}

function onHello() {
  greeted = true
  attempt = 0
  refusedForCapacity = false
  downSince = null
  setState('live', 'live — updates pushed by the controller')
  send('subscribe', activeChannels())
}

function onServerFrame(raw: string) {
  let frame: Record<string, unknown>
  try {
    frame = JSON.parse(raw)
  } catch {
    return
  }
  if (frame.op === 'hello') {
    onHello()
    return
  }
  if (frame.op === 'error') {
    handleErrorFrame(frame)
    return
  }
  if (frame.op) return
  const event = frame as unknown as WsEvent
  if (!event.channel) return
  deliver(event)
}

function handleErrorFrame(frame: Record<string, unknown>) {
  const error = frame.error as { code?: string; message?: string } | undefined
  const code = error?.code ?? ''
  if (code === 'ws_clients_exhausted') {
    capacityThisSocket = true
    setState('reconnecting', 'controller busy — too many open tabs; polling')
    return
  }
  const terminal = TERMINAL_ERROR_REASONS[code]
  if (terminal) {
    refuse(terminal)
    return
  }
  console.warn('ws error frame', error)
}

function refuse(reason: string) {
  refused = true
  if (reconnectTimer) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
  setState('refused', reason)
  socket?.close()
}

function deliver(event: WsEvent) {
  if (event.type === 'DropNotice') {
    const channel = (event as unknown as { channel: string }).channel
    const targets = channel === '*' ? [...listeners.keys()] : [channel as WsChannel]
    for (const target of targets) {
      for (const listener of listeners.get(target) ?? []) listener({ ...event, channel: target })
    }
    return
  }
  for (const listener of listeners.get(event.channel) ?? []) listener(event)
}

function scheduleReconnect() {
  if (refused || reconnectTimer) return
  const delay = nextReconnectDelay(attempt, refusedForCapacity)
  attempt += 1
  downSince ??= Date.now()
  const outFor = Date.now() - downSince
  if (outFor > DOWN_AFTER_MS) {
    setState('down', `no connection for ${Math.round(outFor / 1000)} s — polling only`)
  } else if (connection.value !== 'reconnecting') {
    setState('reconnecting', 'reconnecting — polling meanwhile')
  }
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null
    connect()
  }, delay)
}

export function connect() {
  if (refused) return
  if (socket && socket.readyState <= WebSocket.OPEN) return
  greeted = false
  let next: WebSocket
  try {
    next = new WebSocket(wsUrl())
  } catch {
    scheduleReconnect()
    return
  }
  socket = next
  capacityThisSocket = false
  next.onmessage = (ev) => onServerFrame(String(ev.data))
  next.onclose = (ev) => {
    if (socket === next) socket = null
    greeted = false
    refusedForCapacity = capacityThisSocket || ev.code === CLOSE_TRY_AGAIN_LATER
    if (ev.code === CLOSE_TRY_AGAIN_LATER) {
      setState('reconnecting', 'controller busy — too many open tabs; polling')
    }
    scheduleReconnect()
  }
  next.onerror = () => next.close()
}

export function subscribe(channels: WsChannel[], listener: Listener): () => void {
  const added: WsChannel[] = []
  for (const channel of channels) {
    const before = wanted.get(channel) ?? 0
    wanted.set(channel, before + 1)
    if (before === 0) added.push(channel)
    let set = listeners.get(channel)
    if (!set) {
      set = new Set()
      listeners.set(channel, set)
    }
    set.add(listener)
  }
  send('subscribe', added)
  return () => {
    const removed: WsChannel[] = []
    for (const channel of channels) {
      const before = wanted.get(channel) ?? 0
      const after = Math.max(0, before - 1)
      wanted.set(channel, after)
      if (after === 0) removed.push(channel)
      listeners.get(channel)?.delete(listener)
    }
    send('unsubscribe', removed)
  }
}
