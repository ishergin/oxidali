import { useState } from 'preact/hooks'
import { api } from '../api/client'
import type {
  AttributeGroup,
  BankReading,
  Condition,
  EnergyBank,
  LuminaireValue,
  OperationAccepted,
  Attributes,
  MemoryBankSummary,
  ObservedValue,
  PhysicalDeviceCore,
  PhysicalDevicePatch,
  TargetStateRequest,
  WriteAttributesRequest,
} from '../api/types'
import {
  AttrRow,
  Badge,
  Card,
  CctSlider,
  Chip,
  EditableName,
  LampState,
  LevelSlider,
  ObservedRow,
  RgbInputs,
  type RgbwafChannel,
  Src,
} from '../components/ui'
import { ensureProductsLoaded, productName } from '../products'
import {
  ADAPTER,
  ago,
  attrNum,
  attrOf,
  BANK_TEMPERATURE_OFFSET,
  daliVersion,
  implementedPartNumbers,
  lightDistributionLabel,
  memoryBusUnitOf,
  memoryDiagnosticsOf,
  memoryEnergyOf,
  memoryLuminaireOf,
  renderBank,
  scaleLabel,
  GROUP_COUNT,
  hex2,
  hex4,
  hex6,
  hexWide,
  LEVEL_MAX,
  lightSourceType,
  pad2,
  rgbwafChannelNames,
  rgbwafControlType,
  rgbwafDrives,
  rgbwafIsTarget,
  SCENE_COUNT,
  SCENE_MASK,
  STATUS_FLAG_LABELS,
  sourceLabel,
  typeLabel,
  registerDeviceNow,
} from '../format'
import { useLive } from '../hooks'
import { nav } from '../router'
import { errorMessage, mutate, notify, opCommitted, runOp } from '../toast'

const ALL_ATTRIBUTE_GROUPS: AttributeGroup[] = [
  'runtime_status',
  'common_102',
  'dt8_color',
  'dt6_led',
  'extended',
  'groups',
  'scenes',
]
const LEVEL_STEP = 16
const BANK1_UNLOCK_BYTE = 0x55

const TYPE_OVERRIDE_OPTIONS = ['unknown', 'dt6_led', 'dt8_color']
const TYPE_OVERRIDE_DALI_CODE: Record<string, number | undefined> = {
  unknown: undefined,
  dt6_led: 6,
  dt8_color: 8,
}

function typeOverrideOptions(
  declared: number[] | undefined,
  current: string | null | undefined,
): string[] {
  if (!declared) return TYPE_OVERRIDE_OPTIONS
  return TYPE_OVERRIDE_OPTIONS.filter((o) => {
    if (o === current) return true
    const code = TYPE_OVERRIDE_DALI_CODE[o]
    return code === undefined || declared.includes(code)
  })
}

function DeclaredTypes({ declared }: { declared: number[] | undefined }) {
  if (!declared) return <span class="types muted">not read</span>
  if (declared.length === 0) return <span class="types muted">none declared</span>
  return <span class="types">{declared.map((t) => `DT${t}`).join(' · ')}</span>
}
const COLOR_MODE_OVERRIDE_OPTIONS = ['brightness', 'cct', 'xy', 'rgb', 'unknown']

function OverrideSelect({
  value,
  options,
  onSelect,
}: {
  value: string | null | undefined
  options: string[]
  onSelect: (v: string | null) => void
}) {
  return (
    <select
      class="sel"
      value={value ?? ''}
      onChange={(e) => {
        const v = e.currentTarget.value
        onSelect(v === '' ? null : v)
      }}
    >
      <option value="">inherit</option>
      {options.map((o) => (
        <option key={o} value={o}>
          {o}
        </option>
      ))}
    </select>
  )
}

function WideValue({ v }: { v: number | null }) {
  if (v == null) return <>—</>
  return (
    <>
      {v}
      <span class="hex">{hexWide(v)}</span>
    </>
  )
}

function RawBeside({
  decoded,
  raw,
  hex,
}: {
  decoded: string | null
  raw: number | null
  hex?: boolean
}) {
  if (raw == null) return <>—</>
  return (
    <>
      {decoded ?? 'unknown'}
      <span class="raw">{hex ? hex2(raw) : raw}</span>
    </>
  )
}

function RawByte({ raw }: { raw: number | null }) {
  if (raw == null) return <>—</>
  return <span class="mono">{hex2(raw)}</span>
}

function versionPair(attrs: Attributes, section: string, prefix: string): string {
  const major = attrNum(attrs, section, `${prefix}_major`)
  const minor = attrNum(attrs, section, `${prefix}_minor`)
  if (major == null) return '—'
  return `${major}.${minor ?? 0}`
}

function BankChip({ b }: { b: MemoryBankSummary }) {
  const ranges = b.ranges.map((r) => `${r.start}–${r.start + r.length - 1}`).join(', ')
  return (
    <span class="bank-chip" title={`bytes ${ranges}`}>
      Bank {b.bank} <span class="sep">·</span> {b.total_bytes_read} B{' '}
      <span class="sep">·</span> {ago(b.last_read_ms)}
    </span>
  )
}

interface WritableField {
  field: keyof WriteAttributesRequest
  label: string
  unit?: string
  section?: string
  attrKey?: string
  current?: (dev: PhysicalDeviceCore) => number | null
  tcOnly?: boolean
  dt6Only?: boolean
  options?: { value: number; label: string }[]
}

const FADE_TIME_OPTIONS: { value: number; label: string }[] = [
  { value: 0, label: '0 — extended fade time' },
  { value: 700, label: '0.7 s' },
  { value: 1000, label: '1.0 s' },
  { value: 1400, label: '1.4 s' },
  { value: 2000, label: '2.0 s' },
  { value: 2800, label: '2.8 s' },
  { value: 4000, label: '4.0 s' },
  { value: 5700, label: '5.7 s' },
  { value: 8000, label: '8.0 s' },
  { value: 11300, label: '11.3 s' },
  { value: 16000, label: '16.0 s' },
  { value: 22600, label: '22.6 s' },
  { value: 32000, label: '32.0 s' },
  { value: 45300, label: '45.3 s' },
  { value: 64000, label: '64.0 s' },
  { value: 90500, label: '90.5 s' },
]
const DIMMING_CURVE_OPTIONS: { value: number; label: string }[] = [
  { value: 0, label: '0 — standard logarithmic' },
  { value: 1, label: '1 — linear' },
]
const mirekOf = (kelvin: number | null | undefined): number | null =>
  kelvin == null || kelvin === 0 ? null : Math.round(1_000_000 / kelvin)
const WRITABLE_FIELDS: WritableField[] = [
  { field: 'power_on_level', label: 'Power-on level' },
  { field: 'system_failure_level', label: 'System failure level' },
  { field: 'fade_time_ms', label: 'Fade time', options: FADE_TIME_OPTIONS },
  { field: 'fade_rate', label: 'Fade rate' },
  { field: 'min_level', label: 'Min arc level' },
  { field: 'max_level', label: 'Max arc level' },
  {
    field: 'extended_fade_time_ms',
    label: 'Extended fade time',
    unit: 'ms',
    section: 'extended',
    attrKey: 'fade_time_ms',
  },
  {
    field: 'dimming_curve',
    label: 'Dimming curve',
    section: 'dt6_led',
    dt6Only: true,
    options: DIMMING_CURVE_OPTIONS,
  },
  {
    field: 'tc_coolest_mirek',
    label: 'Tc limit, cool end',
    unit: 'mirek',
    tcOnly: true,
    current: (dev) => mirekOf(dev.color_temperature_range?.max_kelvin),
  },
  {
    field: 'tc_warmest_mirek',
    label: 'Tc limit, warm end',
    unit: 'mirek',
    tcOnly: true,
    current: (dev) => mirekOf(dev.color_temperature_range?.min_kelvin),
  },
]

