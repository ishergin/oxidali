import { api } from '../api/client'
import type { GroupApplyOutcome, Operation, SceneApplyOutcome } from '../api/types'
import { Badge, Chip } from '../components/ui'
import { isPreempted, opStatusChip, opStatusLabel, opSummary, opTitle, pad2 } from '../format'
import { useLive } from '../hooks'
import { nav } from '../router'

const OPS_POLL_MS = 1000

const MAX_DETAILED_OPS = 20

type Bucket = { n: number; label: string; cls: 'ok' | 'skip' | 'fail' }

function buckets(op: Operation): Bucket[] {
  const r = op.result
  if (!r) return []
  if (r.written_total != null || r.written || r.updated || r.cleared) {
    const b: Bucket[] = [
      { n: r.written_total ?? r.written?.length ?? 0, label: 'written', cls: 'ok' },
    ]
    if ((r.updated_total ?? 0) > 0) b.push({ n: r.updated_total ?? 0, label: 'updated', cls: 'ok' })
    if ((r.cleared_total ?? 0) > 0) b.push({ n: r.cleared_total ?? 0, label: 'cleared', cls: 'skip' })
    b.push({ n: r.skipped_total ?? r.skipped?.length ?? 0, label: 'skipped', cls: 'skip' })
    b.push({ n: r.failed_total ?? r.failed?.length ?? 0, label: 'failed', cls: 'fail' })
    return b
  }
  if (r.programmed_total != null || r.programmed || r.skipped || r.failed) {
    return [
      { n: r.programmed_total ?? r.programmed?.length ?? 0, label: 'programmed', cls: 'ok' },
      { n: r.skipped_total ?? r.skipped?.length ?? 0, label: 'skipped', cls: 'skip' },
      { n: r.failed_total ?? r.failed?.length ?? 0, label: 'failed', cls: 'fail' },
    ]
  }
  return []
}

type OutcomeRow = (GroupApplyOutcome | SceneApplyOutcome) & { bucket: string }

function outcomeRows(op: Operation): OutcomeRow[] {
  const r = op.result
  if (!r) return []
  const rows: OutcomeRow[] = []
  const push = (list: (GroupApplyOutcome | SceneApplyOutcome)[] | undefined, bucket: string) => {
    for (const o of list ?? []) rows.push({ ...o, bucket })
  }
  push(r.programmed, 'programmed')
  push(r.written, 'written')
  push(r.updated, 'updated')
  push(r.cleared, 'cleared')
  push(r.skipped, 'skipped')
  push(r.failed, 'failed')
  return rows
}

function OpDetail({ op }: { op: Operation }) {
  const chip = opStatusChip(op.status, op.error?.code)
  const bs = buckets(op)
  const rows = outcomeRows(op)
  const attrOutcomes = Object.entries(op.attribute_read_outcomes ?? {})
  return (
    <div class="card opdetail">
      <header>
        <h3>{opTitle(op)}</h3>
        <Chip cls={chip.cls} spin={chip.spin}>
          {opStatusLabel(op)}
        </Chip>
        <span class="id">{op.operation_id}</span>
      </header>

      <div class="meta-row">
        <span>
          Type <span class="mono">{op.type}</span>
        </span>
        <span>
          Operation <span class="mono">{op.operation_id}</span>
        </span>
        {opSummary(op) && (
          <span>
            Result <span class="mono">{opSummary(op)}</span>
          </span>
        )}
      </div>

      {(op.status === 'running' || op.status === 'accepted') && (
        <div style="padding: 12px 16px">
          <div class="bar indet">
            <div class="fill" />
          </div>
        </div>
      )}

      {isPreempted(op) ? (
        <div class="meta-row">
          <span>Stood down for an operator command — nothing was changed.</span>
        </div>
      ) : (
        op.error && (
          <div class="err-box">
            <span class="code">{op.error.code}</span>
            {op.error.message ? <> — {op.error.message}</> : null}
          </div>
        )
      )}

      {bs.length > 0 && (
        <div class="buckets">
          {bs.map((b) => (
            <div class={`bucket ${b.cls}`} key={b.label}>
              <span class="n">{b.n}</span>
              <span class="l">{b.label}</span>
            </div>
          ))}
        </div>
      )}

      {rows.length > 0 && (
        <table class="otable">
          <thead>
            <tr>
              <th>VL</th>
              <th>Group</th>
              <th>Action</th>
              <th>SA</th>
              <th>Reason</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((o, i) => (
              <tr key={i}>
                <td class="mono">VL {pad2(o.virtual_lamp_id)}</td>
                <td class="mono">{'group_id' in o ? `G${o.group_id}` : '—'}</td>
                <td class={o.action === 'remove' ? 'act-rm' : 'act-add'}>{o.action}</td>
                <td class="mono">
                  {o.physical_short_address != null ? `SA ${pad2(o.physical_short_address)}` : '—'}
                </td>
                <td class={`reason${o.bucket === 'skipped' ? ' skip' : ''}`}>
                  {o.reason ? `${o.bucket} · ${o.reason}` : o.bucket === 'failed' ? o.bucket : '—'}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {attrOutcomes.length > 0 && (
        <div class="attr-outcomes">
          {attrOutcomes.map(([group, outcome]) => (
            <Badge key={group}>
              {group}: <span style={`color: var(--${outcome === 'ok' ? 'ok' : 'warn'})`}>{outcome}</span>
            </Badge>
          ))}
        </div>
      )}

      <div class="footnote">
        <b>TTL:</b> finished operations are evicted from the tracker 60 s after completion —
        inspect outcomes before they expire.
      </div>
    </div>
  )
}

export function Operations({ selectedId }: { selectedId?: string }) {
  const { data } = useLive(async () => {
    const list = await api.operations()
    const ops = (
      await Promise.all(
        list.operations
          .slice()
          .reverse()
          .slice(0, MAX_DETAILED_OPS)
          .map((id) => api.operation(id).catch(() => null)),
      )
    ).filter((o): o is Operation => o !== null)
    return ops
  }, ['operations'], { intervalMs: OPS_POLL_MS })

  const ops = data ?? []
  const selected = ops.find((o) => o.operation_id === selectedId) ?? ops[0]

  return (
    <>
      <div class="crumbs">
        <a href="#/">Adapter 0</a> / Operations
      </div>
      <div class="head">
        <h1>Operations</h1>
        <span class="sub">{ops.length} tracked · finished operations evict after 60 s</span>
      </div>

      {ops.length === 0 ? (
        <div class="empty">
          No tracked operations. Operation-backed actions (discovery, applies, attribute
          reads/writes) appear here while running and for 60 s after they finish.
        </div>
      ) : (
        <div class="panes">
          <div class="oplist">
            {ops.map((op) => {
              const chip = opStatusChip(op.status, op.error?.code)
              const running = op.status === 'running' || op.status === 'accepted'
              return (
                <div
                  key={op.operation_id}
                  class={`op${selected?.operation_id === op.operation_id ? ' active' : ''}`}
                  onClick={() => nav(`/operations/${op.operation_id}`)}
                >
                  <div class="row1">
                    <span class="title">{opTitle(op)}</span>
                    <Chip cls={chip.cls} spin={chip.spin}>
                      {opStatusLabel(op)}
                    </Chip>
                  </div>
                  {running && (
                    <div class="bar indet">
                      <div class="fill" />
                    </div>
                  )}
                  <div class="row2">
                    <span class="id">{op.operation_id}</span>
                  </div>
                </div>
              )
            })}
          </div>
          {selected && <OpDetail op={selected} />}
        </div>
      )}
    </>
  )
}
