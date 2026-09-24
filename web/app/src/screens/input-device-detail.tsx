import { api } from '../api/client'
import type { InputDeviceDetail, InputInstance, ReadValue } from '../api/types'
import { Badge, BitChips, Card, Chip, EditableName, EditableText, Switch } from '../components/ui'
import { ADAPTER, ago, daliVersion, deviceNow, hex2, pad2, registerDeviceNow } from '../format'
import { useLive } from '../hooks'
import { nav } from '../router'
import { mutate, notify, runOp } from '../toast'

const DEV_CAPS: (readonly [number, string])[] = [
  [1 << 0, 'application controller'],
  [1 << 1, 'has instances'],
]

const DEV_STATUS: (readonly [number, string, string])[] = [
  [1 << 0, 'device error', 'err'],
  [1 << 1, 'quiescent', 'warn'],
  [1 << 2, 'address is MASK', 'warn'],
  [1 << 3, 'application active', 'ok'],
  [1 << 4, 'controller error', 'err'],
  [1 << 5, 'power cycle seen', 'warn'],
  [1 << 6, 'reset state', 'warn'],
]

const INSTANCE_ACTIVE = 1 << 1
const INSTANCE_ERROR = 1 << 0

const FB_CAP_BRIGHTNESS = 1 << 1
const FB_CAPS: (readonly [number, string])[] = [
  [1 << 0, 'visible'],
  [FB_CAP_BRIGHTNESS, 'brightness'],
  [1 << 2, 'colour'],
  [1 << 3, 'audible'],
  [1 << 4, 'volume'],
  [1 << 5, 'pitch'],
  [1 << 6, 'common brightness'],
]

const FB_COLOUR_CAPS: (readonly [number, string])[] = [
  [1 << 0, 'red'],
  [1 << 1, 'green'],
  [1 << 2, 'blue'],
  [1 << 3, '2-bit resolution'],
  [1 << 4, 'mixing'],
  [1 << 5, 'common colour'],
]

const FILTER_BITS: { bit: number; label: string }[] = [
  { bit: 0, label: 'release' },
  { bit: 1, label: 'press' },
  { bit: 2, label: 'short_press' },
  { bit: 3, label: 'double_press' },
  { bit: 4, label: 'long_press_start' },
  { bit: 5, label: 'long_press_repeat' },
  { bit: 6, label: 'long_press_stop' },
  { bit: 7, label: 'stuck_free' },
]

function Prov({ v }: { v: ReadValue<unknown> | undefined }) {
  if (v?.read_at_ms != null) {
    return (
      <span class="src readback">
        <span class="dot" />
        read · {ago(v.read_at_ms)}
      </span>
    )
  }
  if (v?.value != null) {
    return (
      <span class="src stored">
        <span class="dot" />
        stored
      </span>
    )
  }
  return (
    <span class="src stale">
      <span class="dot" />
      never read
    </span>
  )
}

function Row({
  k,
  title,
  hint,
  controls,
  children,
  prov,
}: {
  k: string
  title?: string
  hint?: string
  controls?: boolean
  children: preact.ComponentChildren
  prov?: ReadValue<unknown>
}) {
  return (
    <div class={controls ? 'attr controls' : 'attr'}>
      <span class="k" title={title}>
        {k}
        {hint && <span class="rowhint">{hint}</span>}
      </span>
      <span class="v">{children}</span>
      {controls || prov === undefined ? <span /> : <Prov v={prov} />}
    </div>
  )
}

function feedbackColourCss(colour: number | null): string {
  if (colour === null || colour < 1 || colour > 63) return 'transparent'
  const r = colour & 0x03
  const g = (colour >> 2) & 0x03
  const b = (colour >> 4) & 0x03
  const max = Math.max(r, g, b, 1)
  const ch = (v: number) => Math.round((v / max) * 255)
  return `rgb(${ch(r)}, ${ch(g)}, ${ch(b)})`
}

