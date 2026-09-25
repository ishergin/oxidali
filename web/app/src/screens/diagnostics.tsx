import { api } from '../api/client'
import type {
  DaliWireCounters,
  Diagnostics,
  MqttBridgeCounters,
  RedundancyCounters,
  SubscriberCounters,
} from '../api/types'
import { Badge, Card, Chip } from '../components/ui'
import { CounterRows, type Flat, useDeltas } from '../counters'
import { uptime } from '../format'
import { useLive, useSnapshotFrames } from '../hooks'

const DIAG_POLL_MS = 2000

const RULES_GAUGES = ['stat_counts', 'log_lines'] as const

const FAULT_KEYS = new Set([
  'ingress_overflow',
  'oversize_rejected',
  'kind_mismatch',
  'commands_unrouted',
  'delivery_rejected_dropped',
  'invalid_command',
  'execution_failed',
  'confirmation_publish_failed',
  'event_publish_failed',
  'read_attributes_contended_aborts',
  'read_attributes_transport_aborts',
  'read_attributes_sequence_incomplete',
  'discovery_device_type_degraded',
  'hydrate_failed',
  'persist_failed',
  'effects_ingress_rejected',
  'cell_busy',
  'aborted',
  'window_closed',
  'late',
  'probe_failed',
  'handover_incomplete',
  'outstanding_expired',
  'publish_failed',
  'flush_error_total',
  'no_space_total',
  'hydrate_error_total',
  'decode_failed',
  'unsupported_len',
  'dropped',
  'skipped_unknown_observed',
  'events_dropped_total',
  'inbox_overflow_total',
  'sniffer_dropped_total',
  'transaction_budget_exceeded',
  'transaction_leaks',
  'config_write_signal_publish_failed',
  'terminal_signal_publish_failed',
  'terminal_event_publish_failed_total',
  'evidence_publish_failed',
  'pending_outcomes_expired',
  'health_probes_expired',
  'ignored_commands',
  'ignored_events',
])

const WEBSOCKET_GAUGES = ['clients'] as const

const POLLER_GAUGES = ['targets_excluded'] as const

const DALI_WIRE_GAUGES = [
  'load_permille',
  'load_own_permille',
  'bus_power_down_active',
  'system_failure_active',
] as const

function isFault(key: string, value: number): boolean {
  return FAULT_KEYS.has(key) && value > 0
}

function SubscriberRows({
  title,
  subs,
  path,
  deltas,
}: {
  title: string
  subs: (SubscriberCounters & { name?: string })[]
  path: string
  deltas: Flat
}) {
  if (subs.length === 0) return null
  return (
    <>
      {subs.map((s, i) => {
        const overflowed = s.receiver_overflow > 0
        const delta = deltas[`${path}[${i}].delivered`] ?? 0
        return (
          <div class="attr" key={`${path}-${i}`}>
            <span class="k">
              {title} <span class="ro">{s.name ? s.name : `#${i}`}</span>
            </span>
            <span class={`v${overflowed ? ' fault' : ''}`}>
              {s.delivered.toLocaleString()}
              {overflowed ? ` · ${s.receiver_overflow} lost` : ''}
            </span>
            <span class={`delta${delta > 0 ? ' live' : ''}`}>{delta > 0 ? `+${delta}` : ''}</span>
          </div>
        )
      })}
    </>
  )
}

function SpreadRow({ label: name, cells }: { label: string; cells: [string, number][] }) {
  return (
    <div class="attr spread">
      <span class="k">{name}</span>
      <span class="v">
        {cells.map(([tag, value]) => (
          <span class="cell" key={tag}>
            <i>{tag}</i>
            {value.toLocaleString()}
          </span>
        ))}
      </span>
    </div>
  )
}

function DaliWireCard({
  wire,
  deltas,
}: {
  wire: DaliWireCounters
  deltas: Flat
}) {
  const {
    frames_sent_by_priority: byPriority,
    transactions_by_class: byClass,
    ...flat
  } = wire
  return (
    <Card title="DALI wire">
      <CounterRows
        block={flat}
        path="dali_wire"
        deltas={deltas}
        isFault={isFault}
        gauges={DALI_WIRE_GAUGES}
      />
      <SpreadRow
        label="Frames by priority"
        cells={byPriority.map((v, i) => [`P${i + 1}`, v] as [string, number])}
      />
      <SpreadRow
        label="Transactions by class"
        cells={['user', 'conf', 'auto', 'query'].map(
          (tag, i) => [tag, byClass[i]] as [string, number],
        )}
      />
    </Card>
  )
}

function MqttCard({ mqtt, deltas }: { mqtt: MqttBridgeCounters; deltas: Flat }) {
  const { connected, ...totals } = mqtt
  return (
    <Card title="MQTT bridge" span2>
      <div class="attr">
        <span class="k">Connected</span>
        <span class={`v${connected ? '' : ' fault'}`}>{connected ? 'yes' : 'no'}</span>
        <span class="delta" />
      </div>
      <CounterRows block={totals} path="mqtt" deltas={deltas} isFault={isFault} />
    </Card>
  )
}

