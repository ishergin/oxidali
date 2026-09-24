import { signal } from '@preact/signals'

import { subscribe as subscribeWs } from './ws'
import type {
  InputDeviceDetail,
  InputDeviceSummary,
  Adapter,
  AttributeReadsRequest,
  ControllerSummary,
  ControllerTime,
  DaliCommandRequest,
  DaliCommandResponse,
  DaliLevelRequest,
  DaliRawRequest,
  Diagnostics,
  CommissioningAddressChangeRequest,
  CommissioningIdentifyRequest,
  CommissioningReplacementRequest,
  CommissioningStepName,
  CommissioningStepRequest,
  CommissioningStepResult,
  DiscoveryMode,
  Group,
  GroupList,
  GroupMembershipMatrix,
  GroupMatrixRow,
  HclOverride,
  HclSchedule,
  HclScheduleList,
  Health,
  HomeAssistantSettings,
  HomeAssistantSettingsPatch,
  Operation,
  OperationAccepted,
  OperationList,
  PhysicalDeviceAttributes,
  PhysicalDeviceCore,
  PhysicalDeviceMemoryBanks,
  PhysicalDeviceList,
  PhysicalDevicePatch,
  FirmwareState,
  PollerSettings,
  DaliSettings,
  RulePatched,
  RulesDocument,
  RulesDocumentJson,
  RulesParseError,
  RulesParseOk,
  RulesParseResult,
  Scene,
  SceneList,
  SceneMatrix,
  SceneRecallRequest,
  SceneRowState,
  StatsReportPayload,
  TargetStateRequest,
  VirtualLamp,
  VirtualLampList,
  VirtualLampPatch,
  WriteAttributesRequest,
  RedundancySettings,
  RedundancyState,
  Policies,
  ConfigSliceRow,
} from './types'

const BASE = '/api/v1'
const OPERATION_POLL_MS = 700
const OPERATION_POLL_TIMEOUT_MS = 120_000

export class ApiError extends Error {
  status: number
  code: string
  body?: unknown
  constructor(status: number, code: string, message?: string, body?: unknown) {
    super(message ?? code)
    this.status = status
    this.code = code
    this.body = body
  }
}

export const controllerRole = signal<'active' | 'standby' | null>(null)

function noteRole(res: Response): void {
  const value = res.headers.get('X-Dali2rust-Role')
  if (value === 'active' || value === 'standby') {
    controllerRole.value = value
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })
  noteRole(res)
  const text = await res.text()
  const json = text ? JSON.parse(text) : null
  if (!res.ok) {
    const code = json?.error ?? `http_${res.status}`
    throw new ApiError(res.status, code, json?.message, json ?? undefined)
  }
  return json as T
}

export function isRulesParseError(v: unknown): v is RulesParseError {
  const e = v as RulesParseError | null
  return (
    typeof e === 'object' &&
    e !== null &&
    e.error === 'parse_error' &&
    typeof e.line === 'number' &&
    typeof e.column === 'number'
  )
}

const get = <T>(path: string) => request<T>('GET', path)
const post = <T>(path: string, body?: unknown) => request<T>('POST', path, body ?? {})
const put = <T>(path: string, body: unknown) => request<T>('PUT', path, body)
const patch = <T>(path: string, body: unknown) => request<T>('PATCH', path, body)
const del = <T>(path: string) => request<T>('DELETE', path)

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

const TERMINAL = new Set(['succeeded', 'failed', 'timed_out', 'cancelled'])

export async function awaitOperation(
  accepted: OperationAccepted,
  onUpdate?: (op: Operation) => void,
): Promise<Operation> {
  const deadline = Date.now() + OPERATION_POLL_TIMEOUT_MS
  const pushed = watchOperationFrames(accepted.operation_id, onUpdate)
  try {
    return await pollOperation(accepted, deadline, onUpdate, pushed)
  } finally {
    pushed.dispose()
  }
}

interface PushedTerminal {
  latest: () => Operation | null
  dispose: () => void
}

