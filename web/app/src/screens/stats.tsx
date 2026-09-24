import { api } from '../api/client'
import type { StatsReportPayload } from '../api/types'
import { busLoadPercent } from '../components/bus-load'
import { Badge, Card, Chip } from '../components/ui'
import { CounterRows, useDeltas } from '../counters'
import { uptime } from '../format'
import { useLive, useSnapshotFrames } from '../hooks'

const STATS_POLL_MS = 5000

const INTERNAL_PRESSURE_ALERT_BYTES = 12 * 1024

const FAULT_KEYS = new Set([
  'commands_ingress_overflow_total',
  'errors_total',
  'confirmation_timeouts_total',
  'failed_total',
  'timed_out_total',
  'events_dropped_total',
])

const OPERATION_GAUGES = ['running'] as const

const WEBSOCKET_GAUGES = ['clients'] as const

const DALI_GAUGES = [
  'wire_load_permille',
  'wire_load_own_permille',
  'isr_late_ticks_total',
  'isr_max_gap_us',
] as const

const RULES_GAUGES = [
  'timers_active',
  'rules_loaded',
  'vars_in_use',
  'latency_p50_ms',
  'latency_p95_ms',
  'latency_max_ms',
] as const

function isFault(key: string, value: number): boolean {
  return FAULT_KEYS.has(key) && value > 0
}

function kib(bytes: number): string {
  return `${(bytes / 1024).toFixed(1)}`
}

function MemoryCard({ c }: { c: StatsReportPayload['controller'] }) {
  if (c.internal_free_bytes === null) return null
  const tight =
    c.internal_min_free_bytes !== null &&
    c.internal_min_free_bytes < INTERNAL_PRESSURE_ALERT_BYTES
  return (
    <Card title="Memory" span2>
      <div class="attr">
        <span class="k">Internal free</span>
        <span class="v">
          {c.internal_free_bytes.toLocaleString()} <span class="unit">B</span>
        </span>
        <span class="delta" />
      </div>
      {c.internal_largest_block_bytes !== null && (
        <div class="attr">
          <span class="k">
            Internal largest block <span class="unit">@boot</span>
          </span>
          <span class="v">
            {c.internal_largest_block_bytes.toLocaleString()} <span class="unit">B</span>
          </span>
          <span class="delta" />
        </div>
      )}
      {c.internal_min_free_bytes !== null && (
        <div class="attr">
          <span class="k">Internal min ever</span>
          <span class={`v${tight ? ' warn' : ''}`}>
            {c.internal_min_free_bytes.toLocaleString()} <span class="unit">B</span>
          </span>
          <span class="delta" />
        </div>
      )}
      {c.free_heap_bytes !== null && (
        <div class="attr sub">
          <span class="k">Total heap free (incl. PSRAM)</span>
          <span class="v">
            {c.free_heap_bytes.toLocaleString()} <span class="unit">B</span>
          </span>
          <span class="delta" />
        </div>
      )}
    </Card>
  )
}

function Tile({
  label,
  value,
  unit,
  foot,
  warn,
}: {
  label: string
  value: string
  unit?: string
  foot: string
  warn?: boolean
}) {
  return (
    <div class="tile">
      <div class="l">{label}</div>
      <div class={`n${warn ? ' warn' : ''}`}>
        {value}
        {unit ? <span class="u">{unit}</span> : null}
      </div>
      <div class="foot">{foot}</div>
    </div>
  )
}

