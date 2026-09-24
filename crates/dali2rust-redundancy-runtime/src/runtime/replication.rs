use dali2rust_platform::http_fetch::{FetchError, HttpFetch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceDigest {
    pub name: String,
    pub bytes: Option<usize>,
    pub crc32: u32,
}

#[must_use]
pub fn slices_to_pull(peer: &[SliceDigest], local: &[SliceDigest]) -> Vec<String> {
    peer.iter()
        .filter(|row| row.bytes.is_some())
        .filter(|row| {
            local
                .iter()
                .find(|mine| mine.name == row.name)
                .is_none_or(|mine| mine.crc32 != row.crc32 || mine.bytes != row.bytes)
        })
        .map(|row| row.name.clone())
        .collect()
}

#[must_use]
pub fn should_pull(enabled: bool, application_active: bool, peer_url: &str) -> bool {
    enabled && !application_active && !peer_url.is_empty()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassOutcome {
    Idle,
    PeerUnreachable(FetchError),
    UpToDate,
    Pulled(Vec<String>),
}

pub const MANIFEST_PATH: &str = "/api/v1/config/slices";

#[must_use]
pub fn slice_path(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        match byte {
            b'/' | b'%' | b'?' | b'#' | b' ' => {
                encoded.push_str(&format!("%{byte:02X}"));
            }
            _ => encoded.push(char::from(byte)),
        }
    }
    format!("{MANIFEST_PATH}/{encoded}")
}

pub fn fetch_manifest(
    fetch: &dyn HttpFetch,
    peer_url: &str,
) -> Result<Vec<SliceDigest>, FetchError> {
    let body = fetch.get(peer_url, MANIFEST_PATH)?;
    parse_manifest(&body)
}

pub fn parse_manifest(body: &[u8]) -> Result<Vec<SliceDigest>, FetchError> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(body).map_err(|_| FetchError::Malformed)?;
    Ok(rows.iter().filter_map(digest_of).collect())
}

fn digest_of(row: &serde_json::Value) -> Option<SliceDigest> {
    let name = row.get("name")?.as_str()?.to_string();
    let bytes = match row.get("bytes") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(usize::try_from(v.as_u64()?).ok()?),
    };
    Some(SliceDigest {
        name,
        bytes,
        crc32: u32::try_from(row.get("crc32")?.as_u64()?).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, bytes: Option<usize>, crc: u32) -> SliceDigest {
        SliceDigest {
            name: name.to_string(),
            bytes,
            crc32: crc,
        }
    }

    #[test]
    fn only_a_passive_controller_with_a_peer_pulls() {
        assert!(should_pull(true, false, "http://peer"));
        assert!(!should_pull(true, true, "http://peer"));
        assert!(!should_pull(false, false, "http://peer"));
        assert!(!should_pull(true, false, ""));
    }

    #[test]
    fn a_slice_we_do_not_have_is_pulled() {
        let peer = vec![row("groups_a0", Some(40), 7)];
        assert_eq!(slices_to_pull(&peer, &[]), vec!["groups_a0"]);
    }

    #[test]
    fn a_same_length_edit_is_still_a_difference() {
        let peer = vec![row("poller_settings", Some(40), 7)];
        let local = vec![row("poller_settings", Some(40), 9)];
        assert_eq!(slices_to_pull(&peer, &local), vec!["poller_settings"]);
    }

    #[test]
    fn an_identical_slice_is_left_alone() {
        let peer = vec![row("poller_settings", Some(40), 7)];
        let local = vec![row("poller_settings", Some(40), 7)];
        assert!(slices_to_pull(&peer, &local).is_empty());
    }

    #[test]
    fn a_slice_the_peer_does_not_hold_is_skipped_rather_than_deleted() {
        let peer = vec![row("groups_a0", None, 0)];
        let local = vec![row("groups_a0", Some(40), 7)];
        assert!(slices_to_pull(&peer, &local).is_empty());
    }

    #[test]
    fn the_peers_order_survives_because_hydration_depends_on_it() {
        let peer = vec![
            row("physical_devices_a0", Some(9), 1),
            row("virtual_lamps_a0", Some(9), 2),
        ];
        assert_eq!(
            slices_to_pull(&peer, &[]),
            vec!["physical_devices_a0", "virtual_lamps_a0"]
        );
    }

    #[test]
    fn a_manifest_row_this_build_does_not_understand_is_skipped_not_fatal() {
        let body = br#"[
            {"name":"poller_settings","bytes":40,"crc32":7},
            {"name":"future_slice"},
            {"name":"groups_a0","bytes":null,"crc32":0}
        ]"#;
        let rows = parse_manifest(body).expect("manifest");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "poller_settings");
        assert_eq!(rows[1].bytes, None);
    }

    #[test]
    fn a_body_that_is_not_a_manifest_is_malformed() {
        assert_eq!(parse_manifest(b"not json"), Err(FetchError::Malformed));
    }

    #[test]
    fn the_slice_path_hangs_off_the_manifest_path() {
        assert_eq!(slice_path("groups_a0"), "/api/v1/config/slices/groups_a0");
    }

    #[test]
    fn an_adapter_scoped_name_stays_one_path_segment() {
        assert_eq!(slice_path("a0/groups"), "/api/v1/config/slices/a0%2Fgroups");
        assert_eq!(
            slice_path("a0/physical_devices"),
            "/api/v1/config/slices/a0%2Fphysical_devices"
        );
    }
}

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::msg::RegistrySliceReloadCommand;
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_domain::registry::{DaliSettingsReadPort, RedundancySettingsReadPort};