function watchOperationFrames(
  operationId: string,
  onUpdate?: (op: Operation) => void,
): PushedTerminal {
  let latest: Operation | null = null
  const dispose = subscribeWs(['operations'], (event) => {
    if (event.type !== 'OperationStatusChangedEvent') return
    const op = event.payload as Operation
    if (op?.operation_id !== operationId) return
    latest = op
    onUpdate?.(op)
  })
  return { latest: () => latest, dispose }
}

async function fetchOperation(operationId: string): Promise<Operation | null> {
  try {
    return await get<Operation>(`/operations/${operationId}`)
  } catch {
    return null
  }
}

async function pollOperation(
  accepted: OperationAccepted,
  deadline: number,
  onUpdate: ((op: Operation) => void) | undefined,
  pushed: PushedTerminal,
): Promise<Operation> {
  let recordSeen = false
  for (;;) {
    const seen = pushed.latest()
    if (seen && TERMINAL.has(seen.status)) {
      const full = await fetchOperation(accepted.operation_id)
      if (!full) return seen
      onUpdate?.(full)
      return full
    }
    await sleep(OPERATION_POLL_MS)
    let op: Operation
    try {
      op = await get<Operation>(`/operations/${accepted.operation_id}`)
      recordSeen = true
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        if (!recordSeen && Date.now() <= deadline) continue
        return { operation_id: accepted.operation_id, type: accepted.type, status: 'unknown' }
      }
      throw e
    }
    onUpdate?.(op)
    if (TERMINAL.has(op.status)) return op
    if (Date.now() > deadline) return op
  }
}

