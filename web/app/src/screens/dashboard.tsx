import { api } from '../api/client'
import type { Diagnostics, Operation, StatsReportPayload } from '../api/types'
import { BusLoadChart, busLoadMeasured, busLoadPercent } from '../components/bus-load'
import { Badge, Card, Chip } from '../components/ui'
import { ADAPTER, busHealth, opStatusChip, opTitle, uptime } from '../format'
import { useLive, useSnapshotFrames } from '../hooks'
import { runOp } from '../toast'

const RECENT_OPS = 5

export function Dashboard() {
  const pushedStats = useSnapshotFrames<StatsReportPayload>('StatsSnapshot')
  const pushedDiag = useSnapshotFrames<Diagnostics>('DiagnosticsSnapshot')
  const { data, reload } = useLive(async () => {
    const [controller, health, stats, adapters, opList, devices, diagnostics] = await Promise.all([
      api.controller(),
      api.health(),
      api.stats(),
      api.adapters(),
      api.operations(),
      api.physicalDevices(ADAPTER),
      api.diagnostics(),
    ])
    const ids = opList.operations.slice(-RECENT_OPS).reverse()
    const ops = (
      await Promise.all(ids.map((id) => api.operation(id).catch(() => null)))
    ).filter((o): o is Operation => o !== null)
    return {
      controller,
      health,
      stats,
      adapter: adapters.adapters[0],
      ops,
      diagnostics,
      deviceCount: devices.physical_devices.length,
    }
  }, ['virtual_lamps', 'physical_devices', 'stats', 'operations', 'diagnostics'], {
    onEvent: (event) => pushedStats.consume(event) || pushedDiag.consume(event),
  })

  if (!data) return <div class="empty">Loading controller…</div>
  const { controller: c, health: h, adapter: a, ops, deviceCount } = data
  const controllerStats = pushedStats.latest ?? data.stats
  const diag = pushedDiag.latest ?? data.diagnostics
  const wire = diag.dali_wire
  const bus = busHealth(wire, { enabled: a.enabled })
  const frames = wire.frames_sent_by_priority.reduce((x, y) => x + y, 0)
  const foreign = diag.phy_sniffer.forward16 + diag.phy_sniffer.forward24
  const measured = busLoadMeasured(diag)

  const discover = async () => {
    await runOp('Discover devices', () =>
      api.discoveryRun(ADAPTER, 'scan_known_short_addresses'),
    )
    void reload()
  }

  return (
    <>
      <div class="head">
        <h1>dali2rust · Controller</h1>
        <Badge>
          {c.firmware_version} · {c.target_mcu}
        </Badge>
        <Chip cls={h.status === 'ok' ? 'ok' : h.status === 'degraded' ? 'warn' : 'err'}>
          {h.status === 'ok' ? 'Online' : h.status}
        </Chip>
        <span class="spacer" />
        <span class="sub">
          uptime{' '}
          <span class="mono" style="color:var(--text-muted)">
            {uptime(controllerStats.controller.uptime_ms)}
          </span>
        </span>
      </div>

      <div class="grid2">
        <Card
          title="DALI bus"
          action={
            <a class="act" href="#/diagnostics" style="text-decoration:none">
              Counters →
            </a>
          }
        >
          <div class={`bushealth ${bus.cls}`}>
            <span class="ring">
              {bus.cls === 'ok' ? '✓' : bus.cls === 'idle' ? '·' : bus.cls === 'warn' ? '◔' : '⚠'}
            </span>
            <span class="txt">
              <div class="st">{bus.label}</div>
              <div class="d">{bus.detail}</div>
            </span>
            <span class="load">
              <div class="n">
                {measured ? busLoadPercent(wire.load_permille) : '—'}
                {measured ? <span class="u">%</span> : null}
              </div>
              <div class="l">bus load</div>
              <div class="s">
                {measured ? `own ${busLoadPercent(wire.load_own_permille)}%` : 'not measured'}
              </div>
            </span>
          </div>
          <div class="buscounts">
            <div class="c">
              <div class="n">{frames}</div>
              <div class="l">frames</div>
            </div>
            <div class="c">
              <div class="n">{foreign}</div>
              <div class="l">foreign</div>
            </div>
            <div class="c">
              <div class={`n${wire.collisions > 0 ? ' warn' : ''}`}>{wire.collisions}</div>
              <div class="l">collisions</div>
            </div>
            <div class="c">
              <div class={`n${wire.bus_power_down_entries > 0 ? ' err' : ''}`}>
                {wire.bus_power_down_entries}
              </div>
              <div class="l">power down</div>
            </div>
          </div>
          <BusLoadChart diag={diag} />
        </Card>

        <Card title="Controller">
          <div class="attr">
            <span class="k">Target MCU</span>
            <span class="v">{c.target_mcu}</span>
            <span />
          </div>
          <div class="attr">
            <span class="k">Registry</span>
            <span class="v">
              <Chip cls={c.hydrated ? 'ok' : 'warn'}>{c.hydrated ? 'hydrated' : 'not hydrated'}</Chip>
            </span>
            <span />
          </div>
          <div class="attr">
            <span class="k">Hostname</span>
            <span class="v">{c.network.hostname || '—'}</span>
            <span />
          </div>
          <div class="attr">
            <span class="k">IP address</span>
            <span class="v">{c.network.ip || '—'}</span>
            <span />
          </div>
          <div class="attr">
            <span class="k">Home Assistant</span>
            <span class="v">
              {c.home_assistant.enabled
                ? c.home_assistant.connected
                  ? 'connected'
                  : 'enabled'
                : 'disabled'}
              {c.home_assistant.broker_url ? (
                <span class="faint"> {c.home_assistant.broker_url}</span>
              ) : null}
            </span>
            <span />
          </div>
        </Card>

        <Card
          title={`Adapter ${a.adapter_id} · ${a.name}`}
          action={
            <Chip cls={a.enabled ? 'ok' : 'idle'}>
              {a.enabled ? 'enabled' : 'disabled'}
            </Chip>
          }
        >
          <div class="counters">
            <div class="counter">
              <div class="n">{a.counters.commands}</div>
              <div class="l">commands</div>
            </div>
            <div class="counter">
              <div class={`n${a.counters.timeouts > 0 ? ' warn' : ''}`}>{a.counters.timeouts}</div>
              <div class="l">timeouts</div>
            </div>
            <div class="counter">
              <div class={`n${a.counters.errors > 0 ? ' err' : ''}`}>{a.counters.errors}</div>
              <div class="l">errors</div>
            </div>
          </div>
          <div class="attr">
            <span class="k">Physical devices</span>
            <span class="v">
              {deviceCount} <span class="unit">discovered</span>
            </span>
            <span />
          </div>
          <div class="attr">
            <span class="k">Capacity</span>
            <span class="v">
              {a.limits.virtual_lamps} VL · {a.limits.groups} groups · {a.limits.scenes} scenes
            </span>
            <span />
          </div>
        </Card>

        <Card title="Quick actions">
          <div class="actions">
            <button class="btn primary" onClick={discover}>
              ⌕ Discover devices
            </button>
            <a class="btn" href="#/groups" style="text-decoration:none">
              ⊞ Open groups
            </a>
            <a class="btn" href="#/scenes" style="text-decoration:none">
              ✦ Open scenes
            </a>
          </div>
        </Card>

        <Card
          title="Recent operations"
          action={
            <a class="act" href="#/operations" style="text-decoration:none">
              All operations →
            </a>
          }
        >
          {ops.length === 0 ? (
            <div class="empty">No tracked operations — finished operations evict after 60 s.</div>
          ) : (
            ops.map((op) => {
              const chip = opStatusChip(op.status)
              return (
                <a class="oprow" href={`#/operations/${op.operation_id}`} key={op.operation_id}>
                  <span class="t">{opTitle(op)}</span>
                  <span class="id">{op.operation_id}</span>
                  <Chip cls={chip.cls} spin={chip.spin}>
                    {op.status.replace('_', ' ')}
                  </Chip>
                </a>
              )
            })
          )}
        </Card>
      </div>
    </>
  )
}
