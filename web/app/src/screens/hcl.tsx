import { useState } from 'preact/hooks'
import { api } from '../api/client'
import { ADAPTER } from '../format'
import type {
  ControllerTime,
  HclAlgorithm,
  HclLevelMode,
  HclOverride,
  HclOverrideTarget,
  HclSchedule,
  HclSchedulePoint,
  HclTarget,
  HclTimeRef,
  Weekday,
} from '../api/types'
import { Card, EditableText, SelChip } from '../components/ui'
import { usePoll } from '../hooks'
import { nav } from '../router'
import { errorMessage, mutateBusy, notify, opCommitted, trackOp } from '../toast'

const POLL_MS = 5000
const WEEKDAYS: Weekday[] = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun']
const WEEKDAY_LABEL: Record<Weekday, string> = {
  mon: 'Mo', tue: 'Tu', wed: 'We', thu: 'Th', fri: 'Fr', sat: 'Sa', sun: 'Su',
}
const MINUTES_PER_DAY = 1440
const DALI_GROUP_COUNT = 16
const MAX_TARGETS = 16
const MAX_POINTS = 24
const MAX_LEVEL = 254
const CURVE_W = 720
const CURVE_H = 168
const CURVE_TOP = 10
const CURVE_BASE = 130
const CCT_FLOOR_K = 2000
const CCT_SPAN_K = 3000

const clampInt = (raw: string, lo: number, hi: number, fallback: number) => {
  const n = Number.parseInt(raw, 10)
  return Number.isNaN(n) ? fallback : Math.min(hi, Math.max(lo, n))
}

export function hhmm(minutes: number): string {
  const m = ((minutes % MINUTES_PER_DAY) + MINUTES_PER_DAY) % MINUTES_PER_DAY
  return `${String(Math.floor(m / 60)).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`
}

function parseHhmm(raw: string, fallback: number): number {
  const m = raw.match(/^(\d{1,2}):?(\d{2})$/)
  if (!m) return clampInt(raw, 0, MINUTES_PER_DAY - 1, fallback)
  const minutes = Number(m[1]) * 60 + Number(m[2])
  return minutes >= 0 && minutes < MINUTES_PER_DAY ? minutes : fallback
}

export function describePoint(p: HclSchedulePoint): string {
  const when =
    p.time_ref === 'absolute'
      ? hhmm(p.offset_minutes)
      : `${p.time_ref} ${p.offset_minutes >= 0 ? '+' : '−'}${Math.abs(p.offset_minutes)}`
  const level =
    p.level_mode === 'absolute' ? String(p.level ?? 0)
      : p.level_mode === 'last_active' ? 'last active'
      : null
  const cct = p.color_temperature_kelvin != null ? `${p.color_temperature_kelvin} K` : null
  return [when, level, cct].filter(Boolean).join(' · ')
}

export function nextPoint(
  points: HclSchedulePoint[],
  nowMinutes: number | null,
): HclSchedulePoint | null {
  if (points.length === 0) return null
  if (nowMinutes == null) return points[0]
  const absolute = points.filter((p) => p.time_ref === 'absolute')
  const upcoming = absolute.find((p) => p.offset_minutes > nowMinutes)
  return upcoming ?? absolute[0] ?? points[0]
}

function targetSummary(targets: HclTarget[]): string {
  return targets
    .map((t) =>
      t.scope === 'broadcast'
        ? `A${t.adapter_id}: broadcast`
        : `A${t.adapter_id}: ${(t.group_ids ?? []).map((g) => `G${g}`).join(', ')}`,
    )
    .join(' · ')
}

function overrideTargetSummary(targets: HclOverrideTarget[]): string {
  return targets
    .map((t) =>
      t.scope === 'broadcast'
        ? `broadcast on A${t.adapter_id}`
        : `G${t.group_id} on A${t.adapter_id}`,
    )
    .join(', ')
}

