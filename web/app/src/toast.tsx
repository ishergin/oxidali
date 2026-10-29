import { signal } from '@preact/signals'
import { ApiError, awaitOperation } from './api/client'
import type { Operation, OperationAccepted } from './api/types'
import { Chip } from './components/ui'
import {
  isPreempted,
  notKnownToast,
  opFailure,
  opStatusChip,
  opStatusLabel,
  opSummary,
  PREEMPTED_DETAIL,
  startFailureToast,
} from './format'
import { runWithRetries, type RetryPorts } from './op-retry'

export interface Toast {
  id: number
  title: string
  status: string
  detail?: string
  errorCode?: string
}

export const toasts = signal<Toast[]>([])
let nextToastId = 1

const SUCCESS_DISMISS_MS = 3500
const FAILURE_DISMISS_MS = 9000
const PREEMPT_RETRIES = 3
const PREEMPT_RETRY_MS = 1200

function push(t: Omit<Toast, 'id'>): number {
  const id = nextToastId++
  toasts.value = [...toasts.value, { id, ...t }]
  return id
}

function update(id: number, patch: Partial<Toast>) {
  toasts.value = toasts.value.map((t) => (t.id === id ? { ...t, ...patch } : t))
}

export function dismissToast(id: number) {
  toasts.value = toasts.value.filter((t) => t.id !== id)
}

export function notify(title: string, status: 'info' | 'succeeded' | 'failed' = 'info', detail?: string) {
  const id = push({ title, status, detail })
  setTimeout(() => dismissToast(id), status === 'failed' ? FAILURE_DISMISS_MS : SUCCESS_DISMISS_MS)
}

export function errorMessage(e: unknown): string {
  if (e instanceof ApiError) {
    const msg = e.message && e.message !== e.code ? ` — ${e.message}` : ''
    return `${e.status} ${e.code}${msg}`
  }
  return e instanceof Error ? e.message : String(e)
}

export function failureOf(e: unknown): { status?: number; text: string } {
  return { status: e instanceof ApiError ? e.status : undefined, text: errorMessage(e) }
}

export function opCommitted(op: Operation | null): boolean {
  return op?.status === 'succeeded'
}

export async function mutate(
  title: string,
  fn: () => Promise<unknown>,
  after?: () => void,
): Promise<boolean> {
  try {
    await fn()
    after?.()
    return true
  } catch (e) {
    notify(title, 'failed', errorMessage(e))
    return false
  }
}

export async function mutateBusy(
  title: string,
  setBusy: (busy: boolean) => void,
  fn: () => Promise<unknown>,
  after?: () => void,
): Promise<boolean> {
  setBusy(true)
  try {
    return await mutate(title, fn, after)
  } finally {
    setBusy(false)
  }
}

export async function trackOp(
  title: string,
  accepted: OperationAccepted,
  willRetry = false,
): Promise<Operation | null> {
  const id = push({ title, status: 'accepted', detail: accepted.operation_id })
  try {
    const op = await awaitOperation(accepted, (o) => update(id, { status: o.status }))
    update(id, {
      status: op.status,
      errorCode: op.error?.code,
      detail: isPreempted(op)
        ? `${PREEMPTED_DETAIL}${willRetry ? ' — retrying' : ''}`
        : ([opFailure(op), opSummary(op)].filter(Boolean).join(' · ') || accepted.operation_id),
    })
    setTimeout(
      () => dismissToast(id),
      op.status === 'succeeded' ? SUCCESS_DISMISS_MS : FAILURE_DISMISS_MS,
    )
    return op
  } catch (e) {
    update(id, notKnownToast(errorMessage(e)))
    setTimeout(() => dismissToast(id), FAILURE_DISMISS_MS)
    return null
  }
}

export function notifyNotKnown(title: string, reason: string) {
  const id = push({ title, ...notKnownToast(reason) })
  setTimeout(() => dismissToast(id), FAILURE_DISMISS_MS)
}

export function notifyStartFailure(title: string, e: unknown, retried: boolean) {
  const id = push({ title, ...startFailureToast(failureOf(e), retried) })
  setTimeout(() => dismissToast(id), FAILURE_DISMISS_MS)
}

export function runOp(title: string, start: () => Promise<OperationAccepted>): Promise<Operation | null> {
  const ports: RetryPorts = {
    start,
    track: (accepted, willRetry) => trackOp(title, accepted, willRetry),
    startFailed: (e, retried) => notifyStartFailure(title, e, retried),
    pause: () => new Promise((r) => setTimeout(r, PREEMPT_RETRY_MS)),
  }
  return runWithRetries(ports, PREEMPT_RETRIES)
}

export interface SaveThenApply {
  subject: string
  setBusy: (busy: boolean) => void
  save: (() => Promise<OperationAccepted>) | null
  apply: () => Promise<OperationAccepted | object>
  nothingToApply: string
  onApplied: () => Promise<unknown> | void
}

export type ApplyOutcome = 'committed' | 'failed' | 'unsaved'

export async function saveThenApply(args: SaveThenApply): Promise<ApplyOutcome> {
  const { subject, setBusy, save, apply, nothingToApply, onApplied } = args
  const applyTitle = `${subject} apply`
  let phase = `${subject} save`
  setBusy(true)
  try {
    if (save && !opCommitted(await trackOp(phase, await save()))) return 'unsaved'
    phase = applyTitle
    const resp = await apply()
    const tracked = 'operation_id' in resp
    const op = tracked ? await trackOp(applyTitle, resp as OperationAccepted) : null
    if (!tracked) notify(applyTitle, 'succeeded', nothingToApply)
    await onApplied()
    return tracked && !opCommitted(op) ? 'failed' : 'committed'
  } catch (e) {
    notify(phase, 'failed', errorMessage(e))
    return phase === applyTitle ? 'failed' : 'unsaved'
  } finally {
    setBusy(false)
  }
}

export function OperationToasts() {
  const list = toasts.value
  if (list.length === 0) return null
  return (
    <div class="toasts">
      {list.map((t) => {
        const chip = opStatusChip(t.status, t.errorCode)
        const active = t.status === 'accepted' || t.status === 'running'
        return (
          <div class="op" key={t.id}>
            <div class="row1">
              <span class="title">{t.title}</span>
              {t.status === 'info' ? null : (
                <Chip cls={chip.cls} spin={chip.spin}>
                  {opStatusLabel({ status: t.status, error: { code: t.errorCode } })}
                </Chip>
              )}
              <button class="close" onClick={() => dismissToast(t.id)} title="Dismiss">
                ✕
              </button>
            </div>
            {active && (
              <div class="bar indet">
                <div class="fill" />
              </div>
            )}
            {t.detail && <div class="detail">{t.detail}</div>}
          </div>
        )
      })}
    </div>
  )
}