pub const REPLICATION_INTERVAL_MS: u64 = 30_000;

#[derive(Debug, Default)]
pub struct ReplicationCounters {
    pub passes: AtomicU32,
    pub peer_unreachable: AtomicU32,
    pub slices_pulled: AtomicU32,
    pub slices_rejected: AtomicU32,
    pub reload_publish_failed: AtomicU32,
}

pub trait ReplicationSink: Send + Sync {
    fn local_digests(&self) -> Vec<SliceDigest>;
    fn write_slice(&self, name: &str, bytes: &[u8]) -> bool;
}

pub struct ReplicationDeps {
    pub publisher: BusPublisher,
    pub bus_id: BusId,
    pub fetch: Arc<dyn HttpFetch>,
    pub sink: Arc<dyn ReplicationSink>,
    pub redundancy: Arc<dyn RedundancySettingsReadPort>,
    pub dali: Arc<dyn DaliSettingsReadPort>,
    pub counters: Arc<ReplicationCounters>,
}

pub fn run_pass(deps: &ReplicationDeps) -> PassOutcome {
    let settings = deps.redundancy.redundancy_settings_view();
    let active = deps.dali.dali_settings_view().application_active;
    if !should_pull(settings.enabled, active, &settings.peer_url) {
        return PassOutcome::Idle;
    }
    deps.counters.passes.fetch_add(1, Ordering::Relaxed);
    let peer = match fetch_manifest(deps.fetch.as_ref(), &settings.peer_url) {
        Ok(rows) => rows,
        Err(e) => {
            deps.counters.peer_unreachable.fetch_add(1, Ordering::Relaxed);
            return PassOutcome::PeerUnreachable(e);
        }
    };
    let wanted = slices_to_pull(&peer, &deps.sink.local_digests());
    if wanted.is_empty() {
        return PassOutcome::UpToDate;
    }
    let pulled = pull_each(deps, &settings.peer_url, &wanted);
    if pulled.is_empty() {
        return PassOutcome::UpToDate;
    }
    request_reload(deps, &pulled);
    PassOutcome::Pulled(pulled)
}

fn pull_each(deps: &ReplicationDeps, peer_url: &str, wanted: &[String]) -> Vec<String> {
    let mut pulled = Vec::new();
    for name in wanted {
        let Ok(bytes) = deps.fetch.get(peer_url, &slice_path(name)) else {
            deps.counters.slices_rejected.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if deps.sink.write_slice(name, &bytes) {
            deps.counters.slices_pulled.fetch_add(1, Ordering::Relaxed);
            pulled.push(name.clone());
        } else {
            deps.counters.slices_rejected.fetch_add(1, Ordering::Relaxed);
        }
    }
    pulled
}

fn request_reload(deps: &ReplicationDeps, pulled: &[String]) {
    let first = pulled.first().map(String::as_str).unwrap_or("");
    let name = if pulled.len() > 1 {
        format!("{first}+{}", pulled.len() - 1)
    } else {
        first.to_string()
    };
    let ce = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        CORRELATION_NONE,
        deps.bus_id.0,
        Some(dali2rust_contracts::msg::Origin::Poller),
        RegistrySliceReloadCommand {
            slice_name: dali2rust_contracts::msg::fixed_text_32(&name),
        },
    );
    if deps
        .publisher
        .try_publish(BusChannel::Commands, BusFrame::command(ce))
        != PublishResult::Queued
    {
        deps.counters
            .reload_publish_failed
            .fetch_add(1, Ordering::Relaxed);
    }
}

pub const REPLICATION_HANDLED_EVENTS: &[&str] = &["RedundancySettingsChangedEvent"];

pub fn spawn_replication_worker(
    ev_rx: dali2rust_bus::BusSubscriberRx,
    deps: ReplicationDeps,
) -> std::thread::JoinHandle<()> {
    dali2rust_bsp::esp_thread::spawn_named_stack_in(
        c"replication",
        dali2rust_bsp::std_thread_stack::EVENT_WORKER_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        move || replication_loop(&ev_rx, &deps),
    )
}

fn replication_loop(ev_rx: &dali2rust_bus::BusSubscriberRx, deps: &ReplicationDeps) {
    loop {
        if let PassOutcome::Pulled(slices) = run_pass(deps) {
            log::info!("replication: pulled {} slice(s) from the peer", slices.len());
        }
        let wait = std::time::Duration::from_millis(REPLICATION_INTERVAL_MS);
        if matches!(
            ev_rx.recv_timeout(wait),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        ) {
            return;
        }
    }
}
