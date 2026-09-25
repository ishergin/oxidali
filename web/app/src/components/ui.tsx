import type { ComponentChildren } from 'preact'
import { useEffect, useRef, useState } from 'preact/hooks'
import type { ObservedValue, RuntimeState } from '../api/types'
import { ago, hex2, lampState, LEVEL_MAX } from '../format'
import { useVerifying } from '../observation'
import { draftAfterPoll, draftAfterRefusal, type SliderDraft } from '../slider-draft'
import { type CctRange, cctSliderView } from './cct-view'

const IME_PROCESS_KEY_CODE = 229

function isImeComposing(e: KeyboardEvent): boolean {
  return e.isComposing || e.keyCode === IME_PROCESS_KEY_CODE
}

export function Chip({
  cls,
  spin,
  title,
  children,
}: {
  cls: string
  spin?: boolean
  title?: string
  children: ComponentChildren
}) {
  return (
    <span class={`chip ${cls}${spin ? ' spin' : ''}`} title={title}>
      <span class="dot" />
      {children}
    </span>
  )
}

export function Badge({
  cls,
  children,
}: {
  cls?: string
  children: ComponentChildren
}) {
  return <span class={`badge${cls ? ` ${cls}` : ''}`}>{children}</span>
}

export function Card({
  title,
  action,
  span2,
  children,
}: {
  title: string
  action?: ComponentChildren
  span2?: boolean
  children: ComponentChildren
}) {
  return (
    <div class={`card${span2 ? ' span2' : ''}`}>
      <header>
        <h3>{title}</h3>
        {action}
      </header>
      {children}
    </div>
  )
}

export function EditableText({
  value,
  cls,
  placeholder,
  autoFocus,
  disabled,
  inputMode,
  onCommit,
  onCancel,
}: {
  value: string
  cls?: string
  placeholder?: string
  autoFocus?: boolean
  disabled?: boolean
  inputMode?: 'numeric' | 'decimal' | 'text'
  onCommit: (v: string) => void
  onCancel?: () => void
}) {
  const klass = cls ?? 'inp'
  const [draft, setDraft] = useState<string | null>(null)
  const cancelled = useRef(false)
  const didAutoFocus = useRef(false)
  return (
    <input
      class={klass}
      value={draft ?? value}
      placeholder={placeholder}
      disabled={disabled}
      inputMode={inputMode}
      ref={(el) => {
        if (autoFocus && el && !didAutoFocus.current) {
          didAutoFocus.current = true
          el.focus()
        }
      }}
      onFocus={(e) => setDraft(e.currentTarget.value)}
      onInput={(e) => setDraft(e.currentTarget.value)}
      onKeyDown={(e) => {
        if (isImeComposing(e)) return
        if (e.key === 'Enter') e.currentTarget.blur()
        if (e.key === 'Escape') {
          cancelled.current = true
          e.currentTarget.blur()
        }
      }}
      onBlur={(e) => {
        const v = e.currentTarget.value
        setDraft(null)
        if (cancelled.current) {
          cancelled.current = false
          onCancel?.()
          return
        }
        onCommit(v)
      }}
    />
  )
}

export function EditableName({
  value,
  cls,
  placeholder,
  autoFocus,
  disabled,
  title,
  onCommit,
  onCancel,
  onDismiss,
}: {
  value: string
  cls?: string
  placeholder?: string
  autoFocus?: boolean
  disabled?: boolean
  title?: string
  onCommit: (v: string) => void
  onCancel?: () => void
  onDismiss?: () => void
}) {
  const [draft, setDraft] = useState<string | null>(null)
  const didAutoFocus = useRef(false)
  const dirty = draft !== null && draft !== value
  const klass = cls ?? 'name-edit'

  const commit = () => {
    const next = draft === null ? null : draft.trim()
    setDraft(null)
    if (next !== null && next !== '' && next !== value) onCommit(next)
    else onCancel?.()
  }
  const cancel = () => {
    setDraft(null)
    onCancel?.()
  }

  return (
    <span class="name-edit-wrap">
      <input
        class={`${klass}${dirty ? ' dirty' : ''}`}
        value={draft ?? value}
        placeholder={placeholder}
        disabled={disabled}
        title={title}
        ref={(el) => {
          if (autoFocus && el && !didAutoFocus.current) {
            didAutoFocus.current = true
            el.focus()
          }
        }}
        onFocus={(e) => setDraft(e.currentTarget.value)}
        onInput={(e) => setDraft(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (isImeComposing(e)) return
          if (e.key === 'Enter') commit()
          if (e.key === 'Escape') cancel()
        }}
        onBlur={() => {
          if (!dirty) {
            setDraft(null)
            onDismiss?.()
          }
        }}
      />
      {dirty && (
        <span class="name-acts">
          <button
            class="namebtn ok"
            title="Apply"
            onMouseDown={(e) => e.preventDefault()}
            onClick={commit}
          >
            ✓
          </button>
          <button
            class="namebtn no"
            title="Cancel"
            onMouseDown={(e) => e.preventDefault()}
            onClick={cancel}
          >
            ✕
          </button>
        </span>
      )}
    </span>
  )
}

