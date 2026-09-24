import { useState } from 'preact/hooks'
import { api } from '../api/client'
import type { SceneMatrixRow, SceneRowState, Waf } from '../api/types'
import { Chip, EditableName, EditableText, RgbInputs } from '../components/ui'
import { ADAPTER, kelvinCss, LEVEL_MAX, pad2 } from '../format'
import { useLive } from '../hooks'
import { errorMessage, mutate, notify, saveThenApply } from '../toast'

const DEFAULT_SCENE_LEVEL = 254
const DEFAULT_SCENE_CCT_K = 3000
const SCENES_POLL_MS = 10_000

const RGBWAF_CHANNELS = ['r', 'g', 'b', 'w', 'a', 'f'] as const
const ZERO_RGB = { r: 0, g: 0, b: 0, w: 0, a: 0, f: 0 }

function wafToChannels(waf: Waf | null | undefined) {
  return waf ? { w: waf.w, a: waf.a, f: waf.f } : {}
}

type RowEdit = Partial<SceneRowState>

function mergedDesired(row: SceneMatrixRow, edit: RowEdit | undefined): SceneRowState {
  return { ...row.desired, ...edit }
}

const EDITABLE_ROW_FIELDS = [
  'included',
  'power',
  'level',
  'color_mode',
  'color_temperature_kelvin',
  'xy',
  'rgb',
  'waf',
] as const

function rowMatchesServer(row: SceneMatrixRow, edit: RowEdit): boolean {
  const merged = mergedDesired(row, edit)
  return EDITABLE_ROW_FIELDS.every(
    (f) => JSON.stringify(merged[f] ?? null) === JSON.stringify(row.desired[f] ?? null),
  )
}


