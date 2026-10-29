import { useState } from 'preact/hooks'
import { ApiError, api } from '../api/client'
import type {
  LuminaireNumberKey,
  LuminaireTextKey,
  MemoryLuminaire,
  OperationAccepted,
} from '../api/types'
import { Card, Src } from '../components/ui'
import { ADAPTER, lightDistributionLabel, pad2 } from '../format'
import { errorMessage, runOp } from '../toast'
import {
  canWrite,
  draftNotes,
  EMPTY_DRAFT,
  patchBody,
  touched,
  wholeNumber,
  writeOutcome,
  type LuminaireDraft,
  type WriteOutcome,
  type WriteStarts,
} from './luminaire-draft'

export interface LuminaireNumberRowSpec {
  key: LuminaireNumberKey
  label: string
  unit?: string
  render?: (value: number) => string
}

export const LUMINAIRE_NUMBER_ROWS: LuminaireNumberRowSpec[] = [
  { key: 'year', label: 'Manufactured (year)', render: (y) => `20${pad2(y)}` },
  { key: 'week', label: 'Manufactured (week)', render: (w) => `W${pad2(w)}` },
  { key: 'nominal_input_power_w', label: 'Nominal input power', unit: 'W' },
  { key: 'power_at_minimum_w', label: 'Power at minimum', unit: 'W' },
  { key: 'nominal_min_ac_voltage_v', label: 'Nominal minimum mains voltage', unit: 'V' },
  { key: 'nominal_max_ac_voltage_v', label: 'Nominal maximum mains voltage', unit: 'V' },
  { key: 'nominal_light_output_lm', label: 'Nominal light output', unit: 'lm' },
  { key: 'cri', label: 'CRI' },
  { key: 'cct_kelvin', label: 'CCT', unit: 'K' },
  {
    key: 'light_distribution_type',
    label: 'Light distribution type',
    render: (v) => lightDistributionLabel(v) ?? String(v),
  },
  { key: 'lamp_current_ma', label: 'Lamp current', unit: 'mA' },
]

export const LUMINAIRE_TEXT_ROWS: [key: LuminaireTextKey, label: string][] = [
  ['luminaire_identification', 'Luminaire identification'],
  ['luminaire_colour', 'Luminaire colour'],
  ['light_distribution', 'Light distribution'],
  ['oem_name', 'OEM name'],
  ['customer_stocking_number', 'Customer stocking number'],
  ['free_use', 'Free-use characters'],
]

export function hasEditableLeaf(lum: MemoryLuminaire): boolean {
  const keys = [...LUMINAIRE_NUMBER_ROWS.map((row) => row.key), ...LUMINAIRE_TEXT_ROWS.map(([key]) => key)]
  return keys.some((key) => lum[key] !== undefined)
}

type SetDraft = (update: (draft: LuminaireDraft) => LuminaireDraft) => void

interface EditorProps {
  lum: MemoryLuminaire
  short: number
  reload: () => void
  onClose: () => void
}

export function LuminaireEditor({ lum, short, reload, onClose }: EditorProps) {
  const [draft, setDraft] = useState<LuminaireDraft>(EMPTY_DRAFT)
  const [busy, setBusy] = useState(false)
  const [outcome, setOutcome] = useState<WriteOutcome | null>(null)
  const write = () => void writeBank1({ short, draft, setBusy, setOutcome, reload, onClose })
  const actions = <EditorActions busy={busy} writable={canWrite(draft)} onDiscard={onClose} onWrite={write} />
  return (
    <Card title="Luminaire data" span2 action={actions}>
      <NumberInputRows lum={lum} draft={draft} busy={busy} setDraft={setDraft} />
      <TextInputRows lum={lum} draft={draft} busy={busy} setDraft={setDraft} />
      {draftNotes(draft).map((note) => (
        <p class="hint" key={note}>{note}</p>
      ))}
      {outcome && <OutcomeLine outcome={outcome} />}
    </Card>
  )
}

