use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use dali2rust_bus::{
    publish_required, BusChannel, BusFrame, BusId, BusPublisher, REQUIRED_PUBLISH_BACKOFF_MS,
    REQUIRED_PUBLISH_UNCAPPED,
};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{
    ErrorCode, Origin, RuleCommitCommand, RuleEnableCommand, RuleStageCommand, RulesChangedEvent,
    MAX_RULES_SOURCE_BYTES,
};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_rules_model::{NameResolver, RuleCompiler};

use super::persistence::{
    apply_enable_table, enable_table_of, fnv1a32, reassemble, text_banks, RulesManifest,
    RULES_MANIFEST_VERSION, RULES_TEXT_BANKS,
};
use super::rule_runtime::{classify, Firing};
use super::store::{RulesDocument, RulesStore};

pub const RULES_WORKER_REQUIRED_EVENTS: &[&str] = &["OperationWorkerSignalEvent"];

pub const RULES_WORKER_HANDLED_EVENTS: &[&str] = &[
    "DaliInputEventObservedEvent",
    "RuntimeStateChangedEvent",
    "DaliSceneRecalledEvent",
    "DaliInputDeviceLifecycleEvent",
    "DaliSettingsChangedEvent",
    "VirtualLampChangedEvent",
    "PhysicalDeviceChangedEvent",
    "RedundancyTransitionEvent",
    "Dali103InstanceConfiguredEvent",
    "RegistrySliceReloadedEvent",
];

#[derive(Debug, Default)]
pub struct RulesWorkerCounters {
    pub commits_applied: AtomicU32,
    pub commits_rejected: AtomicU32,
    pub enable_toggles: AtomicU32,
    pub hydrate_failed: AtomicU32,
    pub persist_failed: AtomicU32,
    pub ignored_commands: AtomicU32,
    pub effects_published: AtomicU32,
    pub effects_ingress_rejected: AtomicU32,
    pub effects_skipped_dark: AtomicU32,
    pub hcl_hold_unmapped: AtomicU32,
    pub hcl_schedule_unmapped: AtomicU32,
    pub input_action_unmapped: AtomicU32,
    pub log_lines: AtomicU32,
    pub stat_counts: AtomicU32,
    pub activations_published: AtomicU32,
}

#[derive(Default)]
struct Staging {
    correlation_id: u64,
    chunks: Vec<Option<Vec<u8>>>,
}

struct RulesWorker {
    publisher: BusPublisher,
    bus_id: BusId,
    store: Arc<RulesStore>,
    compiler: Arc<dyn RuleCompiler>,
    resolver: Arc<dyn NameResolver>,
    slices: Option<Arc<dyn SliceStore>>,
    counters: Arc<RulesWorkerCounters>,
    liveness: Arc<dali2rust_platform::liveness::LivenessBeat>,
    staging: Staging,
    world: Arc<dyn crate::runtime::world_port::RulesWorldPort>,
    engine: crate::runtime::engine::Engine,
    engine_cells: Arc<crate::runtime::stats::RulesEngineCells>,
    engine_revision: u32,
    funnel: super::funnel::FunnelState,
    latency: crate::runtime::stats::LatencyWindow,
    names_dirty: bool,
}

const TICK_CAP_MS: u64 = 1_000;

dali2rust_contracts::dispatch_bus_commands! {
    pub const RULES_WORKER_HANDLED_COMMANDS;
    fn dispatch_rules_command(
        payload: &dali2rust_contracts::msg::BusCommandPayload,
        worker: &mut RulesWorker,
        correlation_id: u64,
    );
    payload = payload;
    ignored = { worker.counters.ignored_commands.fetch_add(1, Ordering::Relaxed); };
    RuleStageCommand(chunk) => worker.on_stage(correlation_id, chunk),
    RuleCommitCommand(commit) => worker.on_commit(correlation_id, commit),
    RuleEnableCommand(toggle) => worker.on_enable(toggle),
    RuleRunCommand(run) => worker.on_run(correlation_id, run),
}