export function Scenes({ sceneId }: { sceneId: number }) {
  const { data: scenes, reload: reloadScenes } = useLive(
    async () => (await api.scenes(ADAPTER)).scenes,
    ['scenes'],
    { intervalMs: SCENES_POLL_MS },
  )
  const { data: matrix, reload: reloadMatrix } = useLive(
    () => api.sceneMatrix(ADAPTER, sceneId),
    ['scenes', 'virtual_lamps'],
    { intervalMs: SCENES_POLL_MS, deps: [sceneId] },
  )
  const [edits, setEdits] = useState<Map<number, RowEdit>>(new Map())
  const [busy, setBusy] = useState(false)
  const [recallGroup, setRecallGroup] = useState<number | null>(null)
  const { data: groups } = useLive(
    async () => (await api.groups(ADAPTER)).groups,
    ['groups'],
    { intervalMs: SCENES_POLL_MS },
  )

  const scene = scenes?.find((s) => s.scene_id === sceneId)

  const edit = (vl: number, patch: RowEdit) => {
    setEdits((prev) => {
      const next = new Map(prev)
      const merged = { ...next.get(vl), ...patch }
      const row = matrix?.rows.find((r) => r.virtual_lamp_id === vl)
      if (row && rowMatchesServer(row, merged)) next.delete(vl)
      else next.set(vl, merged)
      return next
    })
  }

  const discard = () => setEdits(new Map())

  const saveName = async (name: string) => {
    if (name === (scene?.name ?? '')) return
    await mutate('Rename scene', () => api.patchScene(ADAPTER, sceneId, { name }), () => {
      void reloadScenes()
    })
  }

  const toggleHa = async (next: boolean) => {
    await mutate('Scene Home Assistant', () => api.patchScene(ADAPTER, sceneId, { ha_select_enabled: next }), () => {
      void reloadScenes()
    })
  }

  const recall = async () => {
    const target = recallGroup === null ? 'whole bus' : `group ${recallGroup}`
    try {
      await api.sceneRecall(
        ADAPTER,
        sceneId,
        recallGroup === null ? undefined : { scope: 'group', group_id: recallGroup },
      )
      notify(`Recall scene ${sceneId}`, 'succeeded', `recall confirmed on bus · ${target}`)
    } catch (e) {
      notify(`Recall scene ${sceneId}`, 'failed', errorMessage(e))
    }
  }

  const applyToBus = async () => {
    if (!matrix) return
    const rows = [...edits.entries()].map(([vl, e]) => {
      const row = matrix.rows.find((r) => r.virtual_lamp_id === vl)
      const d = row ? mergedDesired(row, e) : (e as SceneRowState)
      const desired: Partial<SceneRowState> = d.included
        ? {
            included: true,
            power: 'on',
            level: d.level ?? DEFAULT_SCENE_LEVEL,
            ...(d.color_mode === 'cct' && d.color_temperature_kelvin != null
              ? {
                  color_mode: 'cct',
                  color_temperature_kelvin: d.color_temperature_kelvin,
                }
              : {}),
            ...(d.color_mode === 'rgb' && d.rgb != null
              ? { color_mode: 'rgb', rgb: d.rgb }
              : {}),
            ...(d.color_mode === 'rgbwaf' && d.rgb != null && d.waf != null
              ? {
                  color_mode: 'rgbwaf',
                  rgbwaf: {
                    r: d.rgb.r,
                    g: d.rgb.g,
                    b: d.rgb.b,
                    w: d.waf.w,
                    a: d.waf.a,
                    f: d.waf.f,
                  },
                }
              : {}),
            ...(d.color_mode === 'xy' && d.xy != null
              ? { color_mode: 'xy', xy: d.xy }
              : {}),
          }
        : { included: false }
      return { virtual_lamp_id: vl, desired }
    })
    await saveThenApply({
      subject: `Scene ${sceneId}`,
      setBusy,
      save: edits.size > 0 ? () => api.patchSceneMatrix(ADAPTER, sceneId, rows) : null,
      apply: () => api.sceneApply(ADAPTER, sceneId),
      nothingToApply: 'nothing to write',
      onApplied: () => {
        setEdits(new Map())
        void reloadMatrix()
        void reloadScenes()
      },
    })
  }

  const localChanges = edits.size
  const serverDirty = matrix?.rows.filter((r) => r.dirty).length ?? 0

  return (
    <>
      <div class="crumbs">Adapter {ADAPTER} / Scenes</div>
      <div class="head">
        <h1>Scenes</h1>
        <span class="sub">Adapter {ADAPTER} · 16 scenes</span>
        <span class="h1-wrap" title={`Click to rename scene ${sceneId}`}>
          <EditableName
            key={sceneId}
            cls={`name-edit${scene?.name ? '' : ' name-faint'}`}
            value={scene?.name ?? ''}
            placeholder={`— scene ${sceneId} unnamed`}
            onCommit={(v) => void saveName(v)}
          />
          <label class="hatoggle">
            <input
              type="checkbox"
              checked={scene?.ha_select_enabled !== false}
              onChange={() => void toggleHa(scene?.ha_select_enabled === false)}
            />
            Home Assistant
          </label>
          <span class="pencil">✎</span>
        </span>
        <span class="spacer" />
        <select
          class="sel"
          value={recallGroup === null ? '' : String(recallGroup)}
          onChange={(e) => {
            const v = (e.target as HTMLSelectElement).value
            setRecallGroup(v === '' ? null : Number(v))
          }}
        >
          <option value="">Whole bus</option>
          {(groups ?? []).map((g) => (
            <option key={g.group_id} value={String(g.group_id)}>
              G{g.group_id}
              {g.name ? ` · ${g.name}` : ''}
            </option>
          ))}
        </select>
        <button class="btn recall" onClick={recall}>
          ▶ Recall scene {sceneId}
        </button>
      </div>

      <div class="scene-tabs">
        {(scenes ?? []).map((s) => (
          <a
            key={s.scene_id}
            class={`stab${s.scene_id === sceneId ? ' active' : ''}${s.dirty ? ' dirty' : ''}`}
            href={`#/scenes/${s.scene_id}`}
          >
            <span class="sn">S{s.scene_id}</span>
            <span class="nm">{s.name || '—'}</span>
            <span class="rows">
              {s.row_count_included > 0 ? `${s.row_count_included} lamps` : '0'}
            </span>
          </a>
        ))}
      </div>

      <div class="table-wrap">
        <table class="scenetbl">
          <colgroup>
            <col />
            <col style="width:76px" />
            <col style="width:128px" />
            <col style="width:330px" />
            <col style="width:64px" />
            <col style="width:116px" />
          </colgroup>
          <thead>
            <tr>
              <th>Virtual lamp</th>
              <th class="c">Included</th>
              <th>Level</th>
              <th>Color</th>
              <th class="c">Power</th>
              <th title="What the gear last confirmed for this scene slot">Applied</th>
            </tr>
          </thead>
          <tbody>
            {(matrix?.rows ?? []).map((row) => {
              const vl = row.virtual_lamp_id
              const e = edits.get(vl)
              const d = mergedDesired(row, e)
              const rowDirty = row.dirty || e !== undefined
              const levelDirty = e?.level !== undefined || (row.dirty && d.level !== row.applied.level)
              const inc = d.included
              const incCls =
                inc && row.applied.included
                  ? 'on'
                  : inc
                    ? 'add'
                    : row.applied.included
                      ? 'rm'
                      : ''
              const lvl = d.level ?? DEFAULT_SCENE_LEVEL
              return (
                <tr key={vl} class={rowDirty ? 'rowdirty' : undefined}>
                  <td class="lamp">
                    <span class="vl">VL {pad2(vl)}</span>
                    {row.name || <span class="name-faint">— unnamed</span>}
                  </td>
                  <td class="c">
                    <span
                      class={`inc ${incCls}`}
                      onClick={() =>
                        edit(vl, {
                          included: !inc,
                          ...(inc ? {} : { level: d.level ?? DEFAULT_SCENE_LEVEL }),
                        })
                      }
                    >
                      {inc ? '✓' : ''}
                    </span>
                  </td>
                  <td>
                    {inc ? (
                      <span class="lvl">
                        <EditableText
                          cls={`num${levelDirty ? ' dirty' : ''}`}
                          inputMode="numeric"
                          value={String(lvl)}
                          onCommit={(raw) => {
                            const n = Number(raw.trim())
                            if (raw.trim() !== '' && Number.isFinite(n))
                              edit(vl, { level: Math.max(0, Math.min(LEVEL_MAX, n)) })
                          }}
                        />
                        <span class="unit">/{LEVEL_MAX}</span>
                      </span>
                    ) : (
                      <span class="na">—</span>
                    )}
                  </td>
                  <td>
                    {(() => {
                      const modes = (['cct', 'rgb', 'rgbwaf', 'xy'] as const).filter(
                        (m) => row.capabilities[m],
                      )
                      const active =
                        d.color_mode && (modes as readonly string[]).includes(d.color_mode)
                          ? (d.color_mode as 'cct' | 'rgb' | 'rgbwaf' | 'xy')
                          : null
                      const six = { ...ZERO_RGB, ...d.rgb, ...wafToChannels(d.waf) }
                      return (
                        <span class="ccell">
                          <select
                            class="modesel"
                            disabled={!inc || modes.length === 0}
                            value={active ?? ''}
                            onChange={(ev) => {
                              const v = ev.currentTarget.value
                              if (v === '') edit(vl, { color_mode: null })
                              else if (v === 'cct')
                                edit(vl, {
                                  color_mode: 'cct',
                                  color_temperature_kelvin:
                                    d.color_temperature_kelvin ?? DEFAULT_SCENE_CCT_K,
                                })
                              else if (v === 'rgb')
                                edit(vl, { color_mode: 'rgb', rgb: d.rgb ?? { r: 0, g: 0, b: 0 } })
                              else if (v === 'rgbwaf')
                                edit(vl, {
                                  color_mode: 'rgbwaf',
                                  rgb: d.rgb ?? { r: 0, g: 0, b: 0 },
                                  waf: d.waf ?? { w: 0, a: 0, f: 0 },
                                })
                              else edit(vl, { color_mode: 'xy', xy: d.xy ?? { x: 0, y: 0 } })
                            }}
                          >
                            <option value="">—</option>
                            {modes.map((m) => (
                              <option key={m} value={m}>
                                {m}
                              </option>
                            ))}
                          </select>
                          <span class={`cctl${active === 'rgbwaf' ? ' six' : ''}`}>
                            {inc && active === 'cct' && (
                              <>
                                <span
                                  class="swatch"
                                  style={`background:${kelvinCss(d.color_temperature_kelvin ?? DEFAULT_SCENE_CCT_K)}`}
                                />
                                <EditableText
                                  cls={`num${e?.color_temperature_kelvin !== undefined ? ' dirty' : ''}`}
                                  inputMode="numeric"
                                  value={String(d.color_temperature_kelvin ?? DEFAULT_SCENE_CCT_K)}
                                  onCommit={(raw) => {
                                    const n = Number(raw.trim())
                                    if (raw.trim() === '' || !Number.isFinite(n)) return
                                    edit(vl, {
                                      color_mode: 'cct',
                                      color_temperature_kelvin: Math.max(1000, Math.min(20000, Math.round(n))),
                                    })
                                  }}
                                />
                                <span class="unit">K</span>
                              </>
                            )}
                            {inc && active === 'rgb' && (
                              <RgbInputs
                                compact
                                dirty={e?.rgb !== undefined}
                                values={d.rgb ?? { r: 0, g: 0, b: 0 }}
                                onInput={(c, raw) => {
                                  const n = Number(raw)
                                  if (Number.isFinite(n))
                                    edit(vl, {
                                      color_mode: 'rgb',
                                      rgb: {
                                        ...(d.rgb ?? { r: 0, g: 0, b: 0 }),
                                        [c]: Math.max(0, Math.min(255, n)),
                                      },
                                    })
                                }}
                              />
                            )}
                            {inc && active === 'rgbwaf' && (
                              <RgbInputs
                                compact
                                dirty={e?.rgb !== undefined || e?.waf !== undefined}
                                channels={RGBWAF_CHANNELS}
                                values={six}
                                onInput={(c, raw) => {
                                  const n = Number(raw)
                                  if (!Number.isFinite(n)) return
                                  const v = Math.max(0, Math.min(255, n))
                                  const next = { ...six, [c]: v }
                                  edit(vl, {
                                    color_mode: 'rgbwaf',
                                    rgb: { r: next.r, g: next.g, b: next.b },
                                    waf: { w: next.w, a: next.a, f: next.f },
                                  })
                                }}
                              />
                            )}
                            {inc &&
                              active === 'xy' &&
                              (['x', 'y'] as const).map((c) => (
                                <EditableText
                                  key={c}
                                  cls={`rgb-in sm${e?.xy !== undefined ? ' dirty' : ''}`}
                                  inputMode="decimal"
                                  value={d.xy?.[c] == null ? '' : String(d.xy[c])}
                                  placeholder={c}
                                  onCommit={(raw) => {
                                    const n = Number(raw.trim())
                                    if (raw.trim() !== '' && Number.isFinite(n))
                                      edit(vl, {
                                        color_mode: 'xy',
                                        xy: {
                                          ...(d.xy ?? { x: 0, y: 0 }),
                                          [c]: Math.max(0, Math.min(1, n)),
                                        },
                                      })
                                  }}
                                />
                              ))}
                          </span>
                        </span>
                      )
                    })()}
                  </td>
                  <td class="c">
                    {inc ? <span class="cval">{d.power ?? 'on'}</span> : <span class="na">—</span>}
                  </td>
                  <td>
                    {row.applied.included ? (
                      rowDirty && d.level !== row.applied.level ? (
                        <span class="applied-v diff">
                          {row.applied.level} → {inc ? lvl : 'out'}
                        </span>
                      ) : (
                        <span class="applied-v">{row.applied.level}</span>
                      )
                    ) : inc ? (
                      <span class="applied-v diff">not in scene</span>
                    ) : (
                      <span class="applied-v">—</span>
                    )}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      {localChanges > 0 && (
        <div class="applybar">
          <span class="txt">
            <b>
              {localChanges} pending edit{localChanges === 1 ? '' : 's'}
            </b>{' '}
            in scene {sceneId}
          </span>
          <button class="btn discard" onClick={discard} disabled={busy}>
            Discard
          </button>
          <button class="btn apply" onClick={applyToBus} disabled={busy}>
            Apply to bus
          </button>
        </div>
      )}
      {serverDirty > 0 && (
        <Chip cls="warn">
          {serverDirty} row{serverDirty === 1 ? '' : 's'} dirty on gear
        </Chip>
      )}
      {scene?.dirty && serverDirty === 0 && <Chip cls="warn">scene dirty</Chip>}
    </>
  )
}
