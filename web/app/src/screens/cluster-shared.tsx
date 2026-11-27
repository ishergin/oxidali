import type { ComponentChildren } from 'preact'
import { useEffect, useState } from 'preact/hooks'
import { ApiError, api } from '../api/client'
import { errorMessage } from '../toast'
import { type Option, type Refusal, type RefusalScope, refusalOf } from './cluster-view'

export function refusalFrom(e: unknown, scope: RefusalScope = 'settings'): Refusal {
  if (e instanceof ApiError) return refusalOf(e.status, e.code, e.message, scope)
  return { field: null, text: errorMessage(e) }
}

function selClass(bad: boolean, dirty: boolean): string {
  if (bad) return 'sel err'
  return dirty ? 'sel dirty' : 'sel'
}

export function Select({
  value,
  options,
  disabled,
  bad,
  dirty = false,
  label,
  onPick,
}: {
  value: number | null
  options: Option[]
  disabled: boolean
  bad: boolean
  dirty?: boolean
  label: string
  onPick: (value: number | null) => void
}) {
  return (
    <select
      class={selClass(bad, dirty)}
      aria-label={label}
      disabled={disabled}
      value={value === null ? '' : String(value)}
      onChange={(e) => {
        const raw = e.currentTarget.value
        onPick(raw === '' ? null : Number(raw))
      }}
    >
      {options.map((o) => (
        <option key={o.label} value={o.value === null ? '' : String(o.value)}>
          {o.label}
        </option>
      ))}
    </select>
  )
}

export function StandbyBanner({ peerUrl, children }: { peerUrl: string | null; children: ComponentChildren }) {
  return (
    <p class="req">
      {children}{' '}
      {peerUrl ? (
        <a href={peerUrl} target="_blank" rel="noopener noreferrer">
          Open the active board
        </a>
      ) : (
        'Open the active board'
      )}{' '}
      to change them.
    </p>
  )
}

export function usePeerUrl(readOnly: boolean): string | null {
  const [peer, setPeer] = useState<string | null>(null)
  useEffect(() => {
    if (!readOnly) return
    api
      .redundancySettings()
      .then((r) => setPeer(r.peer_url || null))
      .catch(() => setPeer(null))
  }, [readOnly])
  return peer
}
