export interface ErrorBody {
  error: string
  message?: string
}

export interface StatusFlags {
  raw: number
  lamp_failure: boolean
  gear_failure: boolean
  lamp_on: boolean
  limit_error: boolean
  fade_running: boolean
  reset_state: boolean
  missing_short_address: boolean
  power_cycle_seen: boolean
}

export interface FailureStatus {
  raw: number
  lamp_failure: boolean
  gear_failure: boolean
  communication_failure: boolean
  source: string
}

export interface CapabilityFlags {
  brightness: boolean
  cct: boolean
  xy: boolean
  rgb: boolean
  rgbwaf: boolean
  scenes: boolean
  groups: boolean
}

export type PowerState = 'on' | 'off' | 'unknown'

export type OverrideSource = 'discovered' | 'manual_override'

export interface Xy {
  x: number
  y: number
}

export interface Rgb {
  r: number
  g: number
  b: number
}

export interface Waf {
  w: number
  a: number
  f: number
}

export interface RuntimeState {
  power: PowerState | string
  level: number | null
  color_mode: string
  color_temperature_kelvin: number | null
  xy: Xy | null
  rgb: Rgb | null
  waf?: Waf | null
  status: StatusFlags | null
  failure_status: FailureStatus | null
  value_source: string | null
  last_seen_ms: number | null
  last_dapc_source: string | null
  error: { code: string } | null
}

export interface ObservedValue<T = number> {
  value: T
  source: 'readback' | 'write_confirmed' | string
  last_read_ms?: number | null
  last_write_confirmed_ms?: number | null
}

export type AttributeSection = Record<string, ObservedValue<unknown>>
export type Attributes = Record<string, AttributeSection>

export interface BankReading {
  value: number | null
  not_implemented: boolean
  temporarily_unavailable: boolean
  tmask_since_ms?: number | null
  saturated: boolean
}

export interface ImplementedParts {
  raw: number
  bytes: number
}

export interface BusUnitConfiguration {
  raw: number
  class: string
  emergency_type?: string
}

export interface MemoryBusUnit {
  configuration?: ObservedValue<BusUnitConfiguration>
  implemented_parts?: ObservedValue<ImplementedParts>
}

export interface LuminaireValue {
  raw: number
  value: number | null
  part209_implemented: boolean
}

export interface MemoryLuminaire {
  content_format_id?: ObservedValue<number>
  year?: ObservedValue<LuminaireValue>
  week?: ObservedValue<LuminaireValue>
  nominal_input_power_w?: ObservedValue<LuminaireValue>
  power_at_minimum_w?: ObservedValue<LuminaireValue>
  nominal_min_ac_voltage_v?: ObservedValue<LuminaireValue>
  nominal_max_ac_voltage_v?: ObservedValue<LuminaireValue>
  nominal_light_output_lm?: ObservedValue<LuminaireValue>
  cri?: ObservedValue<LuminaireValue>
  cct_kelvin?: ObservedValue<LuminaireValue>
  light_distribution_type?: ObservedValue<LuminaireValue>
  luminaire_colour?: ObservedValue<string>
  luminaire_identification?: ObservedValue<string>
  light_distribution?: ObservedValue<string>
  oem_name?: ObservedValue<string>
  customer_stocking_number?: ObservedValue<string>
  lamp_current_ma?: ObservedValue<LuminaireValue>
  free_use?: ObservedValue<string>
}

export interface EnergyBank {
  bank_version?: ObservedValue<number>
  energy?: ObservedValue<BankReading>
  energy_scale?: ObservedValue<number>
  power?: ObservedValue<BankReading>
  power_scale?: ObservedValue<number>
}

export interface MemoryEnergy {
  active?: EnergyBank
  apparent?: EnergyBank
  loadside?: EnergyBank
}

export interface Condition {
  flag?: ObservedValue<BankReading>
  counter?: ObservedValue<BankReading>
}

