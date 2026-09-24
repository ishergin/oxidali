import { useState } from 'preact/hooks'
import { api } from '../api/client'
import type { HomeAssistantSettings, HomeAssistantSettingsPatch } from '../api/types'
import { Card, EditableText, FieldRow, Switch } from '../components/ui'
import { usePoll } from '../hooks'
import { errorMessage, mutateBusy, notify, opCommitted, trackOp } from '../toast'

const POLL_MS = 5000
const MAX_HOST = 64
const MAX_USERNAME = 32
const MAX_PASSWORD = 48
const MAX_PREFIX = 32
const MAX_CONTROLLER_ID = 32

const TOPIC_SAFE = /^[A-Za-z0-9_-]+$/

type Draft = HomeAssistantSettings & { broker_password?: string }

const clampPort = (raw: string, fallback: number) => {
  const n = Number.parseInt(raw, 10)
  return Number.isNaN(n) || n < 1 || n > 65535 ? fallback : n
}

function patchBody(next: Draft, base: HomeAssistantSettings): HomeAssistantSettingsPatch {
  const body: HomeAssistantSettingsPatch = {}
  const keys: (keyof HomeAssistantSettings)[] = [
    'enabled',
    'broker_host',
    'broker_port',
    'broker_username',
    'discovery_prefix',
    'state_topic_prefix',
    'controller_id',
    'publish_qos',
    'retain_state',
    'retain_discovery',
    'expose_input_devices',
  ]
  for (const k of keys) if (next[k] !== base[k]) Object.assign(body, { [k]: next[k] })
  if (next.broker_password) body.broker_password = next.broker_password
  return body
}

export function SettingsHomeAssistant() {
  const { data, reload } = usePoll(() => api.homeAssistantSettings(), POLL_MS)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [busy, setBusy] = useState(false)

  if (!data) return <div class="empty">Loading Home Assistant settings…</div>
  const s: Draft = draft ?? data
  const edit = (patch: Partial<Draft>) => setDraft({ ...s, ...patch })

  const body = patchBody(s, data)
  const dirty = Object.keys(body).length
  const idBad = !TOPIC_SAFE.test(s.controller_id) || s.controller_id.length > MAX_CONTROLLER_ID

  const save = async () => {
    await mutateBusy('Home Assistant', setBusy, () => api.patchHomeAssistantSettings(body), () => {
      setDraft(null)
      reload()
      notify('Home Assistant', 'succeeded', 'Applied')
    })
  }

  const republish = async () => {
    setBusy(true)
    try {
      const accepted = await api.publishHomeAssistantDiscovery()
      const op = await trackOp('Discovery', accepted)
      if (!opCommitted(op)) {
        notify('Discovery', 'failed', 'The republish did not complete')
      }
    } catch (e) {
      notify('Discovery', 'failed', errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div>
      <div class="head">
        <h1>Home Assistant</h1>
        <span class="sub">
          The controller publishes itself over MQTT and Home Assistant discovers it. Nothing
          is announced while this is off.
        </span>
        <span class="spacer" />
        <button class="btn" disabled={busy || !dirty} onClick={() => setDraft(null)}>
          Discard
        </button>
        <button
          class="btn primary"
          disabled={busy || !dirty || idBad}
          onClick={() => void save()}
        >
          {dirty ? `Apply (${dirty})` : 'Apply'}
        </button>
      </div>

      <Card title="Integration">
        <FieldRow
          label="Enabled"
          hint="Off by default. Nothing is announced to Home Assistant while this is off."
        >
          <Switch on={s.enabled} onToggle={() => edit({ enabled: !s.enabled })} />
        </FieldRow>
        <FieldRow label="Connection" hint={data.broker_url_view || 'No broker configured yet.'}>
          <span class={data.enabled ? 'chip' : 'chip off'}>
            {data.enabled ? 'enabled' : 'disabled'}
          </span>
        </FieldRow>
      </Card>

      <Card title="Broker">
        <FieldRow label="Host">
          <EditableText
            cls="txt"
            value={s.broker_host}
            onCommit={(raw) => edit({ broker_host: raw.slice(0, MAX_HOST) })}
          />
        </FieldRow>
        <FieldRow label="Port">
          <EditableText
            cls="num"
            inputMode="numeric"
            value={String(s.broker_port)}
            onCommit={(raw) => edit({ broker_port: clampPort(raw, s.broker_port) })}
          />
        </FieldRow>
        <FieldRow label="Username">
          <EditableText
            cls="txt"
            value={s.broker_username}
            onCommit={(raw) => edit({ broker_username: raw.slice(0, MAX_USERNAME) })}
          />
        </FieldRow>
        <FieldRow
          label="Password"
          hint={
            s.broker_password_set
              ? 'A password is stored. Leave blank to keep it.'
              : 'No password stored.'
          }
        >
          <EditableText
            cls="txt"
            value={s.broker_password ?? ''}
            onCommit={(raw) => edit({ broker_password: raw.slice(0, MAX_PASSWORD) })}
          />
        </FieldRow>
      </Card>

      <Card title="Topics and identity">
        <FieldRow label="Discovery prefix">
          <EditableText
            cls="txt"
            value={s.discovery_prefix}
            onCommit={(raw) => edit({ discovery_prefix: raw.slice(0, MAX_PREFIX) })}
          />
        </FieldRow>
        <FieldRow label="State prefix">
          <EditableText
            cls="txt"
            value={s.state_topic_prefix}
            onCommit={(raw) => edit({ state_topic_prefix: raw.slice(0, MAX_PREFIX) })}
          />
        </FieldRow>
        <FieldRow
          label="Controller ID"
          hint="Roots every entity's identity. Changing it after publishing orphans every entity in Home Assistant. Letters, digits, - and _ only."
        >
          <EditableText
            cls={idBad ? 'txt err' : 'txt'}
            value={s.controller_id}
            onCommit={(raw) => edit({ controller_id: raw.slice(0, MAX_CONTROLLER_ID) })}
          />
        </FieldRow>
      </Card>

      <Card title="Publishing">
        <FieldRow label="QoS" hint="0 or 1; exactly-once has no place on a bridge that republishes state on every commit.">
          <EditableText
            cls="num"
            inputMode="numeric"
            value={String(s.publish_qos)}
            onCommit={(raw) => edit({ publish_qos: raw.trim() === '0' ? 0 : 1 })}
          />
        </FieldRow>
        <FieldRow label="Retain state">
          <Switch on={s.retain_state} onToggle={() => edit({ retain_state: !s.retain_state })} />
        </FieldRow>
        <FieldRow label="Retain discovery">
          <Switch
            on={s.retain_discovery}
            onToggle={() => edit({ retain_discovery: !s.retain_discovery })}
          />
        </FieldRow>
        <FieldRow
          label="Expose input devices"
          hint="Panels and sensors as HA entities; each device also has its own switch."
        >
          <Switch
            on={s.expose_input_devices}
            onToggle={() => edit({ expose_input_devices: !s.expose_input_devices })}
          />
        </FieldRow>
        <FieldRow
          label="Discovery"
          hint="Runs as an operation; the controller paces the announcement."
        >
          <button class="btn" disabled={busy || !data.enabled} onClick={() => void republish()}>
            Republish all entities
          </button>
        </FieldRow>
      </Card>
    </div>
  )
}
