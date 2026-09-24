import { api } from '../api/client'
import { Card, FieldRow, Switch, EditableText } from '../components/ui'
import { usePoll, useSettingsDraft } from '../hooks'
import { mutate } from '../toast'

export function SettingsRedundancy() {
  const { data, current, busy, setDraft, discard, save } = useSettingsDraft(() =>
    api.redundancySettings(),
  )
  const { data: state, reload: reloadState } = usePoll(() => api.redundancy())

  if (!data) return <div class="empty">Loading redundancy settings…</div>
  const s = current ?? data
  const dirty =
    s.enabled !== data.enabled ||
    s.role !== data.role ||
    s.probe_interval_ms !== data.probe_interval_ms ||
    s.takeover_after_missed !== data.takeover_after_missed ||
    s.peer_device_short_address !== data.peer_device_short_address ||
    s.peer_url !== data.peer_url

  const apply = () =>
    void save('Redundancy settings', () =>
      api.patchRedundancySettings({
        enabled: s.enabled,
        role: s.role,
        probe_interval_ms: s.probe_interval_ms,
        takeover_after_missed: s.takeover_after_missed,
        peer_device_short_address: s.peer_device_short_address,
        peer_url: s.peer_url,
      }),
    )

  const detectMs = s.takeover_after_missed * s.probe_interval_ms + 60

  const switchover = () =>
    void mutate('Switchover', async () => {
      await api.redundancySwitchover()
      reloadState()
    })

  return (
    <div>
      <div class="head">
        <h1>Redundancy</h1>
        <span class="sub">
          A second controller on this segment, silent while the first is alive. The wire
          decides which is which; the network never does.
        </span>
        <span class="spacer" />
        <button class="btn" disabled={busy || !dirty} onClick={discard}>
          Discard
        </button>
        <button class="btn primary" disabled={busy || !dirty} onClick={apply}>
          Apply
        </button>
      </div>

      <Card title="Role">
        <FieldRow
          label="Enabled"
          hint="Off: nothing probes, nothing takes over, and this controller behaves exactly as a single controller always has."
        >
          <Switch on={s.enabled} onToggle={() => setDraft({ ...s, enabled: !s.enabled })} />
        </FieldRow>
        <FieldRow
          label="This controller is a standby"
          hint="DiiA 351 §5 gives the arbitration algorithm to type B alone. On: this unit asks. Off: it is a primary — dominant, never asks and never stands down of its own accord."
        >
          <Switch
            on={s.role === 'standby'}
            onToggle={() =>
              setDraft({ ...s, role: s.role === 'standby' ? 'primary' : 'standby' })
            }
          />
        </FieldRow>
      </Card>

      <Card title="Detection">
        <FieldRow
          label="Probe interval, ms"
          hint="How often a standby asks the segment whether it still has an owner. 250–60000. One probe is ~38 ms of wire at the lowest priority, so 250 ms is already about 15 % of the bus."
        >
          <EditableText
            value={String(s.probe_interval_ms)}
            onCommit={(raw: string) => {
              const n = Number(raw.trim())
              if (!Number.isInteger(n) || n < 250 || n > 60000) return
              setDraft({ ...s, probe_interval_ms: n })
            }}
          />
        </FieldRow>
        <FieldRow
          label="Take the bus after"
          hint="Consecutive unanswered probes. Never one by default: a missed query is indistinguishable from a NO (102 §3.13 Note 1), so a single lost frame must not move the bus."
        >
          <EditableText
            value={String(s.takeover_after_missed)}
            onCommit={(raw: string) => {
              const n = Number(raw.trim())
              if (!Number.isInteger(n) || n < 1 || n > 5) return
              setDraft({ ...s, takeover_after_missed: n })
            }}
          />
        </FieldRow>
        <FieldRow label="Worst-case detection" hint="Misses × interval, plus one exchange.">
          <span class="mono">{(detectMs / 1000).toFixed(2)} s</span>
        </FieldRow>
        <FieldRow
          label="Peer short address"
          hint="The peer's Part 103 control-device address (0–63, empty = none). Needed only for a planned switchover; the probe is broadcast."
        >
          <EditableText
            value={
              s.peer_device_short_address === null ? '' : String(s.peer_device_short_address)
            }
            placeholder="—"
            onCommit={(raw: string) => {
              const t = raw.trim()
              if (t === '') return setDraft({ ...s, peer_device_short_address: null })
              const n = Number(t)
              if (!Number.isInteger(n) || n < 0 || n > 63) return
              setDraft({ ...s, peer_device_short_address: n })
            }}
          />
        </FieldRow>
      </Card>

      <Card title="Configuration pull">
        <FieldRow
          stacked
          label="Peer API address"
          hint="http://host[:port] of the other controller. A standby pulls names, bindings, schedules and rules from it every 30 s. Empty = no pull, and the standby owns an installation it knows nothing about."
        >
          <EditableText
            value={s.peer_url}
            placeholder="http://192.168.1.11"
            onCommit={(raw: string) => setDraft({ ...s, peer_url: raw.trim() })}
          />
        </FieldRow>
        <FieldRow
          label="Direction"
          hint="Only a passive unit pulls, and it never pushes. The active controller is the source, so a takeover cannot overwrite the peer with a copy of itself."
        >
          <span class="hint">Passive pulls from the active one.</span>
        </FieldRow>
      </Card>

      {state && <RedundancyState state={state} onSwitchover={switchover} busy={busy} />}
    </div>
  )
}