const CONFIRM_MS = 3000

function CommissioningCard({
  short,
  knownShorts,
  absent,
  onMoved,
  reload,
}: {
  short: number
  knownShorts: number[]
  absent: boolean
  onMoved: (next: number) => void
  reload: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [newShort, setNewShort] = useState<number | null>(null)
  const [verify, setVerify] = useState(true)
  const [replacement, setReplacement] = useState<number | null>(null)
  const [armed, setArmed] = useState(false)
  const [restore, setRestore] = useState({
    metadata_and_overrides: true,
    attributes: true,
    groups: true,
    scenes: true,
  })

  const freeShorts: number[] = []
  for (let sa = 0; sa < 64; sa += 1) {
    if (!knownShorts.includes(sa)) freeShorts.push(sa)
  }
  const others = knownShorts.filter((sa) => sa !== short)
  const anyRestore = Object.values(restore).some(Boolean)

  const run = async (title: string, start: () => Promise<OperationAccepted>, after?: () => void) => {
    setBusy(true)
    const op = await runOp(title, start)
    setBusy(false)
    if (opCommitted(op)) after?.()
    reload()
    return op
  }

  const identify = () =>
    run(`Identify · SA ${pad2(short)}`, () =>
      api.commissioningIdentify(ADAPTER, { short_address: short }),
    )

  const changeAddress = async () => {
    if (newShort == null) return
    const target = newShort
    const op = await run(
      `Address change · SA ${pad2(short)} → ${pad2(target)}`,
      () =>
        api.commissioningAddressChange(ADAPTER, {
          short_address: short,
          new_short_address: target,
          verify_after_program: verify,
        }),
    )
    if (opCommitted(op)) onMoved(target)
  }

  const forget = async () => {
    if (!armed) {
      setArmed(true)
      setTimeout(() => setArmed(false), CONFIRM_MS)
      return
    }
    setArmed(false)
    setBusy(true)
    const ok = await mutate(`Forget · SA ${pad2(short)}`, () =>
      api.deletePhysicalDevice(ADAPTER, short),
    )
    setBusy(false)
    if (ok) {
      notify('Forget', 'succeeded', `SA ${pad2(short)} forgotten — a scan will re-create it empty`)
      nav('#/devices')
    }
  }

  const replace = () => {
    if (replacement == null || !anyRestore) return
    return run(
      `Replace · SA ${pad2(short)} ← ${pad2(replacement)}`,
      () =>
        api.commissioningReplacement(ADAPTER, {
          failed_short_address: short,
          replacement_short_address: replacement,
          restore,
        }),
    )
  }

  return (
    <Card title="Commissioning" span2>
      <div class="comm-grid">
        <div class="comm-act">
          <div class="comm-title">Identify</div>
          <div class="comm-row">
            <button class="btn sm" disabled={busy} onClick={identify}>
              {busy ? 'Running…' : 'Identify'}
            </button>
          </div>
          <div class="comm-hint">
            The fixture identifies itself for about 10 s. Press again to extend.
          </div>
        </div>

        <div class="comm-act">
          <div class="comm-title">Change address</div>
          <div class="comm-row">
            <span class="mono">SA {pad2(short)}</span>
            <span class="comm-arrow">→</span>
            <select
              class="sel sm"
              value={newShort == null ? '' : String(newShort)}
              disabled={busy}
              onChange={(e) => {
                const v = (e.target as HTMLSelectElement).value
                setNewShort(v === '' ? null : Number(v))
              }}
            >
              <option value="">free SA…</option>
              {freeShorts.map((sa) => (
                <option key={sa} value={String(sa)}>
                  {pad2(sa)}
                </option>
              ))}
            </select>
            <label class="comm-check">
              <input
                type="checkbox"
                checked={verify}
                disabled={busy}
                onChange={() => setVerify((v) => !v)}
              />
              Verify
            </label>
            <button class="btn sm" disabled={busy || newShort == null} onClick={changeAddress}>
              Move
            </button>
          </div>
          <div class="comm-hint">
            Only free short addresses are offered; a taken target is refused server-side.
          </div>
        </div>

        <div class="comm-act">
          <div class="comm-title">Replace</div>
          <div class="comm-row">
            <select
              class="sel sm"
              value={replacement == null ? '' : String(replacement)}
              disabled={busy}
              onChange={(e) => {
                const v = (e.target as HTMLSelectElement).value
                setReplacement(v === '' ? null : Number(v))
              }}
            >
              <option value="">replacement SA…</option>
              {others.map((sa) => (
                <option key={sa} value={String(sa)}>
                  {pad2(sa)}
                </option>
              ))}
            </select>
            <button
              class="btn sm"
              disabled={busy || replacement == null || !anyRestore}
              onClick={replace}
            >
              Hand over
            </button>
          </div>
          <div class="comm-row comm-restore">
            {(Object.keys(restore) as (keyof typeof restore)[]).map((k) => (
              <label class="comm-check" key={k}>
                <input
                  type="checkbox"
                  checked={restore[k]}
                  disabled={busy}
                  onChange={() => setRestore((r) => ({ ...r, [k]: !r[k] }))}
                />
                {k.replace(/_/g, ' ')}
              </label>
            ))}
          </div>
          <div class="comm-warn">
            Remove or power down the old gear first — the handover refuses while it
            still answers.
          </div>
        </div>

        <div class="comm-act">
          <div class="comm-title">Forget</div>
          <div class="comm-row">
            <button class={armed ? 'btn sm sure' : 'btn sm danger'} disabled={busy} onClick={forget}>
              {armed ? 'Sure?' : 'Forget device'}
            </button>
            {armed && <span class="comm-hint">Tap again to confirm</span>}
          </div>
          <div class={absent ? 'comm-hint' : 'comm-warn'}>
            The controller drops its record: name, notes, overrides, attributes and
            bank data. A bound virtual lamp survives but becomes unbound.
            {absent
              ? ' This device is not answering, so nothing is lost that the wire could still give back.'
              : ' This device still answers on the wire — the next scan re-creates it blank, and the name and the binding do not come back.'}
          </div>
        </div>
      </div>
    </Card>
  )
}

const DEVICE_TABS = [
  { id: 'overview', label: 'Overview', sections: ['common_102'] },
  {
    id: 'levels',
    label: 'Levels & fade',
    sections: ['common_102', 'dt6_led', 'dt8_color', 'extended'],
  },
  { id: 'scenes', label: 'Groups & scenes', sections: ['common_102', 'groups', 'scenes'] },
  {
    id: 'identity',
    label: 'Identity',
    hint: 'banks 0 · 1',
    sections: [
      'common_102',
      'memory_identity',
      'memory_profile',
      'memory_bus_unit',
      'memory_luminaire',
    ],
  },
  { id: 'energy', label: 'Energy', hint: 'banks 202–204', sections: ['common_102', 'memory_energy'] },
  {
    id: 'diagnostics',
    label: 'Diagnostics',
    hint: 'banks 205–207',
    sections: ['common_102', 'memory_diagnostics'],
  },
] as const satisfies readonly {
  id: string
  label: string
  hint?: string
  sections: readonly string[]
}[]

type DeviceTab = (typeof DEVICE_TABS)[number]['id']

const DEFAULT_TAB: DeviceTab = 'overview'

function tabOf(raw: string | undefined): DeviceTab {
  const hit = DEVICE_TABS.find((t) => t.id === raw)
  return hit ? hit.id : DEFAULT_TAB
}

export function DeviceDetail({ short, tab: rawTab }: { short: number; tab?: string }) {
  const tab = tabOf(rawTab)
  const sections = DEVICE_TABS.find((t) => t.id === tab)!.sections
  const { data: siblings } = useLive(
    () => api.physicalDevices(ADAPTER),
    ['physical_devices'],
    { intervalMs: 10000, deps: [short] },
  )
  const { data: dev, reload: reloadCore } = useLive(
    async () => {
      const d = await api.physicalDevice(ADAPTER, short)
      registerDeviceNow(d.now_ms)
      return d
    },
    ['physical_devices', 'virtual_lamps'],
    { intervalMs: 5000, deps: [short] },
  )
  const { data: attrData, reload: reloadAttrs } = useLive(
    () => api.physicalDeviceAttributes(ADAPTER, short, sections),
    ['physical_devices'],
    { intervalMs: 5000, deps: [short, tab] },
  )
  const { data: bankData, reload: reloadBanks } = useLive(
    () =>
      tab === 'identity'
        ? api.physicalDeviceMemoryBanks(ADAPTER, short)
        : Promise.resolve(null),
    ['physical_devices'],
    { intervalMs: 5000, deps: [short, tab] },
  )
  const reload = () => {
    void reloadCore()
    void reloadAttrs()
    void reloadBanks()
  }
  const [edits, setEdits] = useState<Record<string, string>>({})
  const [rgbDraft, setRgbDraft] = useState<Record<RgbwafChannel, string> | null>(null)

  if (!dev) return <div class="empty">Loading device SA {pad2(short)}…</div>

  const attrs: Attributes = attrData?.attributes ?? {}
  const memoryBanks = bankData?.memory_banks ?? []

  const t = typeLabel(dev.device_type_effective, dev.color_mode_effective, dev.capabilities)
  const seen = dev.state.last_seen_ms
  const absent = dev.state.error?.code === 'device_absent'

  const setTarget = async (body: TargetStateRequest) => {
    await mutate('Set target state', () => api.deviceTargetState(ADAPTER, short, body), () => {
      void reload()
    })
  }

  const level = dev.state.level ?? 0
  ensureProductsLoaded()
  const productLabel = productName(attrNum(attrs, 'memory_identity', 'gtin'))
  const DT6_FAILURES: [field: string, label: string][] = [
    ['short_circuit', 'short circuit'],
    ['open_circuit', 'open circuit'],
    ['thermal_shutdown', 'thermal shutdown'],
    ['thermal_overload', 'thermal overload'],
    ['reference_measurement_failed', 'reference measurement failed'],
  ]
  const dt6Failures = DT6_FAILURES.filter(([f]) => (attrNum(attrs, 'dt6_led', f) ?? 0) !== 0).map(
    ([, label]) => label,
  )
  const dt6FailuresRead = DT6_FAILURES.some(([f]) => attrNum(attrs, 'dt6_led', f) != null)
  const dt6Any = Object.keys(attrs.dt6_led ?? {}).length > 0
  const c102Version = attrNum(attrs, 'common_102', 'version')
  const lightSource = attrNum(attrs, 'common_102', 'light_source_type')
  const lightSourcePacked = attrNum(attrs, 'common_102', 'light_source_types')
  const minLevel = attrNum(attrs, 'common_102', 'min_level') ?? 1
  const maxLevel = attrNum(attrs, 'common_102', 'max_level') ?? LEVEL_MAX
  const nudge = (delta: number) =>
    setTarget({ power: 'on', level: Math.min(LEVEL_MAX, Math.max(0, level + delta)) })

  const saveName = async (name: string) => {
    if (name === dev.name) return
    await mutate('Rename', () => api.patchPhysicalDevice(ADAPTER, short, { name }), () => {
      void reload()
    })
  }

  const patchDevice = async (title: string, body: PhysicalDevicePatch) => {
    try {
      await api.patchPhysicalDevice(ADAPTER, short, body)
      void reload()
    } catch (e) {
      notify(title, 'failed', errorMessage(e))
    }
  }

  const readAttributes = async () => {
    await runOp(`Attribute read · SA ${pad2(short)}`, () =>
      api.attributeReads(ADAPTER, short, { attribute_groups: ALL_ATTRIBUTE_GROUPS }),
    )
    void reload()
  }

  const readMemoryBanks = async () => {
    await runOp(`Memory banks · SA ${pad2(short)}`, () =>
      api.attributeReads(ADAPTER, short, {
        attribute_groups: ['common_102'],
        memory_banks: 'all',
      }),
    )
    void reload()
  }

  const editedValue = (field: string): string | undefined => edits[field]
  const attrValueStr = (wf: WritableField): string => {
    const v = wf.current
      ? wf.current(dev)
      : attrNum(attrs, wf.section ?? 'common_102', wf.attrKey ?? wf.field)
    return v == null ? '' : String(v)
  }
  const writableFields = WRITABLE_FIELDS.filter(
    (wf) =>
      (!wf.tcOnly || dev.capabilities.cct) &&
      (!wf.dt6Only || (dev.supported_device_types?.includes(6) ?? false)),
  )
  const changedFields = writableFields.filter((wf) => {
    const e = editedValue(wf.field)
    return e !== undefined && e !== attrValueStr(wf) && e !== ''
  })

  const writeChanges = async () => {
    const body: WriteAttributesRequest = {}
    for (const { field } of changedFields) {
      const n = Number(edits[field])
      if (!Number.isFinite(n)) {
        notify('Write attributes', 'failed', `${field}: not a number`)
        return
      }
      body[field] = n
    }
    if (
      body.tc_coolest_mirek != null &&
      body.tc_warmest_mirek != null &&
      body.tc_coolest_mirek > body.tc_warmest_mirek
    ) {
      notify('Write attributes', 'failed', 'Tc limits: cool end above warm end')
      return
    }
    if (body.min_level != null && body.max_level != null && body.min_level > body.max_level) {
      notify('Write attributes', 'failed', 'Arc levels: min above max')
      return
    }
    const op = await runOp(`Attribute write · SA ${pad2(short)}`, () =>
      api.writeAttributes(ADAPTER, short, body),
    )
    if (opCommitted(op)) setEdits({})
    void reload()
  }

  const membership = attrNum(attrs, 'groups', 'membership')

  const gearFeatures = attrNum(attrs, 'dt8_color', 'gear_features')
  const autoActivation = gearFeatures != null && (gearFeatures & 0x01) !== 0

  const rgbwafControl = attrNum(attrs, 'dt8_color', 'rgbwaf_control')
  const rgbwafLinked = rgbwafControl == null ? 0 : rgbwafControl & 0x3f

  const rgb = dev.state.rgb
  const waf = dev.state.waf
  const sixChannel = dev.capabilities.rgbwaf
  const rgbChannels = sixChannel
    ? (['r', 'g', 'b', 'w', 'a', 'f'] as const)
    : (['r', 'g', 'b'] as const)
  const rgbValue = rgbDraft ?? {
    r: String(rgb?.r ?? 0),
    g: String(rgb?.g ?? 0),
    b: String(rgb?.b ?? 0),
    w: String(waf?.w ?? 0),
    a: String(waf?.a ?? 0),
    f: String(waf?.f ?? 0),
  }
  const applyRgb = () => {
    const values = rgbChannels.map((c) => Number(rgbValue[c]))
    if (values.some((v) => !Number.isFinite(v) || v < 0 || v > 255)) {
      notify('Set RGB', 'failed', 'components must be 0–255')
      return
    }
    setRgbDraft(null)
    const [r, g, b, w, a, f] = values
    void setTarget(
      sixChannel
        ? { color_mode: 'rgbwaf', rgbwaf: { r, g, b, w, a, f } }
        : { color_mode: 'rgb', rgb: { r, g, b } },
    )
  }

  return (
    <>
      <div class="crumbs">
        <a href="#/">Adapter {ADAPTER}</a> / <a href="#/devices">Physical devices</a> / SA{' '}
        {pad2(short)}
      </div>

      <div class="head">
        <span class="h1-wrap" title="Click to rename">
          <EditableName
            cls={`h1-edit${dev.name ? '' : ' name-faint'}`}
            value={dev.name}
            placeholder="— unnamed"
            onCommit={(v) => void saveName(v)}
          />
          <span class="pencil">✎</span>
        </span>
        <Badge cls="addr">SA {pad2(short)}</Badge>
        <Badge cls={t.dt8 ? 'dt8' : undefined}>{t.label}</Badge>
        {seen != null && !absent && <Chip cls="ok">Present</Chip>}
        <LampState state={dev.state} />
        <span class="spacer" />
        <button class="btn" onClick={readMemoryBanks}>
          ↻ Read memory banks
        </button>
        <button class="btn primary" onClick={readAttributes}>
          ↻ Read attributes
        </button>
      </div>

      <div class="strip">
        <div class="level-num">
          {level}
          <span class="max"> /{LEVEL_MAX}</span>
        </div>
        <LevelSlider
          value={level}
          onCommit={(lv) => setTarget(lv === 0 ? { power: 'off' } : { power: 'on', level: lv })}
        />
        <div class="btns">
          <button class="btn" onClick={() => setTarget({ power: 'off' })}>
            Off
          </button>
          <button class="btn icon" onClick={() => nudge(-LEVEL_STEP)}>
            −
          </button>
          <button class="btn icon" onClick={() => nudge(LEVEL_STEP)}>
            +
          </button>
          <button class="btn" onClick={() => setTarget({ power: 'on', level: minLevel })}>
            Min
          </button>
          <button class="btn primary" onClick={() => setTarget({ power: 'on', level: maxLevel })}>
            Max
          </button>
        </div>
        {dev.capabilities.cct && (
          <div class="color-block">
            <CctSlider
              range={dev.color_temperature_range}
              kelvin={dev.state.color_temperature_kelvin}
              onCommit={(k) => setTarget({ color_mode: 'cct', color_temperature_kelvin: k })}
            />
          </div>
        )}
        {dev.capabilities.rgb && (
          <div class="color-block">
            <RgbInputs
              values={rgbValue}
              channels={rgbChannels}
              onInput={(c, raw) => setRgbDraft({ ...rgbValue, [c]: raw })}
            />
            <button class="btn" onClick={applyRgb}>
              Set
            </button>
          </div>
        )}
      </div>

      <div class="ptabs">
        {DEVICE_TABS.map((entry) => (
          <a
            key={entry.id}
            class={`ptab${entry.id === tab ? ' active' : ''}`}
            href={`#/devices/${short}${entry.id === DEFAULT_TAB ? '' : `/${entry.id}`}`}
          >
            {entry.label}
            {'hint' in entry && <span class="hint">{entry.hint}</span>}
          </a>
        ))}
      </div>
      <div class="ptabline" />

      <div class="grid2">
        {tab === 'overview' && (
          <>
        <Card title="Information">
          <AttrRow k="Name" v={dev.name || '—'} />
          <AttrRow
            controls
            k="Device type override"
            v={
              <span class="ovr">
                <OverrideSelect
                  value={dev.device_type_override}
                  options={typeOverrideOptions(
                    dev.supported_device_types,
                    dev.device_type_override,
                  )}
                  onSelect={(v) =>
                    void patchDevice('Device type override', { device_type_override: v })
                  }
                />
                <Badge cls={dev.device_type_source !== 'discovered' ? 'declared' : undefined}>
                  {dev.device_type_effective}
                  <span class="prov">· {sourceLabel(dev.device_type_source)}</span>
                </Badge>
              </span>
            }
          />
          <AttrRow k="Declared types" v={<DeclaredTypes declared={dev.supported_device_types} />} />
          <AttrRow
            controls
            k="Color mode override"
            v={
              <span class="ovr">
                <OverrideSelect
                  value={dev.color_mode_override}
                  options={COLOR_MODE_OVERRIDE_OPTIONS}
                  onSelect={(v) =>
                    void patchDevice('Color mode override', { color_mode_override: v })
                  }
                />
                <Badge cls={dev.color_mode_source !== 'discovered' ? 'declared' : undefined}>
                  {dev.color_mode_effective}
                  <span class="prov">· {sourceLabel(dev.color_mode_source)}</span>
                </Badge>
              </span>
            }
          />
          <AttrRow
            k="Random address"
            v={dev.random_address != null ? hex6(dev.random_address) : '—'}
          />
          {productLabel && <AttrRow k="Product" v={productLabel} />}
          <ObservedRow
            k="Version"
            v={<RawBeside decoded={daliVersion(c102Version)} raw={c102Version} hex />}
            ov={attrOf(attrs, 'common_102', 'version')}
          />
          <ObservedRow
            k="Light source type"
            v={
              <RawBeside
                decoded={lightSourceType(lightSource, lightSourcePacked)}
                raw={lightSource}
              />
            }
            ov={attrOf(attrs, 'common_102', 'light_source_type')}
          />
        </Card>

        <Card title="Status">
          <div class="flags">
            {STATUS_FLAG_LABELS.map(([key, label, cls]) => {
              const raised =
                dev.state.status != null &&
                (dev.state.status as unknown as Record<string, boolean>)[key]
              return (
                <Chip key={key} cls={raised ? cls : 'idle'}>
                  {label}
                </Chip>
              )
            })}
          </div>
          <AttrRow k="Last seen" v={seen == null ? 'never' : ago(seen)} />
          <AttrRow k="Last DAPC source" v={dev.state.last_dapc_source ?? '—'} />
          <AttrRow
            k="Status raw"
            v={dev.state.status ? hex2(dev.state.status.raw) : '—'}
          />
        </Card>
          </>
        )}

        {tab === 'levels' && (
          <>
        {dt6Any && (
          <Card
            title="LED driver · Part 207"
            span2
            action={<Badge>DT6</Badge>}
          >
            <ObservedRow
              k="Failure status"
              wide
              rowv
              v={
                !dt6FailuresRead ? (
                  <Chip cls="idle">not read</Chip>
                ) : dt6Failures.length === 0 ? (
                  <Chip cls="ok">no failure reported</Chip>
                ) : (
                  <>
                    {dt6Failures.map((f) => (
                      <Chip key={f} cls="err">
                        {f}
                      </Chip>
                    ))}
                  </>
                )
              }
              ov={attrOf(attrs, 'dt6_led', 'failure_status')}
            />
            <ObservedRow
              k="Failure status byte"
              v={<RawByte raw={attrNum(attrs, 'dt6_led', 'failure_status')} />}
              ov={attrOf(attrs, 'dt6_led', 'failure_status')}
            />
            <ObservedRow
              k="Reference measurement"
              wide
              rowv
              v={
                attrNum(attrs, 'dt6_led', 'reference_running') ? (
                  <Chip cls="info">running</Chip>
                ) : attrNum(attrs, 'dt6_led', 'reference_measurement_failed') ? (
                  <Chip cls="err">failed</Chip>
                ) : (
                  <Chip cls="idle">idle</Chip>
                )
              }
              ov={attrOf(attrs, 'dt6_led', 'reference_running')}
            />
            <ObservedRow
              k="Current protector"
              wide
              rowv
              v={
                <>
                  <Chip cls={attrNum(attrs, 'dt6_led', 'current_protector_enabled') ? 'ok' : 'idle'}>
                    {attrNum(attrs, 'dt6_led', 'current_protector_enabled') ? 'enabled' : 'disabled'}
                  </Chip>
                  {attrNum(attrs, 'dt6_led', 'current_protector_active') ? (
                    <Chip cls="warn">active</Chip>
                  ) : null}
                </>
              }
              ov={attrOf(attrs, 'dt6_led', 'current_protector_enabled')}
            />
            <ObservedRow
              k="Gear type"
              v={<RawByte raw={attrNum(attrs, 'dt6_led', 'gear_type')} />}
              ov={attrOf(attrs, 'dt6_led', 'gear_type')}
            />
            <ObservedRow
              k="Operating mode"
              v={<RawByte raw={attrNum(attrs, 'dt6_led', 'operating_mode')} />}
              ov={attrOf(attrs, 'dt6_led', 'operating_mode')}
            />
            <ObservedRow
              k="Possible operating modes"
              v={<RawByte raw={attrNum(attrs, 'dt6_led', 'possible_operating_mode')} />}
              ov={attrOf(attrs, 'dt6_led', 'possible_operating_mode')}
            />
            <ObservedRow
              k="Features"
              v={<RawByte raw={attrNum(attrs, 'dt6_led', 'features')} />}
              ov={attrOf(attrs, 'dt6_led', 'features')}
            />
            <ObservedRow
              k="Load increase / decrease"
              v={
                <>
                  {attrNum(attrs, 'dt6_led', 'load_increase') ?? '—'} /{' '}
                  {attrNum(attrs, 'dt6_led', 'load_decrease') ?? '—'}
                </>
              }
              ov={attrOf(attrs, 'dt6_led', 'load_increase')}
            />
            <ObservedRow
              k="Fast fade time"
              v={
                <>
                  {attrNum(attrs, 'dt6_led', 'fast_fade_time') ?? '—'}
                  <span class="raw">
                    min {attrNum(attrs, 'dt6_led', 'min_fast_fade_time') ?? '—'}
                  </span>
                </>
              }
              ov={attrOf(attrs, 'dt6_led', 'fast_fade_time')}
            />
            <ObservedRow
              k="Extended version"
              v={<RawByte raw={attrNum(attrs, 'dt6_led', 'extended_version_number')} />}
              ov={attrOf(attrs, 'dt6_led', 'extended_version_number')}
            />
          </Card>
        )}
        <Card
          title="Fading & arc levels"
          action={
            <button class="act" onClick={writeChanges} disabled={changedFields.length === 0}>
              Write changes{changedFields.length > 0 ? ` (${changedFields.length})` : ''}
            </button>
          }
        >
          {writableFields.map((wf) => {
            const { field, label, unit } = wf
            const ov = wf.current
              ? undefined
              : attrOf(attrs, wf.section ?? 'common_102', wf.attrKey ?? field)
            const server = attrValueStr(wf)
            const dormant =
              field === 'extended_fade_time_ms' &&
              (attrNum(attrs, 'common_102', 'fade_time_ms') ?? 0) > 0
            const shown = editedValue(field) ?? server
            const dirty = editedValue(field) !== undefined && editedValue(field) !== server
            return (
              <div class="attr" key={field}>
                <span class="k">{label}</span>
                <span class="v">
                  {wf.options ? (
                    <select
                      class={`sel attr-enum${dirty ? ' dirty' : ''}`}
                      value={shown}
                      onChange={(e) =>
                        setEdits({ ...edits, [field]: (e.target as HTMLSelectElement).value })
                      }
                    >
                      {shown === '' && <option value="">— not read</option>}
                      {shown !== '' && !wf.options.some((o) => String(o.value) === shown) && (
                        <option value={shown}>{shown} — not a defined value</option>
                      )}
                      {wf.options.map((o) => (
                        <option key={o.value} value={String(o.value)}>
                          {o.label}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      class={dirty ? 'dirty' : undefined}
                      value={shown}
                      onFocus={(e) => {
                        if (editedValue(field) === undefined)
                          setEdits({ ...edits, [field]: e.currentTarget.value })
                      }}
                      onInput={(e) => setEdits({ ...edits, [field]: e.currentTarget.value })}
                    />
                  )}
                  <span class="unit">{unit ?? ''}</span>
                </span>
                <Src ov={ov} />
                {dormant && (
                  <span
                    class="note"
                    title="IEC 62386-102: extended fade time applies only while fade time = 0"
                  >
                    dormant while fade time ≠ 0
                  </span>
                )}
                {field === 'tc_warmest_mirek' && (
                  <span
                    class="note"
                    title="IEC 62386-209 §9.13: an actual Tc left outside the new limits is set to the boundary immediately, without fading"
                  >
                    a write under the active Tc snaps the light, no fade
                  </span>
                )}
                {field === 'min_level' && (
                  <span
                    class="note"
                    title="IEC 62386-102 §9.6: the gear clamps min into [physical minimum, max level] and mirrors back what it accepted"
                  >
                    clamped into [physical min, max]; accepted value mirrors back
                  </span>
                )}
              </div>
            )
          })}
          <ObservedRow
            k={
              <>
                Physical minimum
              </>
            }
            v={attrNum(attrs, 'common_102', 'physical_minimum') ?? '—'}
            ov={attrOf(attrs, 'common_102', 'physical_minimum')}
          />
        </Card>

        {gearFeatures != null && (
          <Card
            title="DT8 colour engine"
            action={<Badge>{hex2(gearFeatures)}</Badge>}
          >
            <ObservedRow
              k="Automatic activation"
              wide
              rowv
              v={
                <Chip cls={autoActivation ? 'ok' : 'warn'}>
                  {autoActivation ? 'on' : 'off'}
                </Chip>
              }
              ov={attrOf(attrs, 'dt8_color', 'gear_features')}
            />
            <AttrRow
              k="Auto calibration"
              v={gearFeatures & 0x40 ? 'supported' : 'not supported'}
            />
            <AttrRow
              k="Auto calibration recovery"
              v={gearFeatures & 0x80 ? 'supported' : 'not supported'}
            />
            <AttrRow
              controls
              k="Restore auto activation"
              v={
                <label class="comm-check">
                  <input
                    type="checkbox"
                    checked={dev.dt8_auto_activation_repair}
                    onChange={(e) =>
                      void patchDevice('Restore auto activation', {
                        dt8_auto_activation_repair: (e.currentTarget as HTMLInputElement).checked,
                      })
                    }
                  />{' '}
                  allowed
                </label>
              }
            />
            {!autoActivation && (
              <div class="comm-warn">
                Colour writes will not reach this gear until the bit is set. Clear the box
                only for a fixture that stores the byte permanently.
              </div>
            )}
            <div class="comm-hint">
              When a read finds the bit clear, set it back before the next colour write.
            </div>
          </Card>
        )}

        {rgbwafControl != null && (
          <Card
            title="RGBWAF channel control"
            action={<Badge>{hex2(rgbwafControl)}</Badge>}
          >
            <ObservedRow
              k="Control type"
              wide
              rowv
              v={
                <Chip cls={rgbwafDrives(rgbwafControl) ? 'ok' : 'warn'}>
                  {rgbwafControlType(rgbwafControl)}
                </Chip>
              }
              ov={attrOf(attrs, 'dt8_color', 'rgbwaf_control')}
            />
            <AttrRow
              k="Linked channels"
              v={rgbwafLinked ? rgbwafChannelNames(rgbwafLinked) : 'none — levels drive the colour'}
            />
            <AttrRow
              controls
              k="Assert on colour write"
              v={
                <label class="comm-check">
                  <input
                    type="checkbox"
                    checked={dev.dt8_rgbwaf_control_assert}
                    onChange={(e) =>
                      void patchDevice('Assert RGBWAF control', {
                        dt8_rgbwaf_control_assert: (e.currentTarget as HTMLInputElement)
                          .checked,
                      })
                    }
                  />{' '}
                  allowed
                </label>
              }
            />
            {rgbwafLinked !== 0 && (
              <div class="comm-warn">
                Linked channels ignore the colour levels: a colour write here is accepted
                and changes nothing. The next colour write will set normalised control.
              </div>
            )}
            {rgbwafLinked === 0 && !rgbwafIsTarget(rgbwafControl) && (
              <div class="comm-hint">
                Colour works here: the levels are unlinked. While the box is ticked, the next
                colour write switches the gear to normalised colour control; clear it to keep
                the gear's own control type.
              </div>
            )}
          </Card>
        )}
          </>
        )}

        {tab === 'scenes' && (
          <>
        <Card
          title="Group membership"
          action={
            <a class="act" href="#/groups" style="text-decoration:none">
              Open matrix →
            </a>
          }
        >
          <div class="gbits">
            {Array.from({ length: GROUP_COUNT }, (_, g) => (
              <span
                key={g}
                class={`gbit${membership != null && membership & (1 << g) ? ' on' : ''}`}
              >
                {g}
              </span>
            ))}
          </div>
          <ObservedRow
            k="Membership raw"
            v={membership == null ? '—' : hex4(membership)}
            ov={attrOf(attrs, 'groups', 'membership')}
          />
        </Card>

        <Card
          title="Scene arc levels"
          span2
          action={
            <a class="act" href="#/scenes" style="text-decoration:none">
              Open scene editor →
            </a>
          }
        >
          <div class="scenes-grid">
            {Array.from({ length: SCENE_COUNT }, (_, s) => {
              const v = attrNum(attrs, 'scenes', `scene_${s}`)
              const unset = v == null || v === SCENE_MASK
              return (
                <div key={s} class={`scene-tile${unset ? ' unset' : ''}`}>
                  <div class="sn">S{s}</div>
                  <div class="sv">{unset ? '—' : v}</div>
                </div>
              )
            })}
          </div>
        </Card>
          </>
        )}

        {tab === 'identity' && (
          <>
        <Card
          title="Identity & memory banks"
          span2
          action={
            <button class="act" onClick={readMemoryBanks}>
              ↻ Read memory banks
            </button>
          }
        >
          {Object.keys(attrs.memory_identity ?? {}).length === 0 ? (
            <div class="empty">Not read yet — press Read memory banks.</div>
          ) : (
            <>
              <ObservedRow
                wide
                k="GTIN"
                v={<WideValue v={attrNum(attrs, 'memory_identity', 'gtin')} />}
                ov={attrOf(attrs, 'memory_identity', 'gtin')}
              />
              <ObservedRow
                wide
                k="Serial number"
                v={
                  <WideValue
                    v={attrNum(attrs, 'memory_identity', 'identification_number')}
                  />
                }
                ov={attrOf(attrs, 'memory_identity', 'identification_number')}
              />
              <ObservedRow
                k="Firmware / hardware version"
                v={`${versionPair(attrs, 'memory_identity', 'firmware_version')} / ${versionPair(attrs, 'memory_identity', 'hardware_version')}`}
                ov={attrOf(attrs, 'memory_identity', 'firmware_version_major')}
              />
              <ObservedRow
                k="DALI part versions (101 / 102 / 103)"
                v={`${daliVersion(attrNum(attrs, 'memory_identity', 'dali_101_version'))} / ${daliVersion(attrNum(attrs, 'memory_identity', 'dali_102_version'))} / ${daliVersion(attrNum(attrs, 'memory_identity', 'dali_103_version'))}`}
                ov={attrOf(attrs, 'memory_identity', 'dali_101_version')}
              />
              <ObservedRow
                k="Logical units (device / gear / gear index)"
                v={`${attrNum(attrs, 'memory_identity', 'logical_control_device_units') ?? '—'} / ${attrNum(attrs, 'memory_identity', 'logical_control_gear_units') ?? '—'} / ${attrNum(attrs, 'memory_identity', 'logical_control_gear_index') ?? '—'}`}
                ov={attrOf(attrs, 'memory_identity', 'logical_control_gear_units')}
              />
              <ObservedRow
                k="Last accessible memory bank"
                v={attrNum(attrs, 'memory_identity', 'last_memory_bank') ?? '—'}
                ov={attrOf(attrs, 'memory_identity', 'last_memory_bank')}
              />
              <ObservedRow
                wide
                k="OEM GTIN"
                v={<WideValue v={attrNum(attrs, 'memory_profile', 'oem_gtin')} />}
                ov={attrOf(attrs, 'memory_profile', 'oem_gtin')}
              />
              <ObservedRow
                wide
                k="OEM serial number"
                v={
                  <WideValue
                    v={attrNum(attrs, 'memory_profile', 'oem_identification_number')}
                  />
                }
                ov={attrOf(attrs, 'memory_profile', 'oem_identification_number')}
              />
              <ObservedRow
                wide
                rowv
                k="Bank 1 lock byte"
                v={(() => {
                  const lock = attrNum(attrs, 'memory_profile', 'bank1_lock_byte')
                  if (lock == null) return '—'
                  return (
                    <>
                      {hex2(lock)}{' '}
                      {lock === BANK1_UNLOCK_BYTE ? (
                        <Chip cls="ok">unlocked</Chip>
                      ) : (
                        <Chip cls="warn">locked</Chip>
                      )}
                    </>
                  )
                })()}
                ov={attrOf(attrs, 'memory_profile', 'bank1_lock_byte')}
              />
              <BusUnitRows attributes={attrs} />
              <LuminaireStringRows attributes={attrs} />
              {memoryBanks.length > 0 && (
                <div class="attr">
                  <span class="k">Banks read</span>
                  <span class="banks">
                    {memoryBanks.map((b) => (
                      <BankChip key={b.bank} b={b} />
                    ))}
                  </span>
                  <span />
                </div>
              )}
            </>
          )}
        </Card>

        <LuminaireDataCard attributes={attrs} />

          </>
        )}

        {tab === 'energy' && <EnergyCards attributes={attrs} now={dev.now_ms} />}

        {tab === 'diagnostics' && <DiagnosticsCards attributes={attrs} now={dev.now_ms} />}

        {tab === 'overview' && (
          <CommissioningCard
            short={short}
            knownShorts={(siblings?.physical_devices ?? []).map((d) => d.short_address)}
            absent={absent}
            onMoved={(next) => nav(`/devices/${next}`)}
            reload={reload}
          />
        )}
      </div>
    </>
  )
}

function BankValueCell({
  ov,
  now,
  format,
  scale,
}: {
  ov: ObservedValue<BankReading> | undefined
  now: number
  format: (raw: number) => string
  scale?: string
}) {
  const r = renderBank(ov, now, format)
  if (r.kind === 'value') {
    return (
      <span class="v plain">
        {r.text}
        {r.saturated ? <span class="chip warn">saturated</span> : null}
        {scale ? <span class="scale">{scale}</span> : null}
      </span>
    )
  }
  if (r.kind === 'not_implemented') {
    return (
      <span class="v rowv">
        <span class="chip muted">not implemented</span>
      </span>
    )
  }
  if (r.kind === 'tmask') {
    return (
      <span class="v rowv">
        <span class={`chip tmask${r.stale ? ' stale' : ''}`}>
          {r.stale ? 'unavailable > 30 s' : 'temporarily unavailable'}
        </span>
      </span>
    )
  }
  return <span class="v plain">—</span>
}

function BankRow({
  label,
  ov,
  now,
  format,
  scale,
}: {
  label: string
  ov: ObservedValue<BankReading> | undefined
  now: number
  format: (raw: number) => string
  scale?: string
}) {
  return (
    <div class="attr">
      <span class="k">{label}</span>
      <BankValueCell ov={ov} now={now} format={format} scale={scale} />
      <Src ov={ov} />
    </div>
  )
}

const plain = (n: number) => n.toLocaleString()

function BusUnitRows({ attributes }: { attributes: Attributes }) {
  const bus = memoryBusUnitOf(attributes)
  if (!bus) return null
  const parts = bus.implemented_parts
  return (
    <>
      {bus.configuration && (
        <ObservedRow
          wide
          rowv
          k="Bus unit configuration"
          v={
            <>
              {bus.configuration.value.raw}{' '}
              <Chip cls="muted">
                {bus.configuration.value.class}
                {bus.configuration.value.emergency_type
                  ? ` ${bus.configuration.value.emergency_type}`
                  : ''}
              </Chip>
            </>
          }
          ov={bus.configuration}
        />
      )}
      {parts && (
        <ObservedRow
          wide
          k="Implemented parts (15x)"
          v={
            <>
              {implementedPartNumbers(parts.value).map((p: number) => `Part ${p}`).join(', ') || 'none'}
              <span class="hex">
                {hex2(parts.value.raw)} · {parts.value.bytes * 8} bits read
              </span>
            </>
          }
          ov={parts}
        />
      )}
    </>
  )
}

function LuminaireStringRows({ attributes }: { attributes: Attributes }) {
  const lum = memoryLuminaireOf(attributes)
  if (!lum) return null
  const rows: Array<[string, ObservedValue<string> | undefined]> = [
    ['Luminaire identification', lum.luminaire_identification],
    ['Luminaire colour', lum.luminaire_colour],
    ['Light distribution', lum.light_distribution],
    ['OEM name', lum.oem_name],
    ['Customer stocking number', lum.customer_stocking_number],
    ['Free-use characters', lum.free_use],
  ]
  return (
    <>
      {rows.map(([label, ov]) =>
        ov === undefined ? null : (
          <div class="attr text" key={label}>
            <span class="k">{label}</span>
            <Src ov={ov} />
            <span class="v">{ov.value === '' ? '—' : ov.value}</span>
          </div>
        ),
      )}
    </>
  )
}

function LuminaireDataCard({ attributes }: { attributes: Attributes }) {
  const lum = memoryLuminaireOf(attributes)
  const format = lum?.content_format_id
  if (!lum || !format) return null
  if (format.value !== 3 && format.value !== 4 && format.value !== 5) {
    return (
      <Card title="Luminaire data" span2 action={<span class="badge">bank 1</span>}>
        <AttrRow
          k="Content format"
          v={
            <>
              {format.value} <Chip cls="muted">not DiiA Part 251</Chip>
            </>
          }
        />
        <div class="attr">
          <span class="k hint">
            Bytes above 0x10 are manufacturer-specific (IEC 62386-102 Table 10).
          </span>
          <span />
          <span />
        </div>
      </Card>
    )
  }
  return (
    <Card title="Luminaire data" span2 action={<span class="badge">bank 1 · Part 251</span>}>
      <AttrRow k="Content format" v={String(format.value)} />
      <LuminaireNumberRow k="Manufactured (year)" ov={lum.year} render={(y) => `20${String(y).padStart(2, '0')}`} />
      <LuminaireNumberRow k="Manufactured (week)" ov={lum.week} render={(w) => `W${String(w).padStart(2, '0')}`} />
      <LuminaireNumberRow k="Nominal input power" ov={lum.nominal_input_power_w} unit="W" />
      <LuminaireNumberRow k="Power at minimum" ov={lum.power_at_minimum_w} unit="W" />
      <LuminaireNumberRow k="Nominal minimum mains voltage" ov={lum.nominal_min_ac_voltage_v} unit="V" />
      <LuminaireNumberRow k="Nominal maximum mains voltage" ov={lum.nominal_max_ac_voltage_v} unit="V" />
      <LuminaireNumberRow k="Nominal light output" ov={lum.nominal_light_output_lm} unit="lm" />
      <LuminaireNumberRow k="CRI" ov={lum.cri} />
      <CctRow ov={lum.cct_kelvin} />
      <LuminaireNumberRow
        k="Light distribution type"
        ov={lum.light_distribution_type}
        render={(v) => lightDistributionLabel(v) ?? String(v)}
      />
      <LuminaireNumberRow k="Lamp current" ov={lum.lamp_current_ma} unit="mA" />
    </Card>
  )
}

function LuminaireNumberRow({
  k,
  ov,
  unit,
  render,
}: {
  k: string
  ov?: ObservedValue<LuminaireValue>
  unit?: string
  render?: (v: number) => string
}) {
  if (!ov) return null
  const v = ov.value.value
  return (
    <div class="attr">
      <span class="k">{k}</span>
      {v === null ? (
        <span class="v rowv">
          <Chip cls="unknown">unknown</Chip>
        </span>
      ) : (
        <span class="v plain">
          {render ? render(v) : plain(v)}
          <span class="unit">{unit ?? ''}</span>
        </span>
      )}
      <Src ov={ov} />
    </div>
  )
}

function CctRow({ ov }: { ov?: ObservedValue<LuminaireValue> }) {
  if (!ov) return null
  if (ov.value.part209_implemented) {
    return (
      <div class="attr">
        <span class="k">CCT</span>
        <span class="v rowv">
          <Chip cls="muted">tunable · Part 209</Chip>
        </span>
        <Src ov={ov} />
      </div>
    )
  }
  return <LuminaireNumberRow k="CCT" ov={ov} unit="K" />
}

function EnergyBankRows({
  title,
  bank,
  energyUnit,
  powerUnit,
  now,
}: {
  title: string
  bank: EnergyBank | undefined
  energyUnit: string
  powerUnit: string
  now: number
}) {
  if (!bank) return null
  return (
    <>
      <BankRow
        label={`${title} energy`}
        ov={bank.energy}
        now={now}
        format={plain}
        scale={scaleLabel(bank.energy_scale?.value ?? null, energyUnit)}
      />
      <BankRow
        label={`${title} power`}
        ov={bank.power}
        now={now}
        format={plain}
        scale={scaleLabel(bank.power_scale?.value ?? null, powerUnit)}
      />
    </>
  )
}

function ConditionRow({ label, cond, now }: { label: string; cond: Condition | undefined; now: number }) {
  if (!cond) return null
  const flag = renderBank(cond.flag, now, plain)
  const count = renderBank(cond.counter, now, plain)
  const chip =
    flag.kind === 'value' ? (
      flag.text === '1' ? (
        <span class="chip warn">active</span>
      ) : (
        <span class="chip ok">clear</span>
      )
    ) : flag.kind === 'not_implemented' ? (
      <span class="chip muted">not implemented</span>
    ) : flag.kind === 'tmask' ? (
      <span class={`chip tmask${flag.stale ? ' stale' : ''}`}>unavailable</span>
    ) : (
      <span class="chip muted">not read</span>
    )
  return (
    <div class="cond">
      <span class="k">{label}</span>
      {chip}
      <span class="n">{count.kind === 'value' ? count.text : '—'}</span>
    </div>
  )
}

const temperature = (raw: number) => `${raw - BANK_TEMPERATURE_OFFSET} °C`

function EnergyCards({ attributes, now }: { attributes: Attributes; now: number }) {
  const energy = memoryEnergyOf(attributes)
  if (!energy) {
    return (
      <Card title="Energy & power" span2 action={<span class="badge">banks 202 / 203 / 204</span>}>
        <div class="empty">No energy readings yet — this gear may not declare device type 51.</div>
      </Card>
    )
  }
  return (
    <>
      {energy ? (
        <Card title="Energy & power" span2 action={<span class="badge">banks 202 / 203 / 204</span>}>
          <EnergyBankRows title="Active" bank={energy.active} energyUnit="Wh" powerUnit="W" now={now} />
          <EnergyBankRows
            title="Apparent"
            bank={energy.apparent}
            energyUnit="VAh"
            powerUnit="VA"
            now={now}
          />
          <EnergyBankRows
            title="Load-side"
            bank={energy.loadside}
            energyUnit="Wh"
            powerUnit="W"
            now={now}
          />
        </Card>
      ) : null}
    </>
  )
}

function DiagnosticsCards({ attributes, now }: { attributes: Attributes; now: number }) {
  const diag = memoryDiagnosticsOf(attributes)
  const gear = diag?.control_gear
  const source = diag?.light_source
  const lum = diag?.luminaire
  if (!gear && !source && !lum) {
    return (
      <Card title="Diagnostics" span2 action={<span class="badge">banks 205 / 206 / 207</span>}>
        <div class="empty">No diagnostics readings yet — this gear may not declare device type 52.</div>
      </Card>
    )
  }
  return (
    <>
      {gear ? (
        <Card title="Control gear diagnostics" span2 action={<span class="badge">bank 205</span>}>
          <BankRow label="Operating time" ov={gear.operating_time_s} now={now} format={(s) => `${Math.floor(s / 3600).toLocaleString()} h`} />
          <BankRow label="Starts" ov={gear.start_counter} now={now} format={plain} />
          <BankRow label="Supply voltage" ov={gear.supply_voltage_decivolt} now={now} format={(v) => `${(v / 10).toFixed(1)} V`} />
          <BankRow label="Supply frequency" ov={gear.supply_frequency_hz} now={now} format={(f) => (f === 0 ? 'DC' : `${f} Hz`)} />
          <BankRow label="Power factor" ov={gear.power_factor_centi} now={now} format={(p) => (p / 100).toFixed(2)} />
          <BankRow label="Gear temperature" ov={gear.temperature_offset60} now={now} format={temperature} />
          <BankRow label="Output current" ov={gear.output_current_percent} now={now} format={(p) => `${p} %`} />
          <p class="hint">
            These conditions also drive the gear-failure status bit (DiiA 253 §9.2.12) — one fault,
            two places.
          </p>
          <ConditionRow label="Overall failure" cond={gear.overall_failure} now={now} />
          <ConditionRow label="Undervoltage" cond={gear.undervoltage} now={now} />
          <ConditionRow label="Overvoltage" cond={gear.overvoltage} now={now} />
          <ConditionRow label="Output power limitation" cond={gear.output_power_limitation} now={now} />
          <ConditionRow label="Thermal derating" cond={gear.thermal_derating} now={now} />
          <ConditionRow label="Thermal shutdown" cond={gear.thermal_shutdown} now={now} />
        </Card>
      ) : null}

      {source ? (
        <Card title="Light source diagnostics" span2 action={<span class="badge">bank 206</span>}>
          <BankRow label="On time" ov={source.on_time_s} now={now} format={(s) => `${Math.floor(s / 3600).toLocaleString()} h`} />
          <BankRow label="Starts" ov={source.start_counter} now={now} format={plain} />
          <BankRow label="Output voltage" ov={source.voltage_decivolt} now={now} format={(v) => `${(v / 10).toFixed(1)} V`} />
          <BankRow label="Output current" ov={source.current_milliamp} now={now} format={(a) => `${(a / 1000).toFixed(3)} A`} />
          <BankRow label="Source temperature" ov={source.temperature_offset60} now={now} format={temperature} />
          <p class="hint">
            Short circuit is bank offset 0x18 and open circuit 0x1A — the reverse of Table 5&apos;s
            flag numbering.
          </p>
          <ConditionRow label="Overall failure" cond={source.overall_failure} now={now} />
          <ConditionRow label="Short circuit" cond={source.short_circuit} now={now} />
          <ConditionRow label="Open circuit" cond={source.open_circuit} now={now} />
          <ConditionRow label="Thermal derating" cond={source.thermal_derating} now={now} />
          <ConditionRow label="Thermal shutdown" cond={source.thermal_shutdown} now={now} />
        </Card>
      ) : null}

      {lum ? (
        <Card title="Luminaire maintenance data" span2 action={<span class="badge">bank 207</span>}>
          <BankRow label="Rated median useful life" ov={lum.rated_life_kilohours} now={now} format={(k) => `${(k * 1000).toLocaleString()} h`} />
          <BankRow label="Reference gear temperature" ov={lum.reference_temperature_offset60} now={now} format={temperature} />
          <BankRow label="Rated median useful starts" ov={lum.rated_starts_hundreds} now={now} format={(h) => (h * 100).toLocaleString()} />
        </Card>
      ) : null}
    </>
  )
}