export function StatsScreen() {
  const snapshot = useSnapshotFrames<StatsReportPayload>('StatsSnapshot')
  const { data: polled, error } = useLive(() => api.stats(), ['stats'], {
    intervalMs: STATS_POLL_MS,
    onEvent: snapshot.consume,
  })
  const data = snapshot.latest ?? polled
  const deltas = useDeltas(data, data?.sample_ms ?? null)

  if (error) return <div class="empty">Stats unavailable — {error}</div>
  if (!data) return <div class="empty">Loading stats…</div>

  const c = data.controller
  const ops = data.operations
  const memoryFoot =
    c.internal_largest_block_bytes !== null
      ? `largest block ${kib(c.internal_largest_block_bytes)} KiB @boot`
      : 'no allocator port on this build'

  return (
    <>
      <div class="head">
        <h1>Stats</h1>
        <Badge>read model</Badge>
        <span class="spacer" />
        <span class="sub">
          sampled every{' '}
          <span class="mono" style="color:var(--text-muted)">
            {STATS_POLL_MS / 1000} s
          </span>
        </span>
      </div>

      <div class="hint">
        Product totals, not raw per-path counters — those live on Diagnostics. Everything
        suffixed <em>total</em> is free-running and 32-bit: it wraps rather than resets, so the
        right-hand column (change since the previous sample) is what says whether something is
        happening now.
      </div>

      <div class="tiles">
        <Tile label="Uptime" value={uptime(c.uptime_ms)} foot="since boot, from the Clock port" />
        {c.internal_free_bytes !== null ? (
          <Tile
            label="Internal SRAM free"
            value={kib(c.internal_free_bytes)}
            unit="KiB"
            foot={memoryFoot}
            warn={c.internal_free_bytes < INTERNAL_PRESSURE_ALERT_BYTES}
          />
        ) : (
          <Tile label="Internal SRAM free" value="—" foot="not measured on this build" />
        )}
        <Tile
          label="Operations running"
          value={ops.running.toLocaleString()}
          foot={`${ops.succeeded_total.toLocaleString()} succeeded · ${ops.failed_total.toLocaleString()} failed`}
        />
        <Tile
          label="DALI bus load"
          value={busLoadPercent(data.dali.wire_load_permille).toLocaleString()}
          unit="%"
          foot={`own ${busLoadPercent(data.dali.wire_load_own_permille)}% · ${data.dali.foreign_frames_total.toLocaleString()} foreign frames`}
        />
      </div>

      <div class="grid2">
        <MemoryCard c={c} />

        <Card title="Bus">
          <CounterRows block={data.bus} path="bus" deltas={deltas} isFault={isFault} />
        </Card>

        <Card title="DALI">
          <CounterRows
            block={data.dali}
            path="dali"
            deltas={deltas}
            isFault={isFault}
            gauges={DALI_GAUGES}
          />
        </Card>

        <Card title="Operations" span2>
          <CounterRows
            block={data.operations}
            path="operations"
            deltas={deltas}
            isFault={isFault}
            gauges={OPERATION_GAUGES}
          />
        </Card>

        <Card title="WebSocket" span2>
          <CounterRows
            block={data.websocket}
            path="websocket"
            deltas={deltas}
            isFault={isFault}
            gauges={WEBSOCKET_GAUGES}
          />
        </Card>

        <Card
          title="Home Assistant bridge"
          action={
            <Chip cls={data.mqtt.connected ? 'ok' : 'idle'}>
              {data.mqtt.connected ? 'connected' : 'disconnected'}
            </Chip>
          }
        >
          <CounterRows
            block={{
              publishes_total: data.mqtt.publishes_total,
              publish_failures_total: data.mqtt.publish_failures_total,
            }}
            path="mqtt"
            deltas={deltas}
            isFault={isFault}
          />
        </Card>

        <Card title="Input devices">
          <CounterRows block={data.input} path="input" deltas={deltas} isFault={isFault} />
        </Card>

        <Card title="Rules engine" span2>
          <CounterRows
            block={data.rules}
            path="rules"
            deltas={deltas}
            isFault={isFault}
            gauges={RULES_GAUGES}
          />
        </Card>

        {data.network && (
          <Card title="Network">
            <CounterRows block={data.network} path="network" deltas={deltas} isFault={isFault} />
          </Card>
        )}
      </div>
    </>
  )
}