function InstancePanel({
  short,
  inst,
  refresh,
}: {
  short: number
  inst: InputInstance
  refresh: () => void
}) {
  const a = ADAPTER
  const filter = inst.event_filter.value
  const isButton = inst.instance_type === 1
  const schemeBad = inst.event_scheme.value !== null && !inst.event_scheme_confirmed
  const instanceActive =
    inst.instance_status !== null && (inst.instance_status & INSTANCE_ACTIVE) !== 0

  const setInstanceEnabled = (enabled: boolean) => {
    void runOp('instance enabled', () =>
      api.configureInputInstance(a, short, inst.instance_number, { enabled }),
    ).then(refresh)
  }
  const commitFeedback = (raw: string, field: string) => {
    const value = Number(raw)
    if (!Number.isInteger(value) || value < 0 || value > 255) return
    void runOp('feedback config', () =>
      api.configureInputFeedback(a, short, inst.instance_number, { [field]: value }),
    ).then(refresh)
  }
  const commitGroup = (raw: string, slot: number) => {
    const trimmed = raw.trim()
    const groups: (number | null)[] = inst.instance_groups.map((g) => g.value ?? null)
    if (trimmed === '' || trimmed === '—') {
      groups[slot] = null
    } else {
      const value = Number(trimmed)
      if (!Number.isInteger(value) || value < 0 || value > 31) return
      groups[slot] = value
    }
    void runOp('instance groups', () =>
      api.configureInputInstance(a, short, inst.instance_number, { instance_groups: groups }),
    ).then(refresh)
  }
  const timerRow = (
    slot: number,
    field: string,
    unitMs: number,
    unit: string,
    title: string,
    tip: string,
    hint?: string,
  ) => (
    <Row k={title} title={tip} hint={hint} prov={inst.timers[slot]}>
      <EditableText
        value={
          inst.timers[slot]?.value === null || inst.timers[slot] === undefined
            ? ''
            : String((inst.timers[slot].value ?? 0) * unitMs)
        }
        placeholder="—"
        inputMode="numeric"
        onCommit={(raw: string) => {
          const v = Number(raw)
          if (!Number.isInteger(v) || v < 0 || (v === 0 && field !== 't_double_ms')) return
          void runOp(field, () =>
            api.configureInputInstance(a, short, inst.instance_number, {
              timers: { [field]: v },
            }),
          ).then(refresh)
        }}
      />
      <span class="unit">{unit}</span>
    </Row>
  )

  const toggleBit = (bit: number) => {
    if (filter === null) return
    const next = filter.slice()
    next[0] = (next[0] ?? 0) ^ (1 << bit)
    void runOp('instance config', () =>
      api.configureInputInstance(a, short, inst.instance_number, { event_filter: next }),
    ).then(refresh)
  }

  return (
    <div class="grid2">
      <Card title="Behaviour">
        <Row
          k="Instance active"
          title="103 Table 16 bit 1 — the one bit in either status table a controller can write (ENABLE / DISABLE INSTANCE). A disabled instance answers every query exactly as a live one does and simply never emits an event, which is the only explanation for a button that is present, healthy and silent."
        >
          {inst.instance_status === null ? (
            <>
              <button class="btn sm" onClick={() => setInstanceEnabled(true)}>
                Enable
              </button>
              <button class="btn sm" onClick={() => setInstanceEnabled(false)}>
                Disable
              </button>
            </>
          ) : (
            <>
              {(inst.instance_status & INSTANCE_ERROR) !== 0 && <Chip cls="err">error</Chip>}
              <span class="mono faint">{hex2(inst.instance_status)}</span>
              <Switch on={instanceActive} onToggle={() => setInstanceEnabled(!instanceActive)} />
            </>
          )}
        </Row>
        {inst.resolution !== null && !isButton && (
          <Row
            k="Resolution"
            title="Bits of `inputValue` this instance reports (302/304). Read-only — a property of the sensor, not a setting."
          >
            <span class="mono">{inst.resolution}</span>
            <span class="unit">bit</span>
          </Row>
        )}
        {isButton && filter !== null && (
          <Row
            controls
            k="Events sent"
            title="A rule bound to a disabled event can never fire."
            hint="double_press delays every single click by 0.2–2 s"
          >
            <span class="evs">
              {FILTER_BITS.map(({ bit, label }) => (
                <button
                  key={label}
                  class={`ev ${((filter[0] ?? 0) >> bit) & 1 ? 'on' : 'off'}`}
                  onClick={() => toggleBit(bit)}
                >
                  {label}
                </button>
              ))}
            </span>
          </Row>
        )}
        <Row
          controls
          k="Instance groups"
          title="Group 0 is the Part 332 radio-button option; group 1 the panel set one SELECT frame switches. 0..31; anything else the device discards in silence."
        >
          <span class="gsel">
            {inst.instance_groups.map((g, slot) => (
              <EditableText
                key={slot}
                value={g.value === null || g.value === undefined ? '' : String(g.value)}
                placeholder="—"
                inputMode="numeric"
                onCommit={(raw: string) => commitGroup(raw, slot)}
              />
            ))}
            <Prov v={inst.instance_groups[0]} />
          </span>
        </Row>
        <Row
          k="Event scheme"
          title="Scheme 2 is what names the source — short address plus instance number — and it is what commissioning sets. Scheme 0 is the factory default and carries no identity at all, so no rule keyed on this panel can match its events. The device reverts to 0 silently when it loses its short address (103 §9.6.2), which is why the value shown is the read-back and not what we wrote."
          prov={inst.event_scheme}
        >
          <EditableText
            value={inst.event_scheme.value === null ? '' : String(inst.event_scheme.value)}
            placeholder="—"
            inputMode="numeric"
            onCommit={(raw: string) => {
              const v = Number(raw)
              if (!Number.isInteger(v) || v < 0 || v > 4) return
              void runOp('event scheme', () =>
                api.configureInputInstance(a, short, inst.instance_number, { event_scheme: v }),
              ).then(refresh)
            }}
          />
          <span class="unit" />
        </Row>
        <Row
          k="Event priority"
          title="3 is the factory value and where a button belongs. 103 §9.4.1 wants a slot left for the controller's own answers, so a panel at 2 would crowd the backward window."
          hint="2 is refused (422)"
          prov={inst.event_priority}
        >
          <EditableText
            value={inst.event_priority.value === null ? '' : String(inst.event_priority.value)}
            placeholder="—"
            inputMode="numeric"
            onCommit={(raw: string) => {
              const v = Number(raw)
              if (!Number.isInteger(v) || v < 3 || v > 5) return
              void runOp('event priority', () =>
                api.configureInputInstance(a, short, inst.instance_number, { event_priority: v }),
              ).then(refresh)
            }}
          />
          <span class="unit" />
        </Row>
      </Card>

      <Card title="Timing">
        {timerRow(
          0,
          't_short_ms',
          20,
          'ms',
          'T_short',
          'Short vs long press boundary, in ms (steps of 20). Lives on the device and is adjustable there (Part 333) — never persisted here, blank until read.',
        )}
        {timerRow(
          1,
          't_double_ms',
          20,
          'ms',
          'T_double',
          'The standard has no way to tell a single click from the first of a double until this window closes.',
          '0 = off (factory); any other value delays every single click',
        )}
        {timerRow(
          2,
          't_repeat_ms',
          20,
          'ms',
          'T_repeat',
          'Long-press repeat interval. A rule that dims by counting repeats will drift — repeats are legally dropped under load, which is why dim_hold bills by elapsed time.',
        )}
        {timerRow(
          3,
          't_stuck_s',
          1,
          's',
          'T_stuck',
          'How long a held button counts as stuck before the panel reports it, in seconds (5–255). A stuck button is a fault the panel announces, not an event to act on.',
        )}
        <Row k="Events seen" title="Since this controller booted.">
          <span class="mono">{inst.runtime.event_count}</span>
          <span class="unit" />
        </Row>
        {inst.runtime.last_event_at_ms !== null && (
          <Row k="Last event">
            <span class="mono">{ago(inst.runtime.last_event_at_ms)}</span>
            <span class="unit" />
          </Row>
        )}
      </Card>

      {inst.feedback.present && (
        <Card title="Indicator · Part 332" span2>
          <Row
            controls
            k="Brightness"
            title="Active / inactive brightness live in the panel's NVM; the lit-or-not state is re-asserted by rules after every panel power cycle."
            hint={inst.feedback.opcode_map === 'ed1' ? 'answers the 2017 opcode map' : undefined}
          >
            <span class="fbctl">
              {inst.feedback.active_colour !== null && (
                <span
                  class="fb-swatch"
                  title={`colour ${inst.feedback.active_colour}`}
                  style={{ background: feedbackColourCss(inst.feedback.active_colour) }}
                />
              )}
              <EditableText
                value={
                  inst.feedback.active_brightness === null
                    ? ''
                    : String(inst.feedback.active_brightness)
                }
                placeholder="active"
                inputMode="numeric"
                onCommit={(raw) => commitFeedback(raw, 'active_brightness')}
              />
              <EditableText
                value={
                  inst.feedback.inactive_brightness === null
                    ? ''
                    : String(inst.feedback.inactive_brightness)
                }
                placeholder="inactive"
                inputMode="numeric"
                onCommit={(raw) => commitFeedback(raw, 'inactive_brightness')}
              />
              {inst.feedback.capability === null ? (
                <span class="faint">capability not read</span>
              ) : (
                (inst.feedback.capability & FB_CAP_BRIGHTNESS) === 0 && (
                  <Chip cls="warn">on/off only</Chip>
                )
              )}
            </span>
          </Row>
          {inst.feedback.capability !== null && (
            <Row
              controls
              k="Declared capability"
              title="What the instance answered to QUERY FEEDBACK CAPABILITY, decoded. Absent from this list is absent on the panel, not absent from the product."
            >
              <span class="chips">
                <BitChips value={inst.feedback.capability} bits={FB_CAPS} />
              </span>
            </Row>
          )}
          {inst.feedback.colour_capability !== null && (
            <Row
              controls
              k="Declared colour capability"
              title="QUERY FEEDBACK COLOUR CAPABILITY, decoded — the corrected opcode map only, since the 2017 edition has no such query."
            >
              <span class="chips">
                <BitChips value={inst.feedback.colour_capability} bits={FB_COLOUR_CAPS} />
              </span>
            </Row>
          )}
        </Card>
      )}
      {inst.manual_config_active && (
        <div class="warnbar span2">
          Manual configuration is active on this instance (Part 333) — writes to these
          variables are discarded by the device, silently.
        </div>
      )}
      {schemeBad && (
        <div class="warnbar span2">
          The event scheme reverted on the device: its events no longer name it, so no rule
          keyed on this panel can match them.
        </div>
      )}
    </div>
  )
}

