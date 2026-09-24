import { useState } from 'preact/hooks'
import { ensureProductsLoaded, productName } from '../products'

import { api } from '../api/client'
import type { DiscoveryMode, PhysicalDeviceSummary, VirtualLamp } from '../api/types'
import { Badge, Chip, LampState } from '../components/ui'
import {
  ADAPTER,
  ago,
  pad2,
  registerDeviceNow,
  statusSummary,
  typeLabel,
} from '../format'
import { nav } from '../router'
import { runOp } from '../toast'
import { useLive } from '../hooks'

function DeviceRow({ dev, lamp }: { dev: PhysicalDeviceSummary; lamp: VirtualLamp | undefined }) {
  const t = typeLabel(dev.device_type_effective, dev.color_mode_effective, dev.capabilities)
  const status = statusSummary(dev.state)
  const seen = dev.state.last_seen_ms
  const productLabel = productName(dev.gtin ?? null)
  return (
    <tr class="click" onClick={() => nav(`/devices/${dev.short_address}`)}>
      <td>
        <Badge cls="addr">SA {pad2(dev.short_address)}</Badge>
      </td>
      <td>
        {dev.name ? (
          <span class="nm">{dev.name}</span>
        ) : productLabel ? (
          <span class="nm faint">{productLabel}</span>
        ) : (
          <span class="nm faint">— unnamed</span>
        )}
      </td>
      <td>
        <Badge cls={t.dt8 ? 'dt8' : undefined}>{t.label}</Badge>
      </td>
      <td>
        <LampState state={dev.state} />
      </td>
      <td>
        {status === null ? (
          <span class="ok-txt">—</span>
        ) : status.cls === 'ok-txt' ? (
          <span class="ok-txt">OK</span>
        ) : (
          <Chip cls={status.cls}>{status.label}</Chip>
        )}
      </td>
      <td>
        {lamp ? (
          <span class="vl">
            VL {pad2(lamp.virtual_lamp_id)}
            {lamp.name && <span class="vname"> · {lamp.name}</span>}
          </span>
        ) : (
          <span class="vl none">unbound</span>
        )}
      </td>
      <td>
        {seen == null ? (
          <span class="seen never">never</span>
        ) : (
          <span class="seen">{ago(seen)}</span>
        )}
      </td>
    </tr>
  )
}

export function Devices() {
  ensureProductsLoaded()
  const { data, reload } = useLive(async () => {
    const [devices, lamps] = await Promise.all([
      api.physicalDevices(ADAPTER),
      api.virtualLamps(ADAPTER),
    ])
    registerDeviceNow(devices.now_ms)
    return { devices: devices.physical_devices, lamps: lamps.virtual_lamps }
  }, ['physical_devices', 'virtual_lamps'])

  const [commissioning, setCommissioning] = useState(false)
  const runDiscovery = async (title: string, mode: DiscoveryMode) => {
    setCommissioning(true)
    try {
      await runOp(title, () => api.discoveryRun(ADAPTER, mode))
    } finally {
      setCommissioning(false)
    }
    void reload()
  }
  const scan = () => runDiscovery('Scan bus', 'scan_known_short_addresses')
  const commission = () => runDiscovery('Commission unaddressed', 'commission_unaddressed')

  const boundBySa = new Map<number, VirtualLamp>()
  for (const lamp of data?.lamps ?? []) {
    const sa = lamp.binding?.physical_short_address
    if (sa != null) boundBySa.set(sa, lamp)
  }

  return (
    <>
      <div class="crumbs">
        <a href="#/">Adapter {ADAPTER}</a> / Physical devices
      </div>
      {commissioning && (
        <div class="warnbar">
          Commissioning the bus — level and colour commands wait until it finishes.
          It cannot be interrupted safely.
        </div>
      )}
      <div class="head">
        <h1>Physical devices</h1>
        <span class="sub">{data ? `${data.devices.length} discovered` : '…'}</span>
        <span class="spacer" />
        <button class="btn" onClick={commission}>
          Commission unaddressed
        </button>
        <button class="btn primary" onClick={scan}>
          ⌕ Scan bus
        </button>
      </div>

      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>SA</th>
              <th>Name</th>
              <th>Type</th>
              <th>State</th>
              <th>Status</th>
              <th>Bound VL</th>
              <th>Last seen</th>
            </tr>
          </thead>
          <tbody>
            {data?.devices.map((dev) => (
              <DeviceRow
                key={dev.short_address}
                dev={dev}
                lamp={boundBySa.get(dev.short_address)}
              />
            ))}
          </tbody>
        </table>
        {data && data.devices.length === 0 && (
          <div class="empty">No devices discovered yet — run "Scan bus".</div>
        )}
      </div>
    </>
  )
}
