import type {
  LuminaireNumberKey,
  LuminaireTextKey,
  MemoryLuminairePatch,
  Operation,
  OperationAccepted,
} from '../api/types.js'
import { isPreempted, opEnd, opFailure, PREEMPTED_DETAIL, startRefused } from '../format.js'

export interface LuminaireDraft {
  numbers: Partial<Record<LuminaireNumberKey, string>>
  texts: Partial<Record<LuminaireTextKey, string>>
}

export const EMPTY_DRAFT: LuminaireDraft = { numbers: {}, texts: {} }

export const WHOLE_NUMBER_NOTE = 'A number here is whole, non-negative and decimal.'
export const ONE_TEXT_NOTE = 'One text field per write: the controller takes one per request.'
export const OUTCOME_NOT_KNOWN = 'Outcome not known. Read bank 1 to see what the gear holds.'

const WHOLE_DECIMAL = /^[0-9]+$/
const TEXT_FIELDS_PER_WRITE = 1

export function wholeNumber(raw: string): number | null {
  return WHOLE_DECIMAL.test(raw) ? Number(raw) : null
}

export function touched<K extends keyof LuminaireDraft>(
  draft: LuminaireDraft,
  kind: K,
  key: keyof LuminaireDraft[K],
  raw: string,
): LuminaireDraft {
  return { ...draft, [kind]: { ...draft[kind], [key]: raw } }
}

export function draftNotes(draft: LuminaireDraft): string[] {
  const notes: string[] = []
  if (Object.values(draft.numbers).some((raw) => wholeNumber(raw ?? '') === null)) {
    notes.push(WHOLE_NUMBER_NOTE)
  }
  if (Object.keys(draft.texts).length > TEXT_FIELDS_PER_WRITE) notes.push(ONE_TEXT_NOTE)
  return notes
}

export function canWrite(draft: LuminaireDraft): boolean {
  const touched = Object.keys(draft.numbers).length + Object.keys(draft.texts).length
  return touched > 0 && draftNotes(draft).length === 0
}

export function patchBody(draft: LuminaireDraft): MemoryLuminairePatch {
  const body: MemoryLuminairePatch = { ...draft.texts }
  for (const [key, raw] of Object.entries(draft.numbers)) {
    body[key as LuminaireNumberKey] = Number(raw)
  }
  return body
}

export interface StartFailure {
  status?: number
  text: string
}

export interface WriteStarts {
  accepted: boolean
  refusal: StartFailure | null
}

export type WriteOutcome =
  | { kind: 'succeeded' }
  | { kind: 'refused' | 'failed' | 'not_known'; text: string }

type EndedOperation = Pick<Operation, 'status' | 'error'>

const NOT_KNOWN: WriteOutcome = { kind: 'not_known', text: OUTCOME_NOT_KNOWN }

export function writeOutcome(op: EndedOperation | null, starts: WriteStarts): WriteOutcome {
  if (op) return operationOutcome(op)
  const refusal = starts.refusal
  if (refusal && startRefused(refusal.status, starts.accepted)) {
    return { kind: 'refused', text: refusal.text }
  }
  return NOT_KNOWN
}

function operationOutcome(op: EndedOperation): WriteOutcome {
  const end = opEnd(op)
  if (end !== 'failed') return end === 'succeeded' ? { kind: 'succeeded' } : NOT_KNOWN
  if (isPreempted(op)) {
    return { kind: 'failed', text: `${PREEMPTED_DETAIL}; the gear may hold part of the write — read bank 1.` }
  }
  return { kind: 'failed', text: opFailure(op) ?? 'The write failed.' }
}

export interface WriteSteps {
  run: (start: () => Promise<OperationAccepted>) => Promise<EndedOperation | null>
  patch: (body: MemoryLuminairePatch) => Promise<OperationAccepted>
  describe: (e: unknown) => StartFailure
  setBusy: (busy: boolean) => void
  setOutcome: (outcome: WriteOutcome | null) => void
  reload: () => Promise<void>
  onClose: () => void
}

export async function writeBank1(draft: LuminaireDraft, steps: WriteSteps): Promise<void> {
  steps.setBusy(true)
  steps.setOutcome(null)
  const outcome = await settle(draft, steps)
  steps.setBusy(false)
  if (outcome.kind !== 'succeeded') steps.setOutcome(outcome)
  await steps.reload()
  if (outcome.kind === 'succeeded') steps.onClose()
}

async function settle(draft: LuminaireDraft, steps: WriteSteps): Promise<WriteOutcome> {
  const starts: WriteStarts = { accepted: false, refusal: null }
  try {
    return writeOutcome(await steps.run(() => startWrite(draft, steps, starts)), starts)
  } catch {
    return NOT_KNOWN
  }
}

async function startWrite(draft: LuminaireDraft, steps: WriteSteps, starts: WriteStarts): Promise<OperationAccepted> {
  try {
    const accepted = await steps.patch(patchBody(draft))
    starts.accepted = true
    return accepted
  } catch (e) {
    starts.refusal = steps.describe(e)
    throw e
  }
}