export interface GearDiagnostics {
  bank_version?: ObservedValue<number>
  operating_time_s?: ObservedValue<BankReading>
  output_current_percent?: ObservedValue<BankReading>
  output_power_limitation?: Condition
  overall_failure?: Condition
  overvoltage?: Condition
  power_factor_centi?: ObservedValue<BankReading>
  start_counter?: ObservedValue<BankReading>
  supply_frequency_hz?: ObservedValue<BankReading>
  supply_voltage_decivolt?: ObservedValue<BankReading>
  temperature_offset60?: ObservedValue<BankReading>
  thermal_derating?: Condition
  thermal_shutdown?: Condition
  undervoltage?: Condition
}

export interface SourceDiagnostics {
  bank_version?: ObservedValue<number>
  current_milliamp?: ObservedValue<BankReading>
  on_time_resettable_s?: ObservedValue<BankReading>
  on_time_s?: ObservedValue<BankReading>
  open_circuit?: Condition
  overall_failure?: Condition
  short_circuit?: Condition
  start_counter?: ObservedValue<BankReading>
  start_counter_resettable?: ObservedValue<BankReading>
  temperature_offset60?: ObservedValue<BankReading>
  thermal_derating?: Condition
  thermal_shutdown?: Condition
  voltage_decivolt?: ObservedValue<BankReading>
}

export interface LuminaireMaintenance {
  bank_version?: ObservedValue<number>
  rated_life_kilohours?: ObservedValue<BankReading>
  rated_starts_hundreds?: ObservedValue<BankReading>
  reference_temperature_offset60?: ObservedValue<BankReading>
}

export interface MemoryDiagnostics {
  control_gear?: GearDiagnostics
  light_source?: SourceDiagnostics
  luminaire?: LuminaireMaintenance
}

export interface ControllerSummary {
  controller_id: string
  firmware_version: string
  target_mcu: string
  uptime_ms: number
  network: { hostname: string; ip: string }
  home_assistant: { enabled: boolean; connected: boolean; broker_url: string }
  cluster: { enabled: boolean }
  adapter_count: number
  hydrated: boolean
}

export interface Health {
  status: string
  uptime_seconds: number
  version: string
}

export interface Adapter {
  adapter_id: number
  name: string
  enabled: boolean
  limits: { virtual_lamps: number; groups: number; scenes: number }
  bus_status: string
  counters: { commands: number; timeouts: number; errors: number }
}

export interface MemoryBankRange {
  start: number
  length: number
}

export interface MemoryBankSummary {
  bank: number
  total_bytes_read: number
  last_read_ms: number | null
  ranges: MemoryBankRange[]
}

export interface PhysicalDeviceCore {
  adapter_id: number
  short_address: number
  now_ms: number
  random_address?: number | null
  name: string
  notes?: string | null
  device_type_discovered: string
  device_type_override?: string | null
  device_type_effective: string
  device_type_source: OverrideSource
  supported_device_types?: number[]
  color_mode_discovered: string
  color_mode_override?: string | null
  color_mode_effective: string
  color_mode_source: OverrideSource
  dt8_auto_activation_repair: boolean
  dt8_rgbwaf_control_assert: boolean
  state: RuntimeState
  capabilities: CapabilityFlags
  color_temperature_range?: ColorTemperatureRange | null
}

export interface PhysicalDeviceSummary {
  short_address: number
  random_address?: number | null
  name: string
  device_type_effective: string
  color_mode_effective: string
  capabilities: CapabilityFlags
  state: RuntimeState
  groups_membership?: number | null
  gtin?: number | null
  identification_number?: number | null
}

export interface PhysicalDeviceAttributes {
  short_address: number
  now_ms: number
  attributes: Attributes
}

export interface PhysicalDeviceMemoryBanks {
  short_address: number
  now_ms: number
  memory_banks: MemoryBankSummary[]
}

export interface ColorTemperatureRange {
  min_kelvin: number
  max_kelvin: number
}

export interface PhysicalDeviceList {
  now_ms: number
  adapter_id: number
  physical_devices: PhysicalDeviceSummary[]
}