export const api = {
  health: () => get<Health>('/health'),
  controller: () => get<ControllerSummary>('/controller'),
  stats: () => get<StatsReportPayload>('/stats'),
  adapters: () => get<{ adapters: Adapter[] }>('/adapters'),

  inputDevices: (a: number) =>
    get<{ input_devices: InputDeviceSummary[] }>(`/adapters/${a}/input-devices`),
  inputDevice: (a: number, short: number) =>
    get<InputDeviceDetail>(`/adapters/${a}/input-devices/${short}`),
  patchInputDevice: (a: number, short: number, body: Record<string, unknown>) =>
    patch<InputDeviceDetail>(`/adapters/${a}/input-devices/${short}`, body),
  forgetInputDevice: (a: number, short: number) =>
    del<{ forgotten: boolean }>(`/adapters/${a}/input-devices/${short}`),
  scanInputDevices: (a: number) =>
    post<OperationAccepted>(`/adapters/${a}/input-devices/scan`, {}),
  commissionInputDevices: (a: number) =>
    post<OperationAccepted>(`/adapters/${a}/input-devices/commission`, {}),
  identifyInputDevice: (a: number, short: number) =>
    post<OperationAccepted>(`/adapters/${a}/input-devices/${short}/identify`, {}),
  configureInputInstance: (a: number, short: number, inst: number, body: Record<string, unknown>) =>
    patch<OperationAccepted>(`/adapters/${a}/input-devices/${short}/instances/${inst}`, body),
  configureInputFeedback: (a: number, short: number, inst: number, body: Record<string, unknown>) =>
    patch<OperationAccepted>(`/adapters/${a}/input-devices/${short}/instances/${inst}/feedback`, body),

  physicalDevices: (a: number) =>
    get<PhysicalDeviceList>(`/adapters/${a}/physical-devices`),
  physicalDevice: (a: number, short: number) =>
    get<PhysicalDeviceCore>(`/adapters/${a}/physical-devices/${short}`),
  physicalDeviceAttributes: (a: number, short: number, sections?: readonly string[]) =>
    get<PhysicalDeviceAttributes>(
      `/adapters/${a}/physical-devices/${short}/attributes` +
        (sections && sections.length ? `?sections=${sections.join(',')}` : ''),
    ),
  physicalDeviceMemoryBanks: (a: number, short: number) =>
    get<PhysicalDeviceMemoryBanks>(`/adapters/${a}/physical-devices/${short}/memory-banks`),
  patchPhysicalDevice: (a: number, short: number, body: PhysicalDevicePatch) =>
    patch<PhysicalDeviceCore>(`/adapters/${a}/physical-devices/${short}`, body),
  deviceTargetState: (a: number, short: number, body: TargetStateRequest) =>
    put<PhysicalDeviceCore>(`/adapters/${a}/physical-devices/${short}/target-state`, body),
  writeAttributes: (a: number, short: number, body: WriteAttributesRequest) =>
    post<OperationAccepted>(`/adapters/${a}/physical-devices/${short}/write-attributes`, body),
  attributeReads: (a: number, short: number, body: AttributeReadsRequest) =>
    post<OperationAccepted>(`/adapters/${a}/physical-devices/${short}/attribute-reads`, body),
  discoveryRun: (a: number, mode: DiscoveryMode) =>
    post<OperationAccepted>(`/adapters/${a}/discovery-runs`, { mode }),

  commissioningIdentify: (a: number, body: CommissioningIdentifyRequest) =>
    post<OperationAccepted>(`/adapters/${a}/commissioning/identify`, body),
  commissioningAddressChange: (a: number, body: CommissioningAddressChangeRequest) =>
    post<OperationAccepted>(`/adapters/${a}/commissioning/address-changes`, body),
  commissioningReplacement: (a: number, body: CommissioningReplacementRequest) =>
    post<OperationAccepted>(`/adapters/${a}/commissioning/replacements`, body),
  commissioningStep: (a: number, step: CommissioningStepName, body: CommissioningStepRequest) =>
    post<CommissioningStepResult>(`/adapters/${a}/commissioning/steps/${step}`, body),

  virtualLamps: (a: number) => get<VirtualLampList>(`/adapters/${a}/virtual-lamps`),
  virtualLamp: (a: number, id: number) =>
    get<VirtualLamp>(`/adapters/${a}/virtual-lamps/${id}`),
  patchVirtualLamp: (a: number, id: number, body: VirtualLampPatch) =>
    patch<VirtualLamp>(`/adapters/${a}/virtual-lamps/${id}`, body),
  bindVirtualLamp: (a: number, id: number, short: number) =>
    put<VirtualLamp>(`/adapters/${a}/virtual-lamps/${id}/binding`, {
      physical_short_address: short,
    }),
  unbindVirtualLamp: (a: number, id: number) =>
    del<VirtualLamp>(`/adapters/${a}/virtual-lamps/${id}/binding`),
  deleteVirtualLamp: (a: number, id: number) =>
    del<null>(`/adapters/${a}/virtual-lamps/${id}`),
  deletePhysicalDevice: (a: number, short: number) =>
    del<null>(`/adapters/${a}/physical-devices/${short}`),
  lampTargetState: (a: number, id: number, body: TargetStateRequest) =>
    put<VirtualLamp>(`/adapters/${a}/virtual-lamps/${id}/target-state`, body),

  groups: (a: number) => get<GroupList>(`/adapters/${a}/groups`),
  patchGroup: (a: number, id: number, body: { name?: string; ha_entity_enabled?: boolean }) =>
    patch<Group>(`/adapters/${a}/groups/${id}`, body),
  groupMatrix: (a: number) =>
    get<GroupMembershipMatrix>(`/adapters/${a}/group-membership-matrix`),
  patchGroupMatrix: (a: number, rows: Partial<GroupMatrixRow>[]) =>
    patch<OperationAccepted>(`/adapters/${a}/group-membership-matrix`, { rows }),
  groupsApply: (a: number) =>
    post<OperationAccepted | GroupMembershipMatrix>(`/adapters/${a}/groups/apply`),
  groupTargetState: (a: number, id: number, body: TargetStateRequest) =>
    put<Group>(`/adapters/${a}/groups/${id}/target-state`, body),

  scenes: (a: number) => get<SceneList>(`/adapters/${a}/scenes`),
  patchScene: (a: number, id: number, body: { name?: string; ha_select_enabled?: boolean }) =>
    patch<Scene>(`/adapters/${a}/scenes/${id}`, body),
  sceneMatrix: (a: number, id: number) =>
    get<SceneMatrix>(`/adapters/${a}/scenes/${id}/matrix`),
  patchSceneMatrix: (
    a: number,
    id: number,
    rows: { virtual_lamp_id: number; desired: Partial<SceneRowState> }[],
  ) => patch<OperationAccepted>(`/adapters/${a}/scenes/${id}/matrix`, { rows }),
  sceneApply: (a: number, id: number) =>
    post<OperationAccepted | SceneMatrix>(`/adapters/${a}/scenes/${id}/apply`),
  sceneRecall: (a: number, id: number, body?: SceneRecallRequest) =>
    post<{ correlation_id: number; status: string }>(`/adapters/${a}/scenes/${id}/recall`, body),

  operations: () => get<OperationList>('/operations'),
  operation: (id: string) => get<Operation>(`/operations/${id}`),

  hclSchedules: () => get<HclScheduleList>('/hcl-schedules'),
  hclSchedule: (id: string) => get<HclSchedule>(`/hcl-schedules/${id}`),
  createHclSchedule: (body: Partial<HclSchedule>) =>
    post<OperationAccepted & { schedule_id: string }>('/hcl-schedules', body),
  patchHclSchedule: (id: string, body: Partial<HclSchedule>) =>
    patch<OperationAccepted>(`/hcl-schedules/${id}`, body),
  deleteHclSchedule: (id: string) => del(`/hcl-schedules/${id}`),
  hclOverride: (id: string) => get<HclOverride>(`/hcl-schedules/${id}/override`),
  clearHclOverride: (id: string) => del(`/hcl-schedules/${id}/override`),

  firmware: () => get<FirmwareState>('/firmware'),
  startFirmwareUpdate: (url: string) =>
    post<OperationAccepted>('/firmware/updates', { url }),

  pollerSettings: () => get<PollerSettings>('/settings/poller'),

  redundancySettings: () => get<RedundancySettings>('/settings/redundancy'),
  patchRedundancySettings: (body: Partial<RedundancySettings>) =>
    patch<RedundancySettings>('/settings/redundancy', body),
  redundancy: () => get<RedundancyState>('/redundancy'),
  redundancySwitchover: () => post<RedundancyState>('/redundancy/switchover', {}),

  policies: () => get<Policies>('/policies'),
  patchPolicies: (body: Partial<Policies>) => patch<Policies>('/policies', body),
  policiesApply: () => post<OperationAccepted>('/policies/apply', {}),

  configSlices: () => get<ConfigSliceRow[]>('/config/slices'),

  daliSettings: () => get<DaliSettings>('/settings/dali'),
  patchDaliSettings: (body: Partial<DaliSettings>) =>
    patch<DaliSettings>('/settings/dali', body),

  homeAssistantSettings: () => get<HomeAssistantSettings>('/settings/home-assistant'),
  patchHomeAssistantSettings: (body: HomeAssistantSettingsPatch) =>
    patch<HomeAssistantSettings>('/settings/home-assistant', body),
  publishHomeAssistantDiscovery: () =>
    post<OperationAccepted>('/settings/home-assistant/discovery-publish', {}),
  patchPollerSettings: (body: Partial<PollerSettings>) =>
    patch<PollerSettings>('/settings/poller', body),

  rules: () => get<RulesDocument>('/rules'),
  rulesJson: () => get<RulesDocumentJson>('/rules?format=json'),
  parseRules: async (source: string): Promise<RulesParseResult> => {
    try {
      return await post<RulesParseOk>('/rules/parse', { source })
    } catch (e) {
      if (e instanceof ApiError && e.code === 'parse_error' && isRulesParseError(e.body)) {
        return e.body
      }
      throw e
    }
  },
  putRules: (source: string, baseRevision: number) =>
    put<OperationAccepted>('/rules', { source, base_revision: baseRevision }),
  patchRule: (name: string, enabled: boolean) =>
    patch<RulePatched>(`/rules/${encodeURIComponent(name)}`, { enabled }),
  runRule: (name: string, dry = false) =>
    post<OperationAccepted>(
      `/rules/${encodeURIComponent(name)}/run${dry ? '?dry=1' : ''}`,
      {},
    ),

  time: () => get<ControllerTime>('/time'),

  diagnostics: () => get<Diagnostics>('/diagnostics'),
  daliCommand: (body: DaliCommandRequest) =>
    post<DaliCommandResponse>('/dali/command', body),
  daliLevel: (body: DaliLevelRequest) => post<DaliCommandResponse>('/dali/level', body),
  daliRaw: (body: DaliRawRequest) => post<DaliCommandResponse>('/dali/raw', body),

}
