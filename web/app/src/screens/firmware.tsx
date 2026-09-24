import { useEffect, useState } from 'preact/hooks'

import { api } from '../api/client'
import type { FirmwareState } from '../api/types'
import { Card, FieldRow } from '../components/ui'
import { usePoll } from '../hooks'
import { bytes } from '../format'
import { errorMessage, notify } from '../toast'

const POLL_MS = 2000

const ACTIVE_POLL_MS = 1000

const URL_MAX = 96

const STATE_LABEL: Record<FirmwareState['update']['state'], string> = {
  idle: 'Idle',
  downloading: 'Downloading',
  finishing: 'Writing',
  ready_to_reboot: 'Written — rebooting',
  failed: 'Failed',
}

const WHY: Record<string, string> = {
  bad_url: 'The URL was refused before anything was fetched.',
  fetch_failed:
    'The image could not be downloaded — the server refused, went away, or sent fewer bytes than it announced. Nothing was selected for boot.',
  no_ota_slot:
    'There is no inactive slot to write into. Flashing over the wire is the only path.',
  write_failed: 'The flash write failed. The other slot is unchanged.',
  invalid_image:
    'The downloaded image is not a valid application — the controller refused to select it.',
  cancelled: 'The update was cancelled.',
}

function isActive(state: FirmwareState['update']['state']): boolean {
  return state === 'downloading' || state === 'finishing' || state === 'ready_to_reboot'
}

function urlProblem(url: string): string | null {
  if (!/^https?:\/\//.test(url)) return 'Must start with http:// or https://'
  if (url.length > URL_MAX) return `Too long: ${url.length} of ${URL_MAX} characters`
  return null
}

function Progress({ update }: { update: FirmwareState['update'] }) {
  const pct = update.percent
  return (
    <div>
      <div class="fw-progline">
        <span class="st">{STATE_LABEL[update.state]}</span>
        {pct != null && <span class="pct">{pct}%</span>}
        <span class="spacer" />
        <span class="bytes">
          {bytes(update.downloaded_bytes)}
          {update.total_bytes > 0 ? ` / ${bytes(update.total_bytes)}` : ''}
        </span>
      </div>
      <div class="fw-prog">
        <i style={{ width: `${pct ?? 0}%` }} />
      </div>
      <div class="fw-hint">
        Background polling is held off until this finishes, and the controller reboots on its
        own — it drops off the network for a few seconds.
      </div>
    </div>
  )
}

function Failure({ code }: { code: string }) {
  return (
    <div class="fw-fail">
      <div>
        <div class="code">{code}</div>
        <div class="why">{WHY[code] ?? 'The update did not complete.'}</div>
      </div>
    </div>
  )
}

function UpdateForm({
  state,
  onStarted,
}: {
  state: FirmwareState
  onStarted: () => void
}) {
  const [draft, setDraft] = useState(state.update.url)
  const [busy, setBusy] = useState(false)
  const problem = draft ? urlProblem(draft) : null
  const active = isActive(state.update.state)

  async function start() {
    setBusy(true)
    try {
      await api.startFirmwareUpdate(draft)
      notify('Update started', 'info', 'The controller is fetching the image.')
      onStarted()
    } catch (e) {
      notify('Update refused', 'failed', errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div>
      <div class="fw-urlrow">
        <input
          class={`fw-url${problem ? ' err' : ''}`}
          value={draft}
          placeholder="http://host/dali2rust.bin"
          disabled={active || busy}
          onInput={(e) => setDraft((e.target as HTMLInputElement).value)}
        />
        <button
          class="btn primary"
          disabled={active || busy || !draft || problem != null}
          onClick={start}
        >
          {state.update.state === 'failed' ? 'Retry' : 'Update'}
        </button>
      </div>
      <div class={`fw-hint${problem ? ' err' : ''}`}>
        {problem ??
          `http or https, up to ${URL_MAX} characters. The controller downloads it, writes the other slot and reboots — about a minute.`}
      </div>
    </div>
  )
}

export function FirmwareScreen() {
  const [pollMs, setPollMs] = useState<number>(POLL_MS)
  const { data, reload } = usePoll(() => api.firmware(), pollMs)
  const active = data ? isActive(data.update.state) : false
  useEffect(() => {
    setPollMs(active ? ACTIVE_POLL_MS : POLL_MS)
  }, [active])

  if (!data) return <div class="empty">Loading firmware state…</div>

  return (
    <div>
      <div class="head">
        <h1>Firmware</h1>
        <span class="sub">
          The controller fetches an image itself and reboots into it. The wire stays the way
          back if it cannot.
        </span>
      </div>

      {data.pending_verify && (
        <div class="fw-verify">
          <span class="t">
            <b>This image has not been kept yet.</b> It is proving itself; if it fails, the next
            reset returns to the previous slot.
          </span>
        </div>
      )}

      <Card title="Running image">
        <FieldRow label="Slot" hint="Which of the two app partitions is executing.">
          <span class="fw-slot">{data.running_slot || '—'}</span>
        </FieldRow>
        {!data.ota_capable && (
          <FieldRow
            label="Network update"
            hint="The partition table has a single app partition, so there is nowhere to write an update. Flashing over the wire is the only path."
          >
            <span class="fw-unavail">unavailable</span>
          </FieldRow>
        )}
      </Card>

      {isActive(data.update.state) && (
        <Card title="Update in progress">
          <Progress update={data.update} />
        </Card>
      )}

      {data.update.state === 'failed' && data.update.error && (
        <Card title="Last update">
          <Failure code={data.update.error} />
        </Card>
      )}

      {data.ota_capable && (
        <Card title="Update from a URL">
          <UpdateForm state={data} onStarted={reload} />
        </Card>
      )}
    </div>
  )
}