export interface RedundancySettings {
  enabled: boolean
  role: 'primary' | 'standby'
  probe_interval_ms: number
  takeover_after_missed: number
  boot_listen_ms: number
  peer_device_short_address: number | null
  peer_url: string
}

export interface RedundancyTransition {
  now_active: boolean
  reason: string
  detected_at_ms: number
  completed_at_ms: number
  took_ms: number
  last_peer_answer_ms: number
  missed_probes: number
}

export interface RedundancyState {
  enabled: boolean
  role: 'primary' | 'standby'
  active: boolean
  answering: boolean
  lease_remaining_ms: number
  probes: { published: number; ingress_rejected: number; owned: number; unowned: number }
  takeovers: number
  stand_downs: number
  role_publish_failed: number
  ignored_events: number
  replication: {
    passes: number
    peer_unreachable: number
    pulled: number
    rejected: number
    reload_publish_failed: number
  }
  transitions: RedundancyTransition[]
}

export interface Policies {
  system_failure_level: number | null
  power_on_level: number | null
  apply_on_discovery: boolean
  manages_anything: boolean
}

export interface ConfigSliceRow {
  name: string
  bytes: number | null
}

export interface DaliSettings {
  dt8_auto_activation_repair: boolean
  dt8_rgbwaf_control_assert: boolean
  application_active: boolean
  device_short_address: number | null
}

export interface PhysicalDevicePatch {
  name?: string
  notes?: string | null
  device_type_override?: string | null
  color_mode_override?: string | null
  dt8_auto_activation_repair?: boolean
  dt8_rgbwaf_control_assert?: boolean
}

export interface TargetStateRequest {
  power?: PowerState
  level?: number
  color_mode?: string
  color_temperature_kelvin?: number
  xy?: Xy
  rgb?: Rgb
  rgbwaf?: { r: number; g: number; b: number; w: number; a: number; f: number }
  transition?: { duration_ms: number }
}

export interface WriteAttributesRequest {
  fade_time_ms?: number
  fade_rate?: number
  power_on_level?: number
  system_failure_level?: number
  extended_fade_time_ms?: number
  tc_coolest_mirek?: number
  tc_warmest_mirek?: number
  min_level?: number
  max_level?: number
  dimming_curve?: number
}

export interface SceneRecallRequest {
  scope: 'group'
  group_id: number
}

export type AttributeGroup =
  | 'runtime_status'
  | 'common_102'
  | 'dt8_color'
  | 'dt6_led'
  | 'groups'
  | 'scenes'
  | 'extended'

export interface AttributeReadsRequest {
  attribute_groups: AttributeGroup[]
  memory_banks?: 'none' | 'identity' | 'profile' | 'all'
}

export interface CommissioningIdentifyRequest {
  short_address: number
}

export interface CommissioningAddressChangeRequest {
  short_address: number
  new_short_address: number
  verify_after_program?: boolean
}

export interface RestoredSlices {
  metadata_and_overrides: boolean
  attributes: boolean
  groups: boolean
  scenes: boolean
}

export interface CommissioningReplacementRequest {
  failed_short_address: number
  replacement_short_address: number
  restore?: Partial<RestoredSlices>
}

export type CommissioningStepName =
  | 'initialise'
  | 'randomise'
  | 'search-address'
  | 'compare'
  | 'program-short-address'
  | 'verify-short-address'
  | 'query-short-address'
  | 'withdraw'
  | 'terminate'
  | 'physical-selection'

export interface CommissioningStepRequest {
  scope?: 'all' | 'unaddressed' | 'short'
  short_address?: number
  search_address?: number
}

export type CommissioningStepAnswer =
  | 'address'
  | 'unaddressed'
  | 'multiple'
  | 'none'

export interface CommissioningStepResult {
  success: boolean
  backward_frame?: number
  backward_violation?: boolean
  match?: boolean
  short_address?: number | null
  answer?: CommissioningStepAnswer
}

export type DiscoveryMode =
  | 'scan_known_short_addresses'
  | 'commission_unaddressed'
  | 'refresh_known'