function RedundancyCard({
  redundancy,
  deltas,
}: {
  redundancy: RedundancyCounters
  deltas: Flat
}) {
  const { armed, ...totals } = redundancy
  return (
    <Card title="Redundancy">
      <div class="attr">
        <span class="k">Arbitration table</span>
        <span class="v">{armed ? 'armed' : 'idle'}</span>
        <span class="delta" />
      </div>
      <CounterRows block={totals} path="redundancy" deltas={deltas} isFault={isFault} />
    </Card>
  )
}

export function DiagnosticsScreen() {
  const snapshot = useSnapshotFrames<Diagnostics>('DiagnosticsSnapshot')
  const { data: polled, error } = useLive(() => api.diagnostics(), ['diagnostics'], {
    intervalMs: DIAG_POLL_MS,
    onEvent: snapshot.consume,
  })
  const data = snapshot.latest ?? polled
  const deltas = useDeltas(data, data?.uptime_ms ?? null)

  if (!data) {
    return (
      <div class="empty">
        {error ? `Diagnostics unavailable — ${error}` : 'Loading diagnostics…'}
      </div>
    )
  }

  const bus = data.bus
  const channels = [
    ['Commands', 'bus.commands', bus.commands],
    ['Confirmations', 'bus.confirmations', bus.confirmations],
    ['Events', 'bus.events', bus.events],
  ] as const

  return (
    <>
      <div class="head">
        <h1>Diagnostics</h1>
        <Badge>counters</Badge>
        {error && (
          <Chip cls="warn" title={error}>
            stale — last poll failed
          </Chip>
        )}
        <span class="spacer" />
        <span class="sub">
          uptime <span class="mono" style="color:var(--text-muted)">{uptime(data.uptime_ms)}</span>
        </span>
      </div>

      <div class="hint">
        Counters are free-running and never reset — the right-hand column is the change since
        the previous sample ({DIAG_POLL_MS / 1000} s), which is what tells you whether something
        is happening now. Worker counters are 32-bit and wrap.
      </div>

      <div class="grid2">
        {channels.map(([title, path, counters]) => (
          <Card title={`Bus · ${title}`} key={path}>
            <CounterRows block={counters} path={path} deltas={deltas} isFault={isFault} />
          </Card>
        ))}

        <Card title="Bus · routing">
          <CounterRows
            block={{
              commands_unrouted: bus.commands_unrouted,
              delivery_rejected_dropped: bus.delivery_rejected_dropped,
            }}
            path="bus"
            deltas={deltas}
            isFault={isFault}
          />
          <CounterRows
            block={data.confirmation_bridge}
            path="confirmation_bridge"
            deltas={deltas}
            isFault={isFault}
          />
        </Card>

        <Card title="Subscriber inboxes" span2>
          <SubscriberRows
            title="Command"
            subs={bus.command_subscribers}
            path="bus.command_subscribers"
            deltas={deltas}
          />
          <SubscriberRows
            title="Confirmation"
            subs={bus.confirmation_subscribers}
            path="bus.confirmation_subscribers"
            deltas={deltas}
          />
          <SubscriberRows
            title="Event"
            subs={bus.event_subscribers}
            path="bus.event_subscribers"
            deltas={deltas}
          />
        </Card>

        <Card title="DALI worker">
          <CounterRows block={data.dali_worker} path="dali_worker" deltas={deltas} isFault={isFault} />
        </Card>

        <DaliWireCard wire={data.dali_wire} deltas={deltas} />

        <Card title="PHY sniffer">
          <CounterRows block={data.phy_sniffer} path="phy_sniffer" deltas={deltas} isFault={isFault} />
        </Card>

        <Card title="Sniffer translator">
          <CounterRows
            block={data.sniffer_translator}
            path="sniffer_translator"
            deltas={deltas}
            isFault={isFault}
          />
        </Card>

        <Card title="State projector">
          <CounterRows block={data.projector} path="projector" deltas={deltas} isFault={isFault} />
        </Card>

        <Card title="Registry">
          <CounterRows block={data.registry} path="registry" deltas={deltas} isFault={isFault} />
        </Card>

        <Card title="Apply orchestrator">
          <CounterRows
            block={data.apply_orchestrator}
            path="apply_orchestrator"
            deltas={deltas}
            isFault={isFault}
          />
        </Card>

        <Card title="Operations">
          <CounterRows block={data.operations} path="operations" deltas={deltas} isFault={isFault} />
        </Card>

        <Card title="HCL scheduler">
          <CounterRows block={data.hcl} path="hcl" deltas={deltas} isFault={isFault} />
        </Card>

        <Card title="Poller">
          <CounterRows
            block={data.poller}
            path="poller"
            deltas={deltas}
            isFault={isFault}
            gauges={POLLER_GAUGES}
          />
        </Card>

        <Card title="WebSocket">
          <CounterRows
            block={data.websocket}
            path="websocket"
            deltas={deltas}
            isFault={isFault}
            gauges={WEBSOCKET_GAUGES}
          />
        </Card>

        <MqttCard mqtt={data.mqtt} deltas={deltas} />

        <RedundancyCard redundancy={data.redundancy} deltas={deltas} />

        <Card title="Rules engine" span2>
          <CounterRows
            block={data.rules}
            path="rules"
            deltas={deltas}
            isFault={isFault}
            gauges={RULES_GAUGES}
          />
        </Card>

        <Card title="Persistence" span2>
          <CounterRows block={data.persistence} path="persistence" deltas={deltas} isFault={isFault} />
        </Card>
      </div>
    </>
  )
}