export function LampState({ state }: { state: RuntimeState }) {
  const s = lampState(state, useVerifying())
  const style = s.colour
    ? `--lamp:${s.colour}${s.glow ? `;--lamp-glow:${s.glow}` : ''}`
    : undefined
  return (
    <span class={`lampstate ${s.cls}`} title={s.title}>
      <span class="lampdot" style={style} />
      <span class="lamplv">{s.label}</span>
      {s.qualifier && <span class="lampcq">{s.qualifier}</span>}
    </span>
  )
}

export function clampInt(raw: string, lo: number, hi: number, fallback: number): number {
  const n = Number.parseInt(raw, 10)
  return Number.isNaN(n) ? fallback : Math.min(hi, Math.max(lo, n))
}

export function Src({ ov }: { ov: ObservedValue<unknown> | undefined }) {
  if (!ov) {
    return (
      <span class="src stale">
        <span class="dot" />
        never read
      </span>
    )
  }
  if (ov.source === 'write_confirmed') {
    return (
      <span class="src written">
        <span class="dot" />
        written · {ago(ov.last_write_confirmed_ms ?? ov.last_read_ms)}
      </span>
    )
  }
  return (
    <span class="src readback">
      <span class="dot" />
      readback · {ago(ov.last_read_ms)}
    </span>
  )
}

export function ObservedRow({
  k,
  v,
  ov,
  wide = false,
  rowv = false,
}: {
  k: ComponentChildren
  v: ComponentChildren
  ov?: ObservedValue<unknown>
  wide?: boolean
  rowv?: boolean
}) {
  return (
    <div class={wide ? 'attr wide' : 'attr'}>
      <span class="k">{k}</span>
      <span class={`v${wide ? '' : ' plain'}${rowv ? ' rowv' : ''}`}>
        {v}
        {!wide && <span class="unit" />}
      </span>
      <Src ov={ov} />
    </div>
  )
}

export function AttrRow({
  k,
  v,
  plain = true,
  controls = false,
}: {
  k: ComponentChildren
  v: ComponentChildren
  plain?: boolean
  controls?: boolean
}) {
  return (
    <div class={controls ? 'attr controls' : 'attr'}>
      <span class="k">{k}</span>
      <span class={plain ? 'v plain' : 'v'}>
        {v}
        <span class="unit" />
      </span>
      <span />
    </div>
  )
}

function useSliderDraft(
  observed: number | null | undefined,
  onCommit: (value: number) => Promise<boolean>,
): { draft: SliderDraft; hold: (value: number) => void; commit: (value: number) => void } {
  const [draft, setDraft] = useState<SliderDraft>(null)
  useEffect(() => {
    const next = draftAfterPoll(draft, observed)
    if (next !== draft) setDraft(next)
  }, [observed, draft])
  const commit = (value: number) => {
    void onCommit(value).then((accepted) => {
      if (!accepted) setDraft((current) => draftAfterRefusal(current, value))
    })
  }
  return { draft, hold: setDraft, commit }
}

export function LevelSlider({
  value,
  max = LEVEL_MAX,
  mini,
  disabled,
  readout,
  onCommit,
}: {
  value: number
  max?: number
  mini?: boolean
  disabled?: boolean
  readout?: boolean
  onCommit: (level: number) => Promise<boolean>
}) {
  const { draft: local, hold, commit } = useSliderDraft(value, onCommit)
  const v = local ?? value
  const track = (
    <input
      type="range"
      class={`slider${mini ? ' mini' : ''}`}
      min={0}
      max={max}
      value={v}
      disabled={disabled}
      style={`--pct:${(v / max) * 100}%`}
      onInput={(e) => hold(Number(e.currentTarget.value))}
      onChange={(e) => commit(Number(e.currentTarget.value))}
    />
  )
  if (!readout) return track
  return (
    <span class="lvlcell">
      {track}
      <span class={`lvlnum${local !== null ? ' draft' : ''}`}>{v}</span>
    </span>
  )
}

