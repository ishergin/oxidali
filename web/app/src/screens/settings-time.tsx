import { useEffect, useState } from 'preact/hooks'
import { api, ApiError } from '../api/client'
import type { ControllerTime, ControllerTimePut } from '../api/types'
import { Card, Chip, EditableText, FieldRow } from '../components/ui'
import { usePoll } from '../hooks'
import {
  browserUtcOffsetMinutes,
  civilTime,
  clockSkewLabel,
  posixTzFromBrowser,
  utcOffsetLabel,
} from '../time-zone'
import { errorMessage, notify } from '../toast'

const TICK_MS = 1000
const INVALID_VALUE = 'invalid_value'
const ZONE_EXAMPLE = '<+01>-1<+02>,M3.5.0/2,M10.5.0/3'

const SOURCE_CHIP: Record<ControllerTime['source'], readonly [cls: string, label: string]> = {
  sntp: ['ok', 'synced by SNTP'],
  manual: ['info', 'set manually'],
  unset: ['warn', 'unset'],
}

interface TimeRead {
  time: ControllerTime
  readAtMs: number
}

function useTick(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs)
    return () => clearInterval(id)
  }, [intervalMs])
  return now
}

export function SettingsTime() {
  const { data, error, reload } = usePoll(
    async (): Promise<TimeRead> => ({ time: await api.time(), readAtMs: Date.now() }),
  )
  const now = useTick(TICK_MS)
  const [draft, setDraft] = useState<string | null>(null)
  const [refused, setRefused] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  if (!data) {
    return <div class="empty">{error ? `Time unavailable — ${error}` : 'Loading time…'}</div>
  }
  const { time, readAtMs } = data
  const zone = draft ?? time.timezone
  const dirty = draft !== null && draft !== time.timezone
  const controllerMs = time.unix_ms === null ? null : time.unix_ms + (now - readAtMs)
  const browserZone = posixTzFromBrowser(browserUtcOffsetMinutes, now)

  const write = async (title: string, body: ControllerTimePut) => {
    setBusy(true)
    try {
      await api.setTime(body)
      setDraft(null)
      setRefused(null)
      notify(title, 'succeeded')
      void reload()
    } catch (e) {
      if (e instanceof ApiError && e.code === INVALID_VALUE && body.timezone !== undefined) {
        setRefused(body.timezone)
      }
      notify(title, 'failed', errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  const takeOver = () =>
    void write(
      'Time from this browser',
      time.source === 'sntp'
        ? { timezone: browserZone }
        : { timezone: browserZone, unix_ms: Date.now() },
    )

  return (
    <div>
      <div class="head">
        <h1>Time</h1>
        <span class="sub">
          The controller's wall clock and zone. HCL schedules and time-of-day rule conditions
          run only on an anchored clock, in local time of this zone.
        </span>
        <span class="spacer" />
        <button class="btn" disabled={busy || !dirty} onClick={() => setDraft(null)}>
          Discard
        </button>
        <button
          class="btn primary"
          disabled={busy || !dirty || zone.trim() === ''}
          onClick={() => void write('Time zone', { timezone: zone.trim() })}
        >
          Apply
        </button>
      </div>

      {!time.synced && (
        <p class="warnbar">
          The clock is not anchored: every HCL schedule and every time-of-day rule condition
          is paused until SNTP syncs or a time is set here.
        </p>
      )}

      <ClockCard
        time={time}
        controllerMs={controllerMs}
        zone={zone}
        zoneRefused={refused !== null && refused === zone}
        onZone={(raw) => setDraft(raw.trim())}
      />
      <BrowserCard
        time={time}
        controllerMs={controllerMs}
        now={now}
        browserZone={browserZone}
        busy={busy}
        onTakeOver={takeOver}
      />
    </div>
  )
}

function ClockCard({
  time,
  controllerMs,
  zone,
  zoneRefused,
  onZone,
}: {
  time: ControllerTime
  controllerMs: number | null
  zone: string
  zoneRefused: boolean
  onZone: (raw: string) => void
}) {
  const [cls, label] = SOURCE_CHIP[time.source]
  const offset = time.utc_offset_minutes
  const anchored = controllerMs !== null && offset !== null
  return (
    <Card title="Controller clock">
      <FieldRow
        label="Local time"
        hint={
          anchored
            ? "In the controller's zone, advancing from the last read."
            : 'Milliseconds since boot are not a time of day, so none is shown.'
        }
      >
        {anchored ? (
          <>
            <span class="mono">{civilTime(controllerMs, offset)}</span>
            <span class="unit">{utcOffsetLabel(offset)}</span>
          </>
        ) : (
          <Chip cls="warn">not set</Chip>
        )}
      </FieldRow>
      <FieldRow
        label="Source"
        hint="SNTP anchors the clock; a manual time is replaced by the next successful sync."
      >
        <Chip cls={cls}>{label}</Chip>
      </FieldRow>
      <FieldRow
        stacked
        label="Zone"
        hint="POSIX TZ. The controller has no zone database, so the daylight-saving rule is part of the string. Stored in flash."
      >
        <EditableText cls={zoneRefused ? 'txt err' : 'txt'} value={zone} onCommit={onZone} />
        {zoneRefused && (
          <p class="req">
            Refused (422): the controller takes a POSIX TZ string such as {ZONE_EXAMPLE}, not
            a zone name.
          </p>
        )}
      </FieldRow>
    </Card>
  )
}

function BrowserCard({
  time,
  controllerMs,
  now,
  browserZone,
  busy,
  onTakeOver,
}: {
  time: ControllerTime
  controllerMs: number | null
  now: number
  browserZone: string
  busy: boolean
  onTakeOver: () => void
}) {
  const sendsTime = time.source !== 'sntp'
  const nothingToSend = !sendsTime && time.timezone === browserZone
  const hint = sendsTime
    ? "Sends this browser's clock and zone in one write."
    : nothingToSend
      ? 'SNTP keeps the time; only the zone would be sent, and it already matches.'
      : 'SNTP keeps the time; only the zone is sent.'
  return (
    <Card title="This browser">
      <FieldRow label="Browser time" hint={Intl.DateTimeFormat().resolvedOptions().timeZone}>
        <span class="mono">{civilTime(now, browserUtcOffsetMinutes(now))}</span>
        {controllerMs !== null && <span class="unit">{clockSkewLabel(controllerMs, now)}</span>}
      </FieldRow>
      <FieldRow
        label="Zone as POSIX TZ"
        hint="Derived from this browser's UTC offsets over the current year, not from a zone name."
      >
        <span class="mono">{browserZone}</span>
      </FieldRow>
      <FieldRow label="Take over" hint={hint}>
        <button class="btn primary" disabled={busy || nothingToSend} onClick={onTakeOver}>
          {sendsTime ? "Use this browser's time and zone" : "Use this browser's zone"}
        </button>
      </FieldRow>
    </Card>
  )
}