export interface VirtualLamp {
  adapter_id: number
  virtual_lamp_id: number
  name: string
  device_type_effective: string
  device_type_source: OverrideSource
  color_mode_effective: string
  color_mode_source: OverrideSource
  binding?: { physical_short_address: number } | null
  ha_entity_enabled: boolean
  state: RuntimeState
  capabilities: CapabilityFlags
  color_temperature_range?: ColorTemperatureRange | null
}

export interface VirtualLampList {
  adapter_id: number
  virtual_lamps: VirtualLamp[]
}

export interface VirtualLampPatch {
  name?: string
  ha_entity_enabled?: boolean
}

export interface Group {
  adapter_id: number
  group_id: number
  name: string
  ha_entity_enabled: boolean
  capabilities_summary: CapabilityFlags
  dirty: boolean
  member_count_desired: number
  member_count_applied: number
}

export interface GroupList {
  adapter_id: number
  groups: Group[]
}

export interface GroupMatrixRow {
  virtual_lamp_id: number
  name: string
  desired: boolean[]
  applied: boolean[]
}

export interface GroupMembershipMatrix {
  adapter_id: number
  groups: { group_id: number; name: string; dirty: boolean }[]
  rows: GroupMatrixRow[]
  dirty: boolean
}

export interface Scene {
  adapter_id: number
  scene_id: number
  name: string
  ha_select_enabled: boolean
  row_count_included: number
  dirty: boolean
}

export interface SceneList {
  adapter_id: number
  scenes: Scene[]
}

export interface SceneRowState {
  included: boolean
  power: PowerState | null
  level: number | null
  color_mode: string | null
  color_temperature_kelvin: number | null
  xy: Xy | null
  rgb: Rgb | null
  waf?: Waf | null
}

export interface SceneMatrixRow {
  virtual_lamp_id: number
  name: string
  capabilities: CapabilityFlags
  desired: SceneRowState
  applied: SceneRowState
  dirty: boolean
}

export interface SceneMatrix {
  adapter_id: number
  scene_id: number
  rows: SceneMatrixRow[]
}

export type OperationStatus =
  | 'accepted'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'timed_out'
  | 'cancelled'

export interface GroupApplyOutcome {
  virtual_lamp_id: number
  group_id: number
  action: string
  physical_short_address?: number
  reason?: string
}

export interface SceneApplyOutcome {
  virtual_lamp_id: number
  action: string
  physical_short_address?: number
  reason?: string
}

export interface Operation {
  operation_id: string
  type: string
  status: OperationStatus | string
  error?: { code: string; message?: string }
  result?: {
    programmed?: GroupApplyOutcome[]
    skipped?: GroupApplyOutcome[]
    failed?: GroupApplyOutcome[]
    programmed_total?: number
    skipped_total?: number
    failed_total?: number
    written?: SceneApplyOutcome[]
    updated?: SceneApplyOutcome[]
    cleared?: SceneApplyOutcome[]
    written_total?: number
    updated_total?: number
    cleared_total?: number
    entities_published?: number
    entities_failed?: number
    short_address?: number
    identify_mechanism?: 'blink_recall_max_min' | 'identify_device'
    old_short_address?: number
    new_short_address?: number
    failed_short_address?: number
    replacement_short_address?: number
    restored?: RestoredSlices
  }
  attribute_read_outcomes?: Record<string, string>
}

export interface OperationList {
  operations: string[]
}

export interface OperationAccepted {
  operation_id: string
  type: string
  status: 'accepted'
}

export interface DaliCommandRequest {
  wire_address: number
  command: number
  repeat_count: number
}

export interface DaliLevelRequest {
  wire_address: number
  level: number
}

export interface DaliRawRequest {
  frame: number
  expects_backward?: boolean
}

export interface DaliCommandResponse {
  success: boolean
  backward_frame: number
  error: string | null
  error_code?: string
  message?: string
}

export type ChannelCounters = {
  publish_attempted: number
  publish_queued: number
  ingress_overflow: number
  oversize_rejected: number
  kind_mismatch: number
}

