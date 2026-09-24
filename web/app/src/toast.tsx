import { signal } from '@preact/signals'
import { ApiError, awaitOperation } from './api/client'
import type { Operation, OperationAccepted } from './api/types'
import { Chip } from './components/ui'
import { isPreempted, opStatusChip, opStatusLabel, opSummary } from './format'

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
        ? `Stood down for an operator command${willRetry ? ' — retrying' : ''}`
        : (opSummary(op) ?? accepted.operation_id),
    })
    setTimeout(
      () => dismissToast(id),
      op.status === 'succeeded' ? SUCCESS_DISMISS_MS : FAILURE_DISMISS_MS,
    )
    return op
  } catch (e) {
    update(id, { status: 'failed', detail: errorMessage(e) })
    setTimeout(() => dismissToast(id), FAILURE_DISMISS_MS)
    return null
  }
}

export async function runOp(
  title: string,
  start: () => Promise<OperationAccepted>,
): Promise<Operation | null> {
  for (let attempt = 0; ; attempt++) {
    let accepted: OperationAccepted
    try {
      accepted = await start()
    } catch (e) {
      notify(title, 'failed', errorMessage(e))
      return null
    }
    const op = await trackOp(title, accepted, attempt < PREEMPT_RETRIES)
    if (!isPreempted(op) || attempt >= PREEMPT_RETRIES) return op
    await new Promise((r) => setTimeout(r, PREEMPT_RETRY_MS))
  }
}

export interface SaveThenApply {
  subject: string
  setBusy: (busy: boolean) => void
  save: (() => Promise<OperationAccepted>) | null
  apply: () => Promise<OperationAccepted | object>
  nothingToApply: string
  onApplied: () => void
}

export async function saveThenApply(args: SaveThenApply): Promise<void> {
  const { subject, setBusy, save, apply, nothingToApply, onApplied } = args
  const applyTitle = `${subject} apply`
  let phase = `${subject} save`
  setBusy(true)
  try {
    if (save && !opCommitted(await trackOp(phase, await save()))) return
    phase = applyTitle
    const resp = await apply()
    if ('operation_id' in resp) await trackOp(applyTitle, resp as OperationAccepted)
    else notify(applyTitle, 'succeeded', nothingToApply)
    onApplied()
  } catch (e) {
    notify(phase, 'failed', errorMessage(e))
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