export function InputDeviceDetailScreen({
  short,
  instance,
}: {
  short: number
  instance?: string
}) {
  const { data: d, reload } = useLive<InputDeviceDetail>(
    () => api.inputDevice(ADAPTER, short),
    ['input'],
    { deps: [short] },
  )
  if (!d) return <div class="empty">Loading control device SA {pad2(short)}…</div>
  registerDeviceNow(d.now_ms)

  const tab = instance !== undefined ? Number(instance) : (d.instances[0]?.instance_number ?? 0)
  const active = d.instances.find((i) => i.instance_number === tab) ?? d.instances[0]
  const now = deviceNow()
  const settling =
    d.nvm_settling_until_ms !== null && now !== null && d.nvm_settling_until_ms > now

  return (
    <div class="input-devices">
      <div class="crumbs">
        <a href="#/">Adapter {ADAPTER}</a> / <a href="#/input-devices">Input devices</a> / SA{' '}
        {pad2(short)}
      </div>

      <div class="head">
        <span class="h1-wrap" title="Click to rename">
          <EditableName
            cls={`h1-edit${d.name ? '' : ' name-faint'}`}
            value={d.name ?? ''}
            placeholder="— unnamed"
            onCommit={(raw) =>
              void mutate(
                'Rename input device',
                () =>
                  api.patchInputDevice(ADAPTER, short, {
                    name: raw.trim() === '' ? null : raw.trim(),
                  }),
                reload,
              )
            }
          />
          <span class="pencil">✎</span>
        </span>
        <Badge cls="addr">SA {pad2(short)}</Badge>
        <Badge>{d.instance_count} instances</Badge>
        {d.present ? (
          <Chip cls="ok">Present</Chip>
        ) : d.last_seen_ms === null ? (
          <Chip cls="idle">not probed</Chip>
        ) : (
          <Chip cls="warn">not answering</Chip>
        )}
        <span class="spacer" />
        <button
          class="btn"
          title="Re-read every control device on the segment. Table 14/15 bytes are volatile and never persisted, so they are blank after a reboot until this runs."
          onClick={() =>
            void runOp('input scan', () => api.scanInputDevices(ADAPTER)).then(reload)
          }
        >
          ⌕ Scan
        </button>
        <button
          class="btn"
          onClick={() =>
            void runOp('identify', () => api.identifyInputDevice(ADAPTER, short)).then(reload)
          }
        >
          ⚲ Identify
        </button>
        <button
          class="btn ghost danger"
          title="Forget this device's record. It stays on the bus and the next scan finds it again."
          onClick={() => {
            void mutate('Forget input device', () => api.forgetInputDevice(ADAPTER, short), () => {
              notify(`SA ${pad2(short)} forgotten`, 'succeeded')
              nav('/input-devices')
            })
          }}
        >
          Forget
        </button>
      </div>

      {settling && (
        <div class="warnbar">
          Settings settling — a config written in the last 30 s may not survive a power cycle
          yet.
        </div>
      )}

      <div class="grid2">
        <Card title="Information">
          <Row k="Short address">
            <span class="mono">{pad2(short)}</span>
            <span class="unit" />
          </Row>
          <Row
            k="Version"
            title="The Part 103 version the device implements, IEC 62386-101 §4.2 encoded."
          >
            {d.version_number !== null ? (
              <>
                <span class="mono">{daliVersion(d.version_number)}</span>
                <span class="raw">{hex2(d.version_number)}</span>
              </>
            ) : (
              <span class="faint">—</span>
            )}
            <span class="unit" />
          </Row>
          <Row
            k="Last seen"
            title="When this device last answered. `null` after a reboot until the next scan — presence is a runtime fact and deliberately not persisted, so `not probed` is a third state and not the same claim as `not answering`."
          >
            <span class="mono">
              {d.last_seen_ms !== null ? ago(d.last_seen_ms) : 'not probed'}
            </span>
            <span class="unit" />
          </Row>
          <Row k="Last event">
            <span class="mono">
              {d.last_event_at_ms !== null ? ago(d.last_event_at_ms) : '—'}
            </span>
            <span class="unit" />
          </Row>
          <Row
            k="Home Assistant"
            title="Per-device opt-in. The global switch lives in Settings · Home Assistant."
          >
            <Switch
              on={d.ha_expose}
              onToggle={() =>
                void mutate(
                  'Home Assistant exposure',
                  () => api.patchInputDevice(ADAPTER, short, { ha_expose: !d.ha_expose }),
                  reload,
                )
              }
            />
          </Row>
        </Card>

        <Card title="What the device declares">
          {d.device_capabilities === null && d.device_status === null ? (
            <div class="empty">
              Nothing read since boot. Every byte in this card is volatile by
              construction — 103 Table 15 moves when another master enables the
              application and a mains cycle sets <i>power cycle seen</i>, so it
              is never persisted and reads blank after every reboot. Run a scan
              to fill it.
              <div class="emptyact">
                <button
                  class="btn"
                  onClick={() =>
                    void runOp('input scan', () => api.scanInputDevices(ADAPTER)).then(reload)
                  }
                >
                  ⌕ Scan
                </button>
              </div>
            </div>
          ) : (
            <>
              {d.device_capabilities !== null && (
                <Row
                  controls
                  k="Capabilities"
                  title="103 Table 14, answered to QUERY DEVICE CAPABILITIES. Read-only — the device declares these, a controller cannot set them."
                >
                  <span class="chips">
                    <BitChips value={d.device_capabilities} bits={DEV_CAPS} />
                  </span>
                </Row>
              )}
              {d.device_status !== null && (
                <Row
                  controls
                  k="Status"
                  title="103 Table 15. Volatile by construction, never persisted — blank after a reboot until the next scan."
                >
                  <span class="chips">
                    <BitChips value={d.device_status} bits={DEV_STATUS} />
                  </span>
                </Row>
              )}
            </>
          )}
        </Card>
      </div>

      {d.instances.length > 0 && (
        <>
          <div class="ptabs">
            {d.instances.map((i) => (
              <a
                key={i.instance_number}
                class={`ptab${i.instance_number === active?.instance_number ? ' active' : ''}`}
                href={`#/input-devices/${short}/${i.instance_number}`}
              >
                {i.instance_type_name ?? `type ${i.instance_type ?? '?'}`}
                <span class="hint">#{i.instance_number}</span>
              </a>
            ))}
          </div>
          <div class="ptabline" />
          {active && (
            <InstancePanel
              key={active.instance_number}
              short={short}
              inst={active}
              refresh={reload}
            />
          )}
        </>
      )}
    </div>
  )
}