export interface SubscriberCounters {
  delivered: number
  receiver_overflow: number
}

export interface EventSubscriberCounters {
  delivered: number
  receiver_overflow: number
  name: string
}

export interface BusCounters {
  commands: ChannelCounters
  confirmations: ChannelCounters
  events: ChannelCounters
  commands_unrouted: number
  delivery_rejected_dropped: number
  command_subscribers: SubscriberCounters[]
  confirmation_subscribers: SubscriberCounters[]
  event_subscribers: EventSubscriberCounters[]
}

export type DaliWireCounters = {
  transactions_started: number
  transactions_completed: number
  transactions_by_class: [number, number, number, number]
  transaction_reopened: number
  transaction_should_exceedances: number
  transaction_budget_exceeded: number
  transaction_leaks: number
  session_spins: number
  frames_sent_by_priority: [number, number, number, number, number]
  p1_window_late: number
  bus_releases: number
  bus_acquire_timeout: number
  collisions: number
  foreign_in_window: number
  corrupted_in_window: number
  exchange_retries: number
  retry_exhausted: number
  send_twice_over_transmitter_max: number
  send_twice_split: number
  bus_power_down_active: number
  bus_power_down_entries: number
  system_failure_active: number
  system_failure_entries: number
  wire_ticks_total: number
  wire_ticks_active: number
  wire_ticks_tx: number
  load_permille: number
  load_own_permille: number
}

export type DaliWorkerCounters = {
  invalid_command: number
  execution_failed: number
  confirmation_publish_failed: number
  event_publish_failed: number
  evidence_publish_failed: number
  event_publish_retried: number
  event_publish_backoff_ms: number
  target_state_superseded: number
  read_attributes_contended_aborts: number
  read_attributes_transport_aborts: number
  read_attributes_preempted: number
  write_attributes_preempted: number
  read_attributes_device_absent: number
  read_attributes_sequence_incomplete: number
  discovery_device_type_degraded: number
  bus_health_probe_failed: number
  memory_bank_short_reads: number
  input_scans_completed: number
  input_devices_addressed: number
  input_config_rejected: number
  tx_suppressed_passive: number
  ignored_commands: number
}

export type SnifferTranslatorCounters = {
  observed_published: number
  unknown_seen: number
  special_tracked: number
  dt8_staged: number
  backward_ignored: number
  publish_failed: number
  input_events_typed: number
  input_events_generic: number
  input_events_ambiguous_scheme: number
  input_lifecycle: number
  input_publish_retried: number
  app_control_pairs: number
}

export type RulesWorkerCounters = {
  commits_applied: number
  commits_rejected: number
  enable_toggles: number
  hydrate_failed: number
  persist_failed: number
  ignored_commands: number
  effects_published: number
  effects_ingress_rejected: number
  effects_skipped_dark: number
  hcl_hold_unmapped: number
  hcl_schedule_unmapped: number
  input_action_unmapped: number
  log_lines: number
  stat_counts: number
  activations_published: number
}

export type ProjectorCounters = {
  runtime_updates_published: number
  runtime_updates_retried: number
  group_expansions: number
  scene_expansions: number
  broadcast_expansions: number
  coalesced_observed: number
  skipped_unknown_observed: number
  skipped_unbound: number
  publish_failed: number
  ignored_events: number
}

export type PersistenceCounters = {
  flush_success_total: number
  flush_error_total: number
  no_space_total: number
  hydrate_loaded_total: number
  hydrate_default_total: number
  hydrate_error_total: number
}

export type PhySnifferCounters = {
  frames: number
  backward8: number
  forward16: number
  forward24: number
  decode_failed: number
  unsupported_len: number
  dropped: number
  poll_fast: number
}

export type HclSchedulerCounters = {
  ticks: number
  ticks_time_unsynced: number
  commands_published: number
  commands_dropped_cap: number
  deferred_dropped_cap: number
  command_timeouts: number
  command_failures: number
  ingress_rejections: number
  overrides_started: number
  overrides_cleared: number
  overrides_reset: number
  ignored_commands: number
  ignored_events: number
}