pub struct RulesWorkerSeams {
    pub store: Arc<RulesStore>,
    pub compiler: Arc<dyn RuleCompiler>,
    pub resolver: Arc<dyn NameResolver>,
    pub slices: Option<Arc<dyn SliceStore>>,
    pub world: Arc<dyn crate::runtime::world_port::RulesWorldPort>,
}

pub fn spawn_rules_worker(
    rx: dali2rust_bus::BusSubscriberRx,
    publisher: BusPublisher,
    bus_id: BusId,
    seams: RulesWorkerSeams,
    counters: Arc<RulesWorkerCounters>,
    engine_cells: Arc<crate::runtime::stats::RulesEngineCells>,
    liveness: Arc<dali2rust_platform::liveness::LivenessBeat>,
) -> std::thread::JoinHandle<()> {
    dali2rust_bsp::esp_thread::spawn_named_stack_in(
        c"rules-worker",
        dali2rust_bsp::std_thread_stack::COMMAND_WORKER_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        move || {
            let now = seams.world.now_ms();
            let mut worker = RulesWorker {
                publisher,
                bus_id,
                store: seams.store,
                compiler: seams.compiler,
                resolver: seams.resolver,
                slices: seams.slices,
                counters,
                liveness,
                staging: Staging::default(),
                world: seams.world,
                engine: crate::runtime::engine::Engine::new(now),
                engine_cells,
                engine_revision: u32::MAX,
                funnel: super::funnel::FunnelState::default(),
                latency: crate::runtime::stats::LatencyWindow::default(),
                names_dirty: false,
            };
            worker.hydrate();
            worker.reload_engine_if_moved();
            worker.feed(crate::runtime::engine::EngineInput::ControllerStarts, None);
            let active = worker.world.controller_active();
            if active {
                worker.feed(
                    crate::runtime::engine::EngineInput::ControllerActive { active: true },
                    None,
                );
            }
            worker.funnel.seed_active(active);
            worker.serve(&rx);
        },
    )
}

impl RulesWorker {
    fn on_stage(&mut self, correlation_id: u64, chunk: &RuleStageCommand) {
        if self.staging.correlation_id != correlation_id {
            self.staging = Staging {
                correlation_id,
                chunks: vec![None; usize::from(chunk.chunk_count)],
            };
        }
        let slot = usize::from(chunk.chunk_index);
        if slot < self.staging.chunks.len() {
            self.staging.chunks[slot] = Some(chunk.bytes.as_slice().to_vec());
        }
    }

    fn on_commit(&mut self, correlation_id: u64, commit: &RuleCommitCommand) {
        let staged = std::mem::take(&mut self.staging);
        match self.try_commit(correlation_id, commit, staged) {
            Ok(()) => {
                self.counters.commits_applied.fetch_add(1, Ordering::Relaxed);
                self.signal(correlation_id, None);
                self.publish_changed();
            }
            Err(message) => {
                self.counters.commits_rejected.fetch_add(1, Ordering::Relaxed);
                self.signal(correlation_id, Some(message));
            }
        }
    }