function OverrideStrip({
  override,
  busy,
  onResume,
}: {
  override: HclOverride | null
  busy: boolean
  onResume: () => void
}) {
  if (!override?.suspended) return null
  const since =
    override.since_local_minutes === undefined ? null : hhmm(override.since_local_minutes)
  return (
    <div class="standdown">
      <span class="ico">‖</span>
      <span class="txt">
        Standing down{since && <> since <b>{since}</b></>} —{' '}
        <span class="g">{overrideTargetSummary(override.targets)}</span> was set by hand, so
        this schedule stops driving it until midnight.
      </span>
      <span class="spacer" />
      <button class="btn ghost" disabled={busy} onClick={onResume}>
        Resume now
      </button>
    </div>
  )
}

function TimeGate({ time }: { time: ControllerTime | null }) {
  if (!time || time.synced) return null
  return (
    <div class="banner warn">
      <span class="ico">⚠</span>
      <span class="txt">
        <b>Controller clock is not synchronised</b> — every schedule is paused.
        <span class="sub">
          A schedule acts on local time of day; without an anchored clock it stays silent
          rather than guessing. Waiting for SNTP, or set the time manually.
        </span>
      </span>
    </div>
  )
}

export function HclSchedules() {
  const { data, error, reload } = usePoll(async () => {
    const [list, time] = await Promise.all([api.hclSchedules(), api.time()])
    const overrides = new Map<string, HclOverride>()
    await Promise.all(
      list.schedules.map(async (s) => {
        overrides.set(s.schedule_id, await api.hclOverride(s.schedule_id))
      }),
    )
    return { list, time, overrides }
  }, POLL_MS)
  const [busy, setBusy] = useState(false)

  if (error) return <div class="empty">Failed to load schedules: {error}</div>
  if (!data) return <div class="empty">Loading schedules…</div>
  const { list, time, overrides } = data

  const resume = async (scheduleId: string) => {
    await mutateBusy(`Schedule ${scheduleId}`, setBusy, () => api.clearHclOverride(scheduleId), reload)
  }

  const toggle = async (s: HclSchedule) => {
    setBusy(true)
    try {
      const committed = opCommitted(
        await trackOp(
          `Schedule ${s.schedule_id}`,
          await api.patchHclSchedule(s.schedule_id, { enabled: !s.enabled }),
        ),
      )
      if (committed) void reload()
    } catch (e) {
      notify(`Schedule ${s.schedule_id}`, 'failed', errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  const create = async () => {
    setBusy(true)
    try {
      const created = await api.createHclSchedule(blankSchedule())
      if (!opCommitted(await trackOp('New schedule', created))) return
      nav(`/hcl/${created.schedule_id}`)
    } catch (e) {
      notify('New schedule', 'failed', errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div>
      <div class="crumbs">Adapter {ADAPTER} / HCL schedules</div>
      <div class="head">
        <h1>HCL schedules</h1>
        <span class="sub">
          {list.schedules.length} schedule{list.schedules.length === 1 ? '' : 's'} · {list.schedules.filter((s) => s.enabled).length} enabled
        </span>
        <span class="spacer" />
        <button class="btn primary" disabled={busy} onClick={create}>
          New schedule
        </button>
      </div>

      <TimeGate time={time} />

      {list.schedules.length === 0 ? (
        <div class="empty">
          No schedules yet. A schedule drives colour temperature and brightness across the
          day on groups or the whole bus.
        </div>
      ) : (
        <div class="hcl-list">
          {list.schedules.map((s) => (
            <ScheduleCard
              key={s.schedule_id}
              schedule={s}
              override={overrides.get(s.schedule_id) ?? null}
              nowMinutes={time.synced ? time.local_minutes : null}
              busy={busy}
              onToggle={() => void toggle(s)}
              onResume={() => void resume(s.schedule_id)}
            />
          ))}
        </div>
      )}
    </div>
  )
}

function ScheduleCard({
  schedule,
  override,
  nowMinutes,
  busy,
  onToggle,
  onResume,
}: {
  schedule: HclSchedule
  override: HclOverride | null
  nowMinutes: number | null
  busy: boolean
  onToggle: () => void
  onResume: () => void
}) {
  const next = nextPoint(schedule.points, nowMinutes)
  const suspended = override?.suspended === true
  return (
    <div class={schedule.enabled ? 'hcl-card' : 'hcl-card paused'}>
      <div class="crow">
        <button
          class={schedule.enabled ? 'toggle on' : 'toggle'}
          disabled={busy}
          aria-label={schedule.enabled ? 'Disable schedule' : 'Enable schedule'}
          onClick={onToggle}
        >
          <i />
        </button>
        <a class="sid" href={`#/hcl/${schedule.schedule_id}`}>
          {schedule.schedule_id}
        </a>
        <span class="algo">{schedule.algorithm}</span>
        <span class="spacer" />
        <span class="days">
          {WEEKDAYS.map((d) => (
            <span key={d} class={schedule.active_days.includes(d) ? 'day on' : 'day'}>
              {WEEKDAY_LABEL[d]}
            </span>
          ))}
        </span>
      </div>
      <div class="meta">
        <div>
          <div class="k">Targets</div>
          <div class="v">{targetSummary(schedule.targets) || '—'}</div>
        </div>
        <div>
          <div class="k">Points</div>
          <div class="v">{schedule.points.length}</div>
        </div>
        <div>
          <div class="k">Next point</div>
          {schedule.enabled && next ? (
            <div class={suspended ? 'v held' : 'v next'}>{describePoint(next)}</div>
          ) : (
            <div class="v paused">{schedule.enabled ? '—' : 'disabled'}</div>
          )}
        </div>
      </div>
      <OverrideStrip override={override} busy={busy} onResume={onResume} />
    </div>
  )
}

function blankSchedule(): Partial<HclSchedule> {
  return {
    enabled: false,
    algorithm: 'stepped',
    active_days: [...WEEKDAYS],
    location: null,
    targets: [{ adapter_id: 0, scope: 'broadcast' }],
    points: [
      {
        time_ref: 'absolute',
        offset_minutes: 480,
        level_mode: 'absolute',
        level: 128,
        color_temperature_kelvin: 3000,
      },
    ],
  }
}

function DayCurve({ points }: { points: HclSchedulePoint[] }) {
  const absolute = points
    .filter((p) => p.time_ref === 'absolute')
    .sort((a, b) => a.offset_minutes - b.offset_minutes)
  const x = (minutes: number) => (minutes / MINUTES_PER_DAY) * CURVE_W
  const yLevel = (level: number) =>
    CURVE_BASE - (level / MAX_LEVEL) * (CURVE_BASE - CURVE_TOP)
  const yCct = (kelvin: number) =>
    CURVE_BASE -
    (Math.min(Math.max(kelvin - CCT_FLOOR_K, 0), CCT_SPAN_K) / CCT_SPAN_K) *
      (CURVE_BASE - CURVE_TOP)

  const levelPts = absolute
    .filter((p) => p.level_mode === 'absolute' && p.level != null)
    .map((p) => `${x(p.offset_minutes)},${yLevel(p.level ?? 0)}`)
  const cctPts = absolute
    .filter((p) => p.color_temperature_kelvin != null)
    .map((p) => `${x(p.offset_minutes)},${yCct(p.color_temperature_kelvin ?? 0)}`)
  const astro = points.filter((p) => p.time_ref !== 'absolute')

  return (
    <div class="curve">
      <svg viewBox={`0 0 ${CURVE_W} ${CURVE_H}`} preserveAspectRatio="none" role="img"
        aria-label="Level and colour temperature across the day">
        <line class="grid" x1="0" y1={CURVE_BASE} x2={CURVE_W} y2={CURVE_BASE} />
        <line class="grid" x1="0" y1="70" x2={CURVE_W} y2="70" opacity=".5" />
        <line class="grid" x1="0" y1={CURVE_TOP} x2={CURVE_W} y2={CURVE_TOP} opacity=".5" />
        {[6, 12, 18].map((h) => (
          <line key={h} class="grid" x1={x(h * 60)} y1="6" x2={x(h * 60)} y2="134" opacity=".45" />
        ))}
        {cctPts.length > 1 && <polyline class="cct-line" points={cctPts.join(' ')} />}
        {levelPts.length > 1 && <polyline class="lvl-line" points={levelPts.join(' ')} />}
        {levelPts.map((p) => {
          const [cx, cy] = p.split(',')
          return <circle key={p} class="lvl-dot" cx={cx} cy={cy} r="3.5" />
        })}
        {astro.length > 0 && (
          <text class="astro-lab" x="6" y="24">
            + {astro.length} point{astro.length > 1 ? 's' : ''} at sunrise/sunset
          </text>
        )}
        {[0, 6, 12, 18, 24].map((h) => (
          <text key={h} class="gridlab" x={Math.min(x(h * 60) + 2, CURVE_W - 34)} y="146">
            {hhmm(h * 60 === MINUTES_PER_DAY ? 0 : h * 60)}
          </text>
        ))}
      </svg>
      <div class="legend">
        <span><i />Level</span>
        <span><i class="cct" />Colour temperature</span>
      </div>
    </div>
  )
}

export function HclScheduleEditor({ id }: { id: string }) {
  const { data, error, reload } = usePoll(
    async () => {
      const [schedule, override] = await Promise.all([
        api.hclSchedule(id),
        api.hclOverride(id),
      ])
      return { schedule, override }
    },
    POLL_MS,
    [id],
  )
  const [draft, setDraft] = useState<HclSchedule | null>(null)
  const [busy, setBusy] = useState(false)

  if (error) return <div class="empty">Failed to load schedule: {error}</div>
  if (!data) return <div class="empty">Loading schedule…</div>
  const s = draft ?? data.schedule
  const edit = (patch: Partial<HclSchedule>) => setDraft({ ...s, ...patch })

  const resume = async () => {
    await mutateBusy(`Schedule ${id}`, setBusy, () => api.clearHclOverride(id), reload)
  }

  const save = async () => {
    setBusy(true)
    try {
      const saved = await trackOp(
        `Schedule ${id}`,
        await api.patchHclSchedule(id, {
          enabled: s.enabled,
          algorithm: s.algorithm,
          active_days: s.active_days,
          location: s.location,
          targets: s.targets,
          points: s.points,
        }),
      )
      if (!opCommitted(saved)) return
      setDraft(null)
    } catch (e) {
      notify(`Schedule ${id}`, 'failed', errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  const remove = async () => {
    setBusy(true)
    try {
      await api.deleteHclSchedule(id)
      nav('/hcl')
    } catch (e) {
      notify(`Schedule ${id}`, 'failed', errorMessage(e))
      setBusy(false)
    }
  }

  const needsLocation = s.points.some((p) => p.time_ref !== 'absolute') && s.location == null

  return (
    <div>
      <div class="head">
        <h1>Schedule</h1>
        <span class="sid">{id}</span>
        <span class="spacer" />
        <button class="btn ghost" disabled={busy} onClick={() => void remove()}>Delete</button>
        <button class="btn" disabled={busy || !draft} onClick={() => setDraft(null)}>Cancel</button>
        <button class="btn primary" disabled={busy || !draft} onClick={() => void save()}>Save</button>
      </div>

      <OverrideStrip override={data.override} busy={busy} onResume={() => void resume()} />

      <Card title="Day curve">
        <DayCurve points={s.points} />
      </Card>

      <PointsPanel points={s.points} onChange={(points) => edit({ points })} />

      <div class="hcl-grid2">
        <TargetsPanel targets={s.targets} onChange={(targets) => edit({ targets })} />
        <BehaviourPanel
          schedule={s}
          needsLocation={needsLocation}
          onChange={edit}
        />
      </div>
    </div>
  )
}

function PointsPanel({
  points,
  onChange,
}: {
  points: HclSchedulePoint[]
  onChange: (points: HclSchedulePoint[]) => void
}) {
  const replace = (index: number, patch: Partial<HclSchedulePoint>) =>
    onChange(points.map((p, i) => (i === index ? { ...p, ...patch } : p)))

  const setMode = (index: number, level_mode: HclLevelMode) =>
    replace(index, {
      level_mode,
      level: level_mode === 'absolute' ? (points[index].level ?? 128) : null,
    })

  return (
    <Card title="Points">
      <table class="hcl-points">
        <thead>
          <tr><th>Time</th><th>Offset</th><th>Level mode</th><th>Level</th><th>CCT</th><th /></tr>
        </thead>
        <tbody>
          {points.map((p, i) => (
            <tr key={i}>
              <td>
                <select class="sel" value={p.time_ref}
                  onChange={(e) => replace(i, { time_ref: (e.target as HTMLSelectElement).value as HclTimeRef })}>
                  <option value="absolute">absolute</option>
                  <option value="sunrise">sunrise</option>
                  <option value="sunset">sunset</option>
                </select>
              </td>
              <td>
                <EditableText cls="inp"
                  value={p.time_ref === 'absolute' ? hhmm(p.offset_minutes) : String(p.offset_minutes)}
                  onCommit={(raw) =>
                    replace(i, {
                      offset_minutes:
                        p.time_ref === 'absolute'
                          ? parseHhmm(raw, p.offset_minutes)
                          : clampInt(raw, -720, 720, p.offset_minutes),
                    })
                  }
                />
              </td>
              <td>
                <select class="sel" value={p.level_mode}
                  onChange={(e) => setMode(i, (e.target as HTMLSelectElement).value as HclLevelMode)}>
                  <option value="absolute">absolute</option>
                  <option value="last_active">last active</option>
                  <option value="none">none</option>
                </select>
              </td>
              <td>
                <EditableText cls="inp" disabled={p.level_mode !== 'absolute'} inputMode="numeric"
                  value={p.level_mode === 'absolute' ? String(p.level ?? 0) : '—'}
                  onCommit={(raw) =>
                    replace(i, { level: clampInt(raw, 0, MAX_LEVEL, p.level ?? 0) })
                  }
                />
              </td>
              <td>
                <EditableText cls="inp" inputMode="numeric" placeholder="—"
                  value={p.color_temperature_kelvin == null ? '' : String(p.color_temperature_kelvin)}
                  onCommit={(raw) =>
                    replace(i, {
                      color_temperature_kelvin:
                        raw.trim() === '' ? null : clampInt(raw, 1000, 20000, 3000),
                    })
                  }
                />
              </td>
              <td>
                <button class="btn sm ghost" disabled={points.length <= 1}
                  onClick={() => onChange(points.filter((_, j) => j !== i))}>×</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div class="panel-foot">
        <button class="btn sm" disabled={points.length >= MAX_POINTS}
          onClick={() => onChange([...points, {
            time_ref: 'absolute', offset_minutes: 720, level_mode: 'absolute',
            level: 128, color_temperature_kelvin: 3000,
          }])}>Add point</button>
        <div class="hint">
          <b>last active</b> returns each gear to the brightness its user last set; the curve
          still drives colour. Transition smoothness is a per-device fade time — set it on the
          device card, not here.
        </div>
      </div>
    </Card>
  )
}

function TargetsPanel({
  targets,
  onChange,
}: {
  targets: HclTarget[]
  onChange: (targets: HclTarget[]) => void
}) {
  const replace = (index: number, patch: Partial<HclTarget>) =>
    onChange(targets.map((t, i) => (i === index ? { ...t, ...patch } : t)))

  const toggleGroup = (index: number, group: number) => {
    const current = targets[index].group_ids ?? []
    const next = current.includes(group)
      ? current.filter((g) => g !== group)
      : [...current, group].sort((a, b) => a - b)
    replace(index, { group_ids: next })
  }

  return (
    <Card title="Targets">
      <div class="panel-body">
        {targets.map((t, i) => (
          <div class="trow" key={i}>
            <select class="sel" value={t.scope}
              onChange={(e) => {
                const scope = (e.target as HTMLSelectElement).value as HclTarget['scope']
                replace(i, { scope, group_ids: scope === 'group' ? (t.group_ids ?? [1]) : undefined })
              }}>
              <option value="group">group</option>
              <option value="broadcast">broadcast</option>
            </select>
            {t.scope === 'group' ? (
              <span class="chips">
                {Array.from({ length: DALI_GROUP_COUNT }, (_, g) => (
                  <SelChip key={g} on={(t.group_ids ?? []).includes(g)} onClick={() => toggleGroup(i, g)}>
                    {String(g)}
                  </SelChip>
                ))}
              </span>
            ) : (
              <span class="hint">whole bus</span>
            )}
            <span class="spacer" />
            <button class="btn sm ghost" disabled={targets.length <= 1}
              onClick={() => onChange(targets.filter((_, j) => j !== i))}>×</button>
          </div>
        ))}
        <div class="panel-foot">
          <button class="btn sm" disabled={targets.length >= MAX_TARGETS}
            onClick={() => onChange([...targets, { adapter_id: 0, scope: 'group', group_ids: [1] }])}>
            Add target
          </button>
          <div class="hint">Groups and broadcast only — a schedule never addresses a single lamp.</div>
        </div>
      </div>
    </Card>
  )
}

function BehaviourPanel({
  schedule,
  needsLocation,
  onChange,
}: {
  schedule: HclSchedule
  needsLocation: boolean
  onChange: (patch: Partial<HclSchedule>) => void
}) {
  const toggleDay = (day: Weekday) =>
    onChange({
      active_days: schedule.active_days.includes(day)
        ? schedule.active_days.filter((d) => d !== day)
        : WEEKDAYS.filter((d) => d === day || schedule.active_days.includes(d)),
    })

  const setCoord = (key: 'latitude_deg' | 'longitude_deg', raw: string) => {
    const value = Number.parseFloat(raw)
    if (Number.isNaN(value)) return
    const base = schedule.location ?? { latitude_deg: 0, longitude_deg: 0 }
    onChange({ location: { ...base, [key]: value } })
  }

  return (
    <Card title="Behaviour">
      <div class="panel-body">
        <div class="field">
          <div class="k">Algorithm</div>
          <div class="radio">
            {(['interpolated', 'stepped'] as HclAlgorithm[]).map((a) => (
              <button key={a} class={schedule.algorithm === a ? 'ropt on' : 'ropt'}
                onClick={() => onChange({ algorithm: a })}>
                <div class="rn">{a}</div>
                <div class="rd">{a === 'interpolated' ? 'Ramps between points' : 'Holds each point'}</div>
              </button>
            ))}
          </div>
        </div>
        <div class="field">
          <div class="k">Active days</div>
          <div class="chips">
            {WEEKDAYS.map((d) => (
              <SelChip key={d} on={schedule.active_days.includes(d)} onClick={() => toggleDay(d)}>
                {WEEKDAY_LABEL[d]}
              </SelChip>
            ))}
          </div>
        </div>
        <div class="field">
          <div class="k">Location</div>
          <EditableText cls="inp wide" placeholder="latitude" inputMode="decimal"
            value={schedule.location ? String(schedule.location.latitude_deg) : ''}
            onCommit={(raw) => setCoord('latitude_deg', raw)} />
          <EditableText cls="inp wide" placeholder="longitude" inputMode="decimal"
            value={schedule.location ? String(schedule.location.longitude_deg) : ''}
            onCommit={(raw) => setCoord('longitude_deg', raw)} />
          {needsLocation && (
            <div class="req">Required: a sunrise/sunset point cannot be placed without one.</div>
          )}
        </div>
      </div>
    </Card>
  )
}