export type PollerCounters = {
  cycles_total: number
  reads_published: number
  reads_completed: number
  reads_failed: number
  reads_preempted: number
  outstanding_expired: number
  skipped_inbox_full: number
  duty_deferred: number
  interactive_deferred: number
  reads_absent: number
  targets_excluded: number
  window_deferred: number
  device_cooldowns: number
  health_probes_published: number
  health_probes_invalid: number
  health_probes_clear: number
  health_probes_one_failure: number
  health_probes_several_failures: number
  health_probes_expired: number
  ignored_events: number
}

export type WebSocketCounters = {
  clients: number
  events_sent_total: number
  events_dropped_total: number
  events_coalesced_total: number
  inbox_overflow_total: number
  upgrades_rejected_total: number
  sniffer_records_total: number
  sniffer_dropped_total: number
  logs_lines_total: number
  logs_dropped_total: number
}

export type ConfirmationBridgeCounters = {
  unmatched: number
}

export type RegistryCounters = {
  runtime_updates_superseded: number
  config_write_signal_publish_failed: number
  ignored_events: number
}

export type ApplyOrchestratorCounters = {
  runs_started: number
  cells_published: number
  skips_published: number
  cell_retries: number
  outcome_timeouts: number
  ingress_backoffs: number
  runs_aborted: number
  terminal_signal_publish_failed: number
  ignored_commands: number
}

export type OperationTrackerCounters = {
  pending_outcomes_expired: number
  ignored_commands: number
  ignored_events: number
}

export type MqttBridgeCounters = {
  connected: boolean
  connects_total: number
  publishes_total: number
  publish_failures_total: number
  commands_received_total: number
  commands_dropped_total: number
  commands_unroutable_total: number
  commands_ingress_rejected_total: number
  discovery_published_total: number
  discovery_failed_total: number
  terminal_event_publish_failed_total: number
  terminal_event_publish_retried_total: number
  bus_discarded_total: number
  bus_coalesced_total: number
  rule_publishes_dropped_total: number
}

export type RedundancyCounters = {
  defended: number
  worker_stale: number
  answered: number
  suppressed: number
  cell_busy: number
  aborted: number
  window_closed: number
  late: number
  probe_failed: number
  handover_incomplete: number
  armed: boolean
}

export interface Diagnostics {
  uptime_ms: number
  bus: BusCounters
  confirmation_bridge: ConfirmationBridgeCounters
  dali_worker: DaliWorkerCounters
  dali_wire: DaliWireCounters
  sniffer_translator: SnifferTranslatorCounters
  projector: ProjectorCounters
  rules: RulesWorkerCounters
  apply_orchestrator: ApplyOrchestratorCounters
  operations: OperationTrackerCounters
  registry: RegistryCounters
  persistence: PersistenceCounters
  phy_sniffer: PhySnifferCounters
  hcl: HclSchedulerCounters
  poller: PollerCounters
  websocket: WebSocketCounters
  mqtt: MqttBridgeCounters
  redundancy: RedundancyCounters
}

export type StatsController = {
  uptime_ms: number
  free_heap_bytes: number | null
  internal_free_bytes: number | null
  internal_largest_block_bytes: number | null
  internal_min_free_bytes: number | null
  internal_total_bytes: number | null
  internal_allocated_blocks: number | null
  rust_internal_live_bytes: number | null
  rust_internal_peak_bytes: number | null
  rust_psram_live_bytes: number | null
  rust_psram_peak_bytes: number | null
}

export type StatsBus = {
  commands_published_total: number
  events_published_total: number
  commands_ingress_overflow_total: number
  confirmation_timeouts_total: number
}