    fn try_commit(
        &mut self,
        correlation_id: u64,
        commit: &RuleCommitCommand,
        staged: Staging,
    ) -> Result<(), &'static str> {
        if staged.correlation_id != correlation_id
            || staged.chunks.len() != usize::from(commit.chunk_count)
        {
            return Err("rule_write_chunk_count_mismatch");
        }
        let mut source = Vec::with_capacity(usize::from(commit.total_len));
        for chunk in &staged.chunks {
            source.extend_from_slice(chunk.as_deref().ok_or("rule_write_chunk_missing")?);
        }
        if source.len() != usize::from(commit.total_len)
            || source.len() > MAX_RULES_SOURCE_BYTES
            || fnv1a32(&source) != commit.source_hash
        {
            return Err("rule_write_integrity");
        }
        if commit.base_revision != self.store.revision() {
            return Err("rule_set_conflict");
        }
        if commit.lang_id != self.compiler.lang_id() {
            return Err("rule_lang_unknown");
        }
        let text = String::from_utf8(source).map_err(|_| "rule_write_not_utf8")?;
        let compiled = self
            .compiler
            .compile(&text, self.resolver.as_ref())
            .map_err(|_| "rule_compile_failed")?;
        let doc = RulesDocument {
            source: text,
            lang_id: commit.lang_id,
            revision: self.store.revision().wrapping_add(1),
            enable_table: enable_table_of(&compiled),
            compiled: Some(compiled),
            diagnostic: None,
        };
        self.persist(&doc);
        self.store.replace(doc);
        Ok(())
    }

    fn serve(&mut self, rx: &dali2rust_bus::BusSubscriberRx) {
        loop {
            self.liveness.beat(dali2rust_platform::liveness::monotonic_ms());
            let now = self.world.now_ms();
            let deadline = self
                .engine
                .next_deadline_ms()
                .unwrap_or(now + TICK_CAP_MS)
                .clamp(now, now + TICK_CAP_MS);
            let wait = std::time::Duration::from_millis(deadline.saturating_sub(now).max(1));
            match self.liveness.while_turning(|| rx.recv_timeout(wait)) {
                Ok(frame) => self.on_frame(frame),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => self.on_tick(),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn on_frame(&mut self, frame: BusFrame) {
        self.reload_engine_if_moved();
        match frame {
            BusFrame::Command(ce) => {
                let correlation_id = ce.meta.correlation_id;
                dispatch_rules_command(&ce.payload, self, correlation_id);
            }
            BusFrame::Event(ev) => {
                if matches!(
                    ev.payload,
                    dali2rust_contracts::msg::BusEventPayload::RegistrySliceReloadedEvent(_)
                ) {
                    self.rehydrate_from_slices();
                }
                if names_may_have_moved(&ev.payload) {
                    self.names_dirty = true;
                }
                let inputs = self.funnel.map(&ev.payload, self.world.as_ref());
                for input in inputs {
                    self.feed(input, None);
                }
            }
            BusFrame::Confirmation(_) => {}
        }
    }

    fn on_tick(&mut self) {
        if std::mem::take(&mut self.names_dirty) {
            self.recompile_in_place();
        }
        self.reload_engine_if_moved();
        let edges = self.funnel.tick_edges(self.world.as_ref());
        for input in edges {
            self.feed(input, None);
        }
        self.feed(crate::runtime::engine::EngineInput::Tick, None);
    }

    fn on_run(&mut self, correlation_id: u64, cmd: &dali2rust_contracts::msg::RuleRunCommand) {
        let enabled = self
            .store
            .document()
            .compiled
            .as_ref()
            .and_then(|set| set.rule(cmd.name.as_str()).map(|r| r.enabled));
        match enabled {
            None => {
                self.signal(correlation_id, Some("rule_not_found"));
                return;
            }
            Some(false) if !cmd.dry => {
                self.signal(correlation_id, Some("rule_disabled"));
                return;
            }
            Some(_) => {}
        }
        self.feed(
            crate::runtime::engine::EngineInput::RunRule {
                name: cmd.name.as_str(),
                dry: cmd.dry,
            },
            Some(correlation_id),
        );
        self.signal(correlation_id, None);
    }

    fn feed(&mut self, input: crate::runtime::engine::EngineInput<'_>, run_corr: Option<u64>) {
        let from_failure = matches!(input, crate::runtime::engine::EngineInput::RuleFailed { .. });
        let started = self.world.now_ms();
        let snapshot = self.snapshot(started);
        let outcomes = self.engine.handle(input, &snapshot);
        let mut failed_rules: Vec<usize> = Vec::new();
        let corr = run_corr.unwrap_or(CORRELATION_NONE);
        for (index, outcome) in outcomes.iter().enumerate() {
            if self.execute_outcome(outcome, &snapshot, corr, started) {
                failed_rules.push(index);
            }
        }
        self.engine_cells.store(&self.engine.counters());
        if from_failure {
            return;
        }
        for index in failed_rules {
            let name = outcomes[index].rule.clone();
            self.feed(
                crate::runtime::engine::EngineInput::RuleFailed { name: &name },
                None,
            );
        }
    }

    fn execute_outcome(
        &mut self,
        outcome: &crate::runtime::engine::ActivationOutcome,
        snapshot: &crate::runtime::engine::WorldSnapshot,
        corr: u64,
        started: u64,
    ) -> bool {
        let executor = crate::runtime::executor::EffectExecutor {
            publisher: &self.publisher,
            bus_id: self.bus_id,
            world: self.world.as_ref(),
            counters: &self.counters,
        };
        let report = if outcome.dry {
            crate::runtime::executor::ExecutionReport::default()
        } else {
            executor.execute(&outcome.effects, snapshot, corr)
        };
        let latency = u16::try_from(self.world.now_ms().saturating_sub(started)).unwrap_or(u16::MAX);
        self.latency.note(latency, &self.engine_cells);
        self.publish_activation(outcome, report.executed, latency);
        if outcome.dry {
            return false;
        }
        let (result, error) = classify(outcome.partial, report.executed, report.failed);
        let firing = Firing { at_ms: self.world.unix_ms(), latency_ms: latency, outcome: result, error };
        self.store.record_firing(&outcome.rule, firing);
        report.failed > 0 || outcome.partial.is_some()
    }

    fn snapshot(&self, now_ms: u64) -> crate::runtime::engine::WorldSnapshot {
        crate::runtime::engine::WorldSnapshot {
            now_ms,
            wall: self.world.wall(),
            sun: self.world.sun(),
            controller_active: self.world.controller_active(),
            lamps: self.world.lamps(),
            groups: self.world.groups(),
            devices: self.world.devices(),
            inputs: self.world.inputs(),
            hcl: self.world.hcl(),
        }
    }

    fn publish_activation(
        &self,
        outcome: &crate::runtime::engine::ActivationOutcome,
        executed: u8,
        latency: u16,
    ) {
        let ev = event_envelope(
            SOURCE_ID_UNSPECIFIED,
            CORRELATION_NONE,
            self.bus_id.0,
            Some(Origin::Rules),
            dali2rust_contracts::msg::RulesActivationEvent {
                rule_name: dali2rust_contracts::msg::fixed_text_64(&outcome.rule),
                dry: outcome.dry,
                effects: executed,
                partial: partial_code(outcome.partial.as_ref()),
                trigger_to_publish_ms: latency,
            },
        );
        if self
            .publisher
            .try_publish(BusChannel::Events, BusFrame::event(ev))
            == dali2rust_bus::PublishResult::Queued
        {
            self.counters.activations_published.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn recompile_in_place(&mut self) {
        let doc = self.store.document();
        if doc.lang_id != self.compiler.lang_id() {
            return;
        }
        let next = match self.compiler.compile(&doc.source, self.resolver.as_ref()) {
            Ok(mut set) => {
                apply_enable_table(&mut set, &doc.enable_table);
                Some(set)
            }
            Err(_) => None,
        };
        if next == doc.compiled {
            return;
        }
        if next.is_none() {
            self.counters.hydrate_failed.fetch_add(1, Ordering::Relaxed);
        }
        let diagnostic = if next.is_none() {
            Some("rules_names_unresolved".to_owned())
        } else {
            None
        };
        self.store.replace(RulesDocument {
            revision: doc.revision.wrapping_add(1),
            compiled: next,
            diagnostic,
            ..doc
        });
    }

    fn rehydrate_from_slices(&mut self) {
        if self.slices.is_none() {
            return;
        }
        self.hydrate();
        let doc = self.store.document();
        self.engine_revision = doc.revision;
        self.store.retain_runtime(doc.compiled.as_ref());
        self.engine.set_rules(doc.compiled, self.world.now_ms());
        self.funnel.set_watch_groups(self.engine.watches_groups());
        self.engine_cells.store(&self.engine.counters());
    }

    fn reload_engine_if_moved(&mut self) {
        let doc = self.store.document();
        if doc.revision == self.engine_revision {
            return;
        }
        self.engine_revision = doc.revision;
        self.store.retain_runtime(doc.compiled.as_ref());
        self.engine.set_rules(doc.compiled, self.world.now_ms());
        self.funnel.set_watch_groups(self.engine.watches_groups());
        self.engine_cells.store(&self.engine.counters());
    }

    fn on_enable(&mut self, toggle: &RuleEnableCommand) {
        let name = toggle.name.as_str();
        let Some(revision) = self.store.set_enabled(name, toggle.enabled) else {
            self.counters.ignored_commands.fetch_add(1, Ordering::Relaxed);
            return;
        };
        self.counters.enable_toggles.fetch_add(1, Ordering::Relaxed);
        self.persist(&self.store.document());
        self.flip_engine_bit(name, toggle.enabled, revision);
        self.publish_changed();
    }

    fn flip_engine_bit(&mut self, name: &str, enabled: bool, revision: u32) {
        let in_step = self.engine_revision == revision.wrapping_sub(1);
        if in_step && self.engine.set_rule_enabled(name, enabled) {
            self.engine_revision = revision;
            self.engine_cells.store(&self.engine.counters());
        }
    }

    fn publish_changed(&self) {
        let doc = self.store.document();
        let ev = event_envelope(
            SOURCE_ID_UNSPECIFIED,
            CORRELATION_NONE,
            self.bus_id.0,
            Some(Origin::Api),
            RulesChangedEvent {
                revision: doc.revision,
                rule_count: doc
                    .compiled
                    .as_ref()
                    .map(|set| u8::try_from(set.rules.len()).unwrap_or(u8::MAX))
                    .unwrap_or(0),
                lang_id: doc.lang_id,
            },
        );
        let _ = self.publisher.try_publish(
            dali2rust_bus::BusChannel::Events,
            BusFrame::event(ev),
        );
    }

    fn signal(&self, correlation_id: u64, error: Option<&'static str>) {
        let signal = match error {
            None => dali2rust_contracts::msg::OperationWorkerSignalEvent::succeeded(correlation_id),
            Some(message) => dali2rust_contracts::msg::OperationWorkerSignalEvent::failed(
                correlation_id,
                error_code(message),
                message,
            ),
        };
        let ev = event_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            Some(Origin::Api),
            signal,
        );
        let _ = self.liveness.while_turning(|| {
            publish_required(
                &self.publisher,
                BusChannel::Events,
                BusFrame::event(ev),
                &REQUIRED_PUBLISH_BACKOFF_MS,
                REQUIRED_PUBLISH_UNCAPPED,
                "rules-worker-signal",
            )
        });
    }

    fn persist(&self, doc: &RulesDocument) {
        let Some(slices) = &self.slices else { return };
        if write_banks(slices.as_ref(), doc).is_err() {
            self.counters.persist_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn hydrate(&mut self) {
        let Some(slices) = self.slices.clone() else { return };
        match load_document(slices.as_ref(), self.compiler.as_ref(), self.resolver.as_ref()) {
            Ok(Some(doc)) => {
                if doc.compiled.is_none() {
                    self.counters.hydrate_failed.fetch_add(1, Ordering::Relaxed);
                }
                self.store.replace(doc);
            }
            Ok(None) => {}
            Err(diagnostic) => {
                self.counters.hydrate_failed.fetch_add(1, Ordering::Relaxed);
                self.store.replace(RulesDocument {
                    diagnostic: Some(diagnostic.to_string()),
                    ..RulesDocument::default()
                });
            }
        }
    }
}

fn partial_code(partial: Option<&crate::runtime::engine::PartialReason>) -> u8 {
    match partial {
        None => 0,
        Some(crate::runtime::engine::PartialReason::ConditionUnevaluable) => 1,
        Some(crate::runtime::engine::PartialReason::EffectBudget) => 2,
        Some(crate::runtime::engine::PartialReason::ChainDepth) => 3,
    }
}

fn error_code(message: &str) -> ErrorCode {
    match message {
        "rule_set_conflict" => ErrorCode::Conflict,
        _ => ErrorCode::OperationFailed,
    }
}

fn write_banks(slices: &dyn SliceStore, doc: &RulesDocument) -> Result<(), ()> {
    for (key, bytes) in text_banks(doc.source.as_bytes()) {
        if bytes.is_empty() && slices.load(key).map_or(true, |held| held.is_empty()) {
            continue;
        }
        write_one(slices, key, &bytes)?;
    }
    let entries = doc.enable_table.clone();
    let manifest = RulesManifest {
        version: RULES_MANIFEST_VERSION,
        lang_id: doc.lang_id,
        total_len: u16::try_from(doc.source.len()).unwrap_or(u16::MAX),
        source_hash: fnv1a32(doc.source.as_bytes()),
        revision: doc.revision,
        entries,
    };
    let bytes = postcard::to_allocvec(&manifest).map_err(|_| ())?;
    write_one(slices, SliceKey::Rules { bank: 3 }, &bytes)
}

fn write_one(slices: &dyn SliceStore, key: SliceKey, bytes: &[u8]) -> Result<(), ()> {
    let mut session = slices.begin_write(key).map_err(|_| ())?;
    session.append(bytes).map_err(|_| ())?;
    session.commit().map_err(|_| ())
}

fn load_document(
    slices: &dyn SliceStore,
    compiler: &dyn RuleCompiler,
    resolver: &dyn NameResolver,
) -> Result<Option<RulesDocument>, &'static str> {
    let manifest_bytes = match slices.load(SliceKey::Rules { bank: 3 }) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let manifest: RulesManifest =
        postcard::from_bytes(&manifest_bytes).map_err(|_| "rules_manifest_undecodable")?;
    if manifest.version != RULES_MANIFEST_VERSION {
        return Err("rules_manifest_version");
    }
    let mut banks = Vec::new();
    for bank in 0..RULES_TEXT_BANKS {
        banks.push(slices.load(SliceKey::Rules { bank }).ok());
    }
    let source = reassemble(&manifest, &banks).ok_or("rules_banks_torn")?;
    let text = String::from_utf8(source).map_err(|_| "rules_source_not_utf8")?;
    if manifest.lang_id != compiler.lang_id() {
        return Err("rules_lang_unknown");
    }
    let mut compiled = match compiler.compile(&text, resolver) {
        Ok(set) => set,
        Err(e) => return Ok(Some(uncompiled_document(text, &manifest, &e))),
    };
    apply_enable_table(&mut compiled, &manifest.entries);
    Ok(Some(RulesDocument {
        source: text,
        lang_id: manifest.lang_id,
        revision: manifest.revision,
        compiled: Some(compiled),
        diagnostic: None,
        enable_table: manifest.entries,
    }))
}

fn uncompiled_document(
    text: String,
    manifest: &RulesManifest,
    error: &dali2rust_rules_model::CompileError,
) -> RulesDocument {
    RulesDocument {
        source: text,
        lang_id: manifest.lang_id,
        revision: manifest.revision,
        compiled: None,
        diagnostic: Some(format!("rules_compile_failed: {error}")),
        enable_table: manifest.entries.clone(),
    }
}

fn names_may_have_moved(payload: &dali2rust_contracts::msg::BusEventPayload) -> bool {
    matches!(
        payload,
        dali2rust_contracts::msg::BusEventPayload::VirtualLampChangedEvent(_)
            | dali2rust_contracts::msg::BusEventPayload::PhysicalDeviceChangedEvent(_)
    )
}