function EditorActions(props: { busy: boolean; writable: boolean; onDiscard: () => void; onWrite: () => void }) {
  const { busy, writable, onDiscard, onWrite } = props
  return (
    <span class="btns">
      <button class="btn sm ghost" disabled={busy} onClick={onDiscard}>
        Discard
      </button>
      <button class="btn sm" disabled={busy || !writable} onClick={onWrite}>
        {busy ? 'Writing…' : 'Write'}
      </button>
    </span>
  )
}

interface RowsProps {
  lum: MemoryLuminaire
  draft: LuminaireDraft
  busy: boolean
  setDraft: SetDraft
}

function NumberInputRows(props: RowsProps) {
  return <>{LUMINAIRE_NUMBER_ROWS.map((row) => <NumberInputRow key={row.key} row={row} {...props} />)}</>
}

function NumberInputRow({ row, lum, draft, busy, setDraft }: RowsProps & { row: LuminaireNumberRowSpec }) {
  const ov = lum[row.key]
  if (!ov) return null
  const shown = ov.value.value === null ? '' : String(ov.value.value)
  const onInput = (value: string) => setDraft((d) => touched(d, 'numbers', row.key, value))
  return (
    <div class="attr">
      <span class="k">{row.label}</span>
      <span class="v">
        <DraftInput raw={draft.numbers[row.key]} shown={shown} numeric busy={busy} onInput={onInput} />
        <span class="unit">{row.unit ?? ''}</span>
      </span>
      <Src ov={ov} />
    </div>
  )
}

function TextInputRows(props: RowsProps) {
  return <>{LUMINAIRE_TEXT_ROWS.map(([key, label]) => <TextInputRow key={key} field={key} label={label} {...props} />)}</>
}

function TextInputRow(props: RowsProps & { field: LuminaireTextKey; label: string }) {
  const { field, label, lum, draft, busy, setDraft } = props
  const ov = lum[field]
  if (!ov) return null
  const onInput = (value: string) => setDraft((d) => touched(d, 'texts', field, value))
  return (
    <div class="attr text">
      <span class="k">{label}</span>
      <Src ov={ov} />
      <span class="v">
        <DraftInput raw={draft.texts[field]} shown={ov.value} busy={busy} onInput={onInput} />
      </span>
    </div>
  )
}

interface DraftInputProps {
  raw: string | undefined
  shown: string
  numeric?: boolean
  busy: boolean
  onInput: (value: string) => void
}

function DraftInput({ raw, shown, numeric, busy, onInput }: DraftInputProps) {
  return (
    <input
      class={raw === undefined ? undefined : 'dirty'}
      aria-invalid={numeric === true && raw !== undefined && wholeNumber(raw) === null}
      inputMode={numeric ? 'numeric' : undefined}
      value={raw ?? shown}
      disabled={busy}
      onInput={(e) => onInput(e.currentTarget.value)}
    />
  )
}

function OutcomeLine({ outcome }: { outcome: WriteOutcome }) {
  if (outcome.kind === 'succeeded') return null
  if (outcome.kind === 'not_known') return <div class="hintbox">{outcome.text}</div>
  return <div class="err-box">{outcome.text}</div>
}

interface WriteArgs {
  short: number
  draft: LuminaireDraft
  setBusy: (busy: boolean) => void
  setOutcome: (outcome: WriteOutcome | null) => void
  reload: () => void
  onClose: () => void
}

async function writeBank1({ short, draft, setBusy, setOutcome, reload, onClose }: WriteArgs) {
  const starts: WriteStarts = { accepted: false, refusal: null }
  setBusy(true)
  setOutcome(null)
  try {
    const op = await runOp(`Bank 1 write · SA ${pad2(short)}`, () => startWrite(short, draft, starts))
    const outcome = writeOutcome(op, starts)
    if (outcome.kind === 'succeeded') onClose()
    else setOutcome(outcome)
  } finally {
    setBusy(false)
    reload()
  }
}

async function startWrite(short: number, draft: LuminaireDraft, starts: WriteStarts): Promise<OperationAccepted> {
  try {
    const accepted = await api.patchMemoryLuminaire(ADAPTER, short, patchBody(draft))
    starts.accepted = true
    return accepted
  } catch (e) {
    starts.refusal = { status: e instanceof ApiError ? e.status : undefined, text: errorMessage(e) }
    throw e
  }
}