export type StatsDali = {
  commands_executed_total: number
  errors_total: number
  target_state_superseded_total: number
  wire_load_permille: number
  wire_load_own_permille: number
  foreign_frames_total: number
  foreign_verbs_projected_total: number
  foreign_dimming_unprojected_total: number
  backward_undecodable_total: number
  backward_frame_size_total: number
  backward_incomplete_total: number
  backward_early_rejected_total: number
  backward_late_rejected_total: number
  backward_multi_answer_total: number
  console_log_dropped_total: number
  console_log_busy_total: number
  console_log_truncated_total: number
  console_log_unavailable_total: number
  console_uart_errors_total: number
  isr_ticks_deficit_raw_total: number
  isr_ticks_surplus_raw_total: number
  isr_ticks_lost_total: number
  isr_ticks_extra_total: number
  isr_late_ticks_total: number
  isr_max_gap_us: number
  answer_staged_total: number
  answer_stage_late_total: number
  answer_stage_max_ticks: number
  sniff_poll_late_total: number
  sniff_poll_gap_max_us: number
  persist_flush_total: number
  persist_flush_slow_total: number
  persist_flush_ms_total: number
  persist_flush_max_ms: number
  persist_gate_waits_total: number
  persist_gate_timeouts_total: number
}

export type StatsOperations = {
  running: number
  succeeded_total: number
  failed_total: number
  timed_out_total: number
  cancelled_total: number
}

export type StatsWebSocket = {
  clients: number
  events_sent_total: number
  events_dropped_total: number
}

export type StatsMqtt = {
  connected: boolean
  publishes_total: number
  publish_failures_total: number
}

export type StatsRules = {
  activations_total: number
  activations_dry: number
  suppressed_cooldown: number
  suppressed_disabled: number
  conditions_rejected: number
  partial_outcomes: number
  chain_depth_exceeded: number
  effects_emitted: number
  actions_failed: number
  continuations_scheduled: number
  continuations_fired: number
  continuations_dropped: number
  timers_active: number
  ticks_time_unsynced: number
  rules_loaded: number
  vars_in_use: number
  latency_p50_ms: number
  latency_p95_ms: number
  latency_max_ms: number
}

export type StatsInput = {
  frames24_rx: number
  events_typed: number
  events_generic: number
  events_typed_from_registry: number
  events_ambiguous_scheme: number
  events_unattributed: number
  lifecycle_events: number
  scans_completed: number
  devices_addressed: number
  config_rejected: number
  addresses_contended: number
  presence_reprobed: number
  readbacks_applied: number
}

export type StatsNetwork = {
  rx_packets_total: number
  tx_packets_total: number
  rx_dropped_total: number
  tx_dropped_total: number
  rx_ring_overruns_total: number
  rx_fifo_overflows_total: number
  link_up_events_total: number
}

export interface StatsReportPayload {
  sample_ms: number
  controller: StatsController
  bus: StatsBus
  dali: StatsDali
  operations: StatsOperations
  websocket: StatsWebSocket
  mqtt: StatsMqtt
  input: StatsInput
  rules: StatsRules
  network: StatsNetwork | null
}

export type InputDeviceSummary = {
  adapter_id: number
  short_address: number
  name: string | null
  present: boolean
  instance_count: number
  first_instance_type: number | null
  ha_expose: boolean
  last_seen_ms: number | null
  last_event_at_ms: number | null
}

export type ReadValue<T> = { value: T | null; read_at_ms: number | null }

export type InputInstance = {
  instance_number: number
  instance_type: number | null
  instance_type_name: string | null
  instance_status: number | null
  resolution: number | null
  event_scheme: ReadValue<number>
  event_scheme_confirmed: boolean
  event_filter: ReadValue<number[]>
  event_priority: ReadValue<number>
  instance_groups: ReadValue<number | null>[]
  timers: ReadValue<number>[]
  manual_config_active: boolean
  feedback: {
    probed: boolean
    present: boolean
    opcode_map: 'diia_corrected' | 'ed1' | null
    capability: number | null
    colour_capability: number | null
    timing: number | null
    active_brightness: number | null
    active_colour: number | null
    inactive_brightness: number | null
    inactive_colour: number | null
  }
  runtime: {
    last_event_info: number | null
    last_event_at_ms: number | null
    event_count: number
    input_value: number | null
  }
}

