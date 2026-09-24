import { api } from '../api/client'
import type { InputDeviceSummary } from '../api/types'
import { Badge, Chip } from '../components/ui'
import { ADAPTER, ago, pad2 } from '../format'
import { useLive } from '../hooks'
import { nav } from '../router'
import { runOp } from '../toast'

function presence(d: InputDeviceSummary) {
  if (d.present) return <Chip cls="ok">yes</Chip>
  if (d.last_seen_ms === null) return <span class="faint it">not probed</span>
  return <Chip cls="idle">no</Chip>
}

function instanceTypeLabel(d: InputDeviceSummary) {
  const t = d.first_instance_type
  if (t === null || t === undefined) return <span class="faint it">not read</span>
  const name =
    t === 1
      ? '301 button'
      : t === 2
        ? '302 absolute'
        : t === 3
          ? '303 occupancy'
          : t === 4
            ? '304 light'
            : `type ${t}`
  return <Badge>{name}</Badge>
}

export function InputDevicesScreen() {
  const list = useLive<{ input_devices: InputDeviceSummary[] }>(
    () => api.inputDevices(ADAPTER),
    ['input'],
  )
  const rows = list.data?.input_devices ?? []
  const silent = rows.filter((d) => !d.present).length

  return (
    <div class="input-devices">
      <div class="crumbs">
        <a href="#/">Adapter {ADAPTER}</a> / Input devices
      </div>
      <div class="head">
        <h1>Input devices</h1>
        <span class="sub">
          {rows.length} known{silent > 0 && ` · ${silent} not answering`}
        </span>
        <span class="spacer" />
        <button
          class="btn"
          onClick={() =>
            void runOp('input scan', () => api.scanInputDevices(ADAPTER)).then(list.reload)
          }
        >
          ⌕ Scan
        </button>
        <button
          class="btn warned"
          title="Opens a 15-minute INITIALISE session and silences event traffic while it runs"
          onClick={() =>
            void runOp('input commissioning', () => api.commissionInputDevices(ADAPTER)).then(
              list.reload,
            )
          }
        >
          Commission
        </button>
      </div>
      <p class="hint">
        <b>Separate address space.</b> A control device at short address 3 and a luminaire at
        short address 3 are different units — this list never collides with Physical devices.
        Events drive local rules and, when exposed, Home Assistant.
      </p>

      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>SA</th>
              <th>Name</th>
              <th>Type</th>
              <th>Present</th>
              <th>Instances</th>
              <th>HA</th>
              <th>Last event</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((d) => (
              <tr
                key={d.short_address}
                class="click"
                onClick={() => nav(`/input-devices/${d.short_address}`)}
              >
                <td>
                  <Badge cls="addr">SA {pad2(d.short_address)}</Badge>
                </td>
                <td>
                  {d.name ? (
                    <span class="nm">{d.name}</span>
                  ) : (
                    <span class="nm faint">— unnamed</span>
                  )}
                </td>
                <td>{instanceTypeLabel(d)}</td>
                <td>{presence(d)}</td>
                <td>{d.instance_count}</td>
                <td>
                  {d.ha_expose ? <Chip cls="ok">exposed</Chip> : <Chip cls="idle">hidden</Chip>}
                </td>
                <td class="faint">
                  {d.last_event_at_ms !== null ? ago(d.last_event_at_ms) : '—'}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {list.data && rows.length === 0 && (
          <div class="empty">Nothing scanned yet — run "Scan".</div>
        )}
      </div>
    </div>
  )
}