function RedundancyState({
  state,
  onSwitchover,
  busy,
}: {
  state: import('../api/types').RedundancyState
  onSwitchover: () => void
  busy: boolean
}) {
  return (
    <>
      <Card title="Now">
        <FieldRow label="Driving the bus" hint="103 §9.9.1 applicationActive.">
          <span class={state.active ? 'chip ok' : 'chip'}>{state.active ? 'yes' : 'no'}</span>
        </FieldRow>
        <FieldRow
          label="Answering probes"
          hint="Whether this controller would answer a peer's probe right now. False on a healthy standby — and false on a primary whose watched workers stopped turning, which is the liveness lease, visible."
        >
          <span class={state.answering ? 'chip ok' : 'chip'}>
            {state.answering ? 'yes' : 'no'}
          </span>
        </FieldRow>
        <FieldRow label="Lease left" hint="Renewed once a second while the watched workers turn.">
          <span class="mono">{state.lease_remaining_ms} ms</span>
        </FieldRow>
        <FieldRow label="Probes" hint="Answered / unanswered since boot.">
          <span class="mono">
            {state.probes.owned} owned · {state.probes.unowned} unowned
          </span>
        </FieldRow>
        <FieldRow
          label="Pulled from the peer"
          hint="Slices written locally / passes that could not reach the peer. An unreachable peer never moves the role: the arbitration worker reads no network state at all."
        >
          <span class="mono">
            {state.replication.pulled} pulled · {state.replication.peer_unreachable} unreachable
          </span>
        </FieldRow>
        <FieldRow
          label="Hand the bus to the peer"
          hint="Enables the peer, then stands this one down. The lights do not move. Only the controller holding the bus can give it away — 103 §9.9.1 leaves a passive one no legal way to take it."
        >
          <button class="btn" disabled={busy || !state.active} onClick={onSwitchover}>
            Switch over
          </button>
        </FieldRow>
      </Card>

      <Card title="Changes of hands">
        {state.transitions.length === 0 ? (
          <p class="hint">Nothing yet. A failover that happens leaves a row here.</p>
        ) : (
          <div class="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Became</th>
                  <th>Why</th>
                  <th>Took</th>
                  <th>Missed</th>
                </tr>
              </thead>
              <tbody>
                {state.transitions.map((t) => (
                  <tr key={`${t.detected_at_ms}-${t.reason}`}>
                    <td>{t.now_active ? 'active' : 'standby'}</td>
                    <td class="mono">{t.reason}</td>
                    <td class="mono">{t.took_ms} ms</td>
                    <td class="mono">{t.missed_probes}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      {!state.enabled && (
        <p class="warnbar">
          Redundancy is off: nothing probes and nothing takes over. Everything above is
          configuration for when it is on.
        </p>
      )}
    </>
  )
}