export function CctSlider({
  kelvin,
  onCommit,
  range,
}: {
  kelvin: number | null | undefined
  onCommit: (kelvin: number) => Promise<boolean>
  range?: CctRange | null
}) {
  const { draft: local, hold, commit } = useSliderDraft(kelvin, onCommit)
  const { min, max, value: v, clamped } = cctSliderView(kelvin, local, range)
  return (
    <>
      <span class="kelvin">{v} K</span>
      <input
        type="range"
        class="slider cct"
        min={min}
        max={max}
        step={50}
        value={v}
        onInput={(e) => hold(Number(e.currentTarget.value))}
        onChange={(e) => commit(Number(e.currentTarget.value))}
      />
      <span class="cct-range" title={range ? 'reported by the fixture' : 'fixture has not reported its range yet'}>
        {min}–{max} K{range ? '' : ' (default)'}
      </span>
      {clamped !== null && (
        <span class="cct-clamped" title="the gear ran at its own limit instead">
          asked {clamped} K &middot; gear {clamped > max ? `max ${max}` : `min ${min}`} K
        </span>
      )}
    </>
  )
}

export type RgbwafChannel = 'r' | 'g' | 'b' | 'w' | 'a' | 'f'

export function RgbInputs({
  values,
  dirty,
  compact,
  channels = ['r', 'g', 'b'],
  onInput,
}: {
  values: Partial<Record<RgbwafChannel, string | number>>
  dirty?: boolean
  compact?: boolean
  channels?: readonly RgbwafChannel[]
  onInput: (channel: RgbwafChannel, raw: string) => void
}) {
  const num = (v: string | number | undefined) =>
    typeof v === 'number' ? v : Number(v ?? 0) || 0
  return (
    <>
      <span
        class="swatch"
        style={`background:rgb(${num(values.r)},${num(values.g)},${num(values.b)})`}
      />
      {channels.map((c) => (
        <input
          key={c}
          class={`rgb-in${compact ? ' sm' : ''}${dirty ? ' dirty' : ''}`}
          value={values[c] ?? 0}
          title={c.toUpperCase()}
          onInput={(e) => onInput(c, e.currentTarget.value)}
        />
      ))}
    </>
  )
}

export function SelChip({
  on,
  onClick,
  children,
}: {
  on: boolean
  onClick: () => void
  children: string
}) {
  return (
    <button type="button" class={on ? 'selchip on' : 'selchip'} onClick={onClick}>
      {children}
    </button>
  )
}

export function FieldRow({
  label,
  hint,
  stacked,
  children,
}: {
  label: string
  hint?: preact.ComponentChildren
  stacked?: boolean
  children: preact.ComponentChildren
}) {
  return (
    <div class={stacked ? 'fieldrow stacked' : 'fieldrow'}>
      <div class="fl">
        <label>{label}</label>
        {hint ? <p class="hint">{hint}</p> : null}
      </div>
      <div class="fc">{children}</div>
    </div>
  )
}

export function Switch({ on, onToggle }: { on: boolean; onToggle: () => void }) {
  return (
    <span
      class={`switch${on ? ' on' : ''}`}
      role="switch"
      aria-checked={on}
      onClick={onToggle}
    >
      <span class="knob" />
    </span>
  )
}

export function BitChips({
  value,
  bits,
}: {
  value: number
  bits: readonly (readonly [number, string] | readonly [number, string, string])[]
}) {
  return (
    <>
      {bits.map(([bit, label, tone]) => (
        <Chip key={label} cls={(value & bit) !== 0 ? tone ?? 'ok' : 'idle'}>
          {label}
        </Chip>
      ))}
      <span class="mono faint">{hex2(value)}</span>
    </>
  )
}

export function MatrixCell({
  desired,
  applied,
  disabled,
  onToggle,
}: {
  desired: boolean
  applied: boolean
  disabled?: boolean
  onToggle?: () => void
}) {
  if (disabled) return <span class="cell dis" />
  let cls = ''
  let mark: ComponentChildren = null
  if (desired && applied) {
    cls = 'on'
    mark = '✓'
  } else if (desired && !applied) {
    cls = 'add'
    mark = '✓'
  } else if (!desired && applied) {
    cls = 'rm'
    mark = <span class="x">✓</span>
  }
  return (
    <span class={`cell ${cls}`} onClick={onToggle}>
      {mark}
    </span>
  )
}
