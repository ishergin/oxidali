import type {
  LuminaireNumberKey,
  LuminaireTextKey,
  MemoryLuminairePatch,
  Operation,
} from '../api/types.js'
import { requestRefused, isPreempted, opFailure, PREEMPTED_DETAIL } from '../format.js'

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

export interface WriteStarts {
  accepted: boolean
  refusal: { status?: number; text: string } | null
}

export type WriteOutcome =
  | { kind: 'succeeded' }
  | { kind: 'refused' | 'failed' | 'not_known'; text: string }

const NOT_KNOWN: WriteOutcome = { kind: 'not_known', text: OUTCOME_NOT_KNOWN }

export function writeOutcome(op: Pick<Operation, 'status' | 'error'> | null, starts: WriteStarts): WriteOutcome {
  if (op) return operationOutcome(op)
  const refusal = starts.refusal
  if (refusal && !starts.accepted && requestRefused(refusal.status)) {
    return { kind: 'refused', text: refusal.text }
  }
  return NOT_KNOWN
}

function operationOutcome(op: Pick<Operation, 'status' | 'error'>): WriteOutcome {
  if (op.status === 'succeeded') return { kind: 'succeeded' }
  if (op.status !== 'failed') return NOT_KNOWN
  if (isPreempted(op)) {
    return { kind: 'failed', text: `${PREEMPTED_DETAIL}; the gear may hold part of the write — read bank 1.` }
  }
  const why = opFailure(op)
  return { kind: 'failed', text: why ? `The write failed: ${why}.` : 'The write failed.' }
}