export type InputDeviceDetail = InputDeviceSummary & {
  notes: string | null
  device_capabilities: number | null
  device_status: number | null
  version_number: number | null
  now_ms: number
  nvm_settling_until_ms: number | null
  instances: InputInstance[]
}

export interface PollerSettings {
  enabled: boolean
  interval_ms: number
  attribute_groups_default: AttributeGroup[]
  include_dt8_color: boolean
  include_energy: boolean
  include_diagnostics: boolean
  skip_unbound_virtual_lamps: boolean
}

export interface ControllerTime {
  synced: boolean
  unix_ms: number | null
  source: 'unset' | 'manual' | 'sntp'
  timezone: string
  local_minutes: number | null
  utc_offset_minutes: number | null
}

export type HclAlgorithm = 'stepped' | 'interpolated'
export type HclTimeRef = 'absolute' | 'sunrise' | 'sunset'
export type HclLevelMode = 'none' | 'absolute' | 'last_active'
export type Weekday = 'mon' | 'tue' | 'wed' | 'thu' | 'fri' | 'sat' | 'sun'

export interface HclLocation {
  latitude_deg: number
  longitude_deg: number
}

export interface HclTarget {
  adapter_id: number
  scope: 'group' | 'broadcast'
  group_ids?: number[]
}

export interface HclSchedulePoint {
  time_ref: HclTimeRef
  offset_minutes: number
  level_mode: HclLevelMode
  level: number | null
  color_temperature_kelvin: number | null
}

export interface HclSchedule {
  schedule_id: string
  enabled: boolean
  algorithm: HclAlgorithm
  active_days: Weekday[]
  location: HclLocation | null
  targets: HclTarget[]
  points: HclSchedulePoint[]
}

export interface HclScheduleList {
  schedules: HclSchedule[]
}

export interface HclOverrideTarget {
  adapter_id: number
  scope: 'group' | 'broadcast'
  group_id?: number
}

export interface HclOverride {
  suspended: boolean
  targets: HclOverrideTarget[]
  since_local_minutes?: number
}

export interface HomeAssistantSettings {
  enabled: boolean
  broker_host: string
  broker_port: number
  broker_username: string
  broker_password_set: boolean
  broker_url_view: string
  discovery_prefix: string
  state_topic_prefix: string
  controller_id: string
  publish_qos: number
  retain_state: boolean
  retain_discovery: boolean
  expose_input_devices: boolean
}

export interface HomeAssistantSettingsPatch
  extends Partial<Omit<HomeAssistantSettings, 'broker_password_set' | 'broker_url_view'>> {
  broker_password?: string
}

export interface RulesDocument {
  lang_id: number
  revision: number
  diagnostic: string | null
  rule_count: number
  source: string
}

export interface RuleJson {
  name: string
  enabled: boolean
  cooldown_ms: number
  hold_hcl: boolean
  triggers: unknown[]
  conditions: unknown[]
  actions: unknown[]
}

export interface RulesJsonProjection {
  lang_id: number
  blocks: unknown[]
  rules: RuleJson[]
}

export interface RulesDocumentJson {
  lang_id: number
  revision: number
  diagnostic: string | null
  rules: RulesJsonProjection | null
}

export interface RuleParseSummary {
  name: string
  enabled: boolean
}

export interface RulesParseOk {
  ok: true
  rules: RuleParseSummary[]
}

export interface RulesParseError {
  error: 'parse_error'
  line: number
  column: number
  message: string
}

export type RulesParseResult = RulesParseOk | RulesParseError

export interface RulePatched {
  name: string
  enabled: boolean
}

export interface FirmwareState {
  running_slot: string
  ota_capable: boolean
  pending_verify: boolean
  update: FirmwareUpdate
}

export interface FirmwareUpdate {
  state: 'idle' | 'downloading' | 'finishing' | 'ready_to_reboot' | 'failed'
  url: string
  downloaded_bytes: number
  total_bytes: number
  percent: number | null
  error: string | null
}
