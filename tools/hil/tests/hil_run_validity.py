import time

from hil import api as api_mod
from hil import config as config_mod
from hil import validity
from hil.seriallog import SerialLog
from hil_harness import UPTIME_SLACK_S, peer_health

HARDWARE_FREE_REPORT = ("hardware-free session: no collected test requests a bench "
                        "fixture, so no instrument was reached and nothing is classified")


UNCLASSIFIED_REPORT = ("collection stopped before the session was classified: no test "
                       "ran, no instrument was reached and nothing is classified")


def pytest_terminal_summary(terminalreporter, exitstatus, config):
    results = getattr(config, "_hil_step_results", [])
    tw = terminalreporter
    if results:
        tw.section("HIL steps")
        for status, name, reason in results:
            line = "STEP %s %s" % (name, status)
            if reason:
                line += " — %s" % reason.replace("Skipped: ", "")
            tw.write_line(line)
    lines = getattr(config, "_hil_validity_report", None)
    if lines:
        tw.section("HIL validity")
        for line in lines:
            tw.write_line(line)


def _validity_state(config):
    from hil.camera import server as server_mod

    state = dict(getattr(config, "_hil_validity", {}) or {})
    cfg = config_mod.load()
    try:
        strays = server_mod.stray_pids()
        state["frame_server"] = server_mod.describe(cfg) + (
            "" if len(strays) <= 1 else
            "  *** %d PROCESSES (%s) — they share one mailbox and corrupt each "
            "other's frames ***" % (len(strays),
                                    ", ".join(str(p) for p in strays)))
    except Exception:
        state["frame_server"] = "unknown"
    counters, retry_events = validity.tally(getattr(config, "_hil_counted", []))
    state["stack_budget"] = validity.load_stack_budget()
    census_lines = _stack_census_lines(config)
    state["stack_min_free"] = validity.stack_min_free(census_lines)
    state["stack_observation_gaps"] = validity.stack_observation_gaps(census_lines)
    state["observed_task_states"] = validity.observed_task_states(census_lines)
    state["boot_heap_budget"] = validity.load_boot_heap_budget()
    state["boot_heap"] = _boot_heap_ladder(config)
    state["runtime_heap_budget"] = validity.load_runtime_heap_budget()
    state["runtime_heap"] = _runtime_heap(cfg)
    state["stack_identity"] = dict(validity.run_identity(cfg.runs_dir),
                                   devices=_registry_device_count(cfg),
                                   version=_running_version(cfg))
    state["gear_segment"] = _gear_segment(cfg)
    state["peer"], state["peer_breach"] = _peer_state(
        cfg, getattr(config, "_hil_peer_start", None))
    state["bus_drops"] = _bus_drops(cfg, getattr(config, "_hil_isr_baseline", {}))
    state["bus_subscriber_losses"] = _bus_subscriber_losses(cfg)
    state["retry_events"] = retry_events
    state["ledger"] = validity.collect(
        dict(counters, bus_contended=len(state.get("contended") or [])))
    return state


def _peer_state(cfg, start):
    if not cfg.has_peer:
        return "none (single controller)", None
    peer = cfg.peer()
    health, end = peer_health(peer)
    breach, continuity = validity.peer_continuity(start, end, UPTIME_SLACK_S)
    identity = validity.run_identity(peer.runs_dir)
    return "%s version=%s (last flashed commit %s); %s" % (
        peer.base, (health or {}).get("version") or "unreachable",
        (identity.get("commit") or "?")[:12], continuity), breach


def _running_version(cfg):
    try:
        return api_mod.Client(cfg).health().get("version")
    except Exception:
        return None


def _gear_segment(cfg):
    wanted = cfg.gear_short_set()
    if wanted is None:
        return "whole registry (HIL_GEAR_SHORTS unset)"
    try:
        held = sorted(d["short_address"] for d in
                      api_mod.Client(cfg).devices_unfiltered()["physical_devices"])
    except Exception:
        held = None
    dropped = "unknown" if held is None else (
        ",".join(str(a) for a in held if a not in wanted) or "none")
    return ("RESTRICTED to %s by HIL_GEAR_SHORTS — not exercised: %s. "
            "A tier run this way is not an acceptance run."
            % (",".join(str(a) for a in sorted(wanted)), dropped))


def _registry_device_count(cfg):
    try:
        return len(api_mod.Client(cfg).devices_unfiltered()["physical_devices"])
    except Exception:
        return None


def _runtime_heap(cfg):
    try:
        return validity.runtime_heap_figures(api_mod.Client(cfg).stats())
    except Exception:
        return {}


def _bus_drops(cfg, isr_baseline=None):
    try:
        client = api_mod.Client(cfg)
        drops = validity.bus_drop_counters(client.diagnostics())
        stats = client.stats()
        timing = validity.isr_timing_counters(stats)
        baseline, baseline_uptime, baseline_host = isr_baseline or ({}, None, None)
        uptime = (stats.get("controller") or {}).get("uptime_ms")
        same_boot = validity.same_boot_interval(
            baseline_host, baseline_uptime, time.monotonic(), uptime,
            slack_s=UPTIME_SLACK_S)
        deltas = validity.counter_deltas(timing, baseline, same_boot=same_boot)
        drops.update(deltas)
        if same_boot and baseline_uptime is not None and uptime is not None:
            drops.update(validity.session_rates(deltas, int(uptime) - int(baseline_uptime)))
        drops.update(validity.absolute_counters(stats))
        return drops
    except Exception:
        return {}


def _bus_subscriber_losses(cfg):
    try:
        return validity.event_subscriber_losses(api_mod.Client(cfg).diagnostics())
    except Exception:
        return []


def _stack_census_lines(config):
    offset = getattr(config, "_hil_serial_offset", None)
    if offset is None:
        return []
    try:
        path = SerialLog(config_mod.load()).log_path
        if not path.exists():
            return []
        with open(path, errors="replace") as fh:
            fh.seek(offset)
            return fh.read().splitlines()
    except Exception:
        return []


def _boot_heap_ladder(config):
    offset = getattr(config, "_hil_serial_offset", None)
    if offset is None:
        return {}
    try:
        path = SerialLog(config_mod.load()).log_path
        if not path.exists():
            return {}
        with open(path, errors="replace") as fh:
            fh.seek(offset)
            return validity.boot_heap_ladder(fh.read().splitlines())
    except Exception:
        return {}


def _write_summary(config, lines):
    try:
        path = config_mod.load().run_dir() / "summary.md"
        body = ["# HIL run summary", "", "## Steps", ""]
        body += ["- STEP %s %s%s" % (name, status,
                                     (" — %s" % reason.replace("Skipped: ", ""))
                                     if reason else "")
                 for status, name, reason in getattr(config, "_hil_step_results", [])]
        body += ["", "## Validity", ""] + ["    " + line for line in lines] + [""]
        path.write_text("\n".join(body))
    except Exception as exc:
        print("could not write summary.md: %s" % exc)


def pytest_sessionfinish(session, exitstatus):
    config = session.config
    if config.getoption("--collect-only"):
        return
    verdict = getattr(config, "_hil_hardware_free", None)
    if verdict is not False:
        config._hil_validity_report = [
            HARDWARE_FREE_REPORT if verdict else UNCLASSIFIED_REPORT]
        return
    state = _validity_state(config)
    budget = validity.load_budget()
    lines = validity.format_report(state, budget)
    over = validity.breaches(state.get("ledger") or {}, budget)
    if over:
        lines.append("")
        lines += ["OVER BUDGET: %s absorbed %d, budget %d" % row for row in over]
        lines.append("A budget is raised with a dated measurement in "
                     "tools/hil/retry_budget.txt, never to make a run green.")
        session.exitstatus = 1
    shed = validity.bus_drop_breaches(state, budget)
    if shed:
        lines.append("")
        lines += ["BUS DROPS OVER BUDGET: %s dropped %d, budget %d" % row for row in shed]
        lines.append("A frame the bus shed is a fact nobody will re-send. If it "
                     "carried an operation's terminal outcome the operation ends "
                     "by TTL reporting a timeout for work that finished "
                     "(ISSUE-50, ADR-021) — find the producer, do not raise "
                     "tools/hil/retry_budget.txt.")
        session.exitstatus = 1
    deep = validity.stack_breaches(state.get("stack_min_free") or {},
                                   state.get("stack_budget") or {})
    if deep:
        lines.append("")
        lines += ["STACK OVER BUDGET: %s used %d B, budget %d B" % row for row in deep]
        lines.append("A task deeper than its budget is one section away from a "
                     "Stack protection fault (ISSUE-49). Shorten the path the "
                     "probe names, or move the growth off the stack — raising "
                     "tools/hil/stack_budget.txt is not the fix.")
        session.exitstatus = 1
    dead = validity.stack_budget_dead_lines(state.get("stack_min_free") or {},
                                            state.get("stack_budget") or {})
    if dead:
        lines.append("")
        lines.append("STACK BUDGET LINE MATCHES NO TASK: %s" % ", ".join(dead))
        lines.append("The census prints the name the task registry holds, and a "
                     "budget key spelled any other way measures nothing. Spell the "
                     "key as `task stack hwm` prints it; a task the firmware spawns "
                     "only on demand belongs in ON_DEMAND_TASKS (hil/validity.py).")
        session.exitstatus = 1
    gaps = state.get("stack_observation_gaps") or []
    if gaps:
        lines.append("")
        lines.append("STACK OBSERVATION STALE/MISSING: %s" % ", ".join(gaps))
        lines.append("A callback-owned task must publish a fresh watermark; a "
                     "cached kernel handle is never dereferenced after exit.")
        session.exitstatus = 1
    peer_breach = state.get("peer_breach")
    if peer_breach:
        lines.append("")
        lines.append("PEER CONTINUITY BROKEN: %s" % peer_breach)
        lines.append("The second controller shares the line: from that moment the "
                     "session measured next to a unit that was booting, arbitrating "
                     "or holding the bus. Find what touched it before trusting the run.")
        session.exitstatus = 1
    pending = state.get("production_state_pending")
    if pending:
        lines.append("")
        lines.append("PRODUCTION STATE: an earlier session never restored %s — it "
                     "was kept, and this session only restored to what it found. "
                     "Run `hil state restore` to put the owner's installation back."
                     % pending)
        session.exitstatus = 1
    residual = state.get("production_state_residual") or []
    if residual:
        lines.append("")
        lines += ["PRODUCTION STATE NOT RESTORED: %s" % line for line in residual]
        lines.append("The session left the owner's installation different from "
                     "how it found it. The snapshot is tools/hil/state/"
                     "production_state_last.json; `hil state restore` retries.")
        session.exitstatus = 1
    eroded = validity.boot_heap_breaches(state.get("boot_heap") or {},
                                        state.get("boot_heap_budget") or {})
    if eroded:
        lines.append("")
        lines += ["BOOT BASELINE BELOW FLOOR: %s left %d B free, floor %d B" % row
                  for row in eroded]
        lines.append("This release leaves less internal SRAM standing at boot "
                     "than the last one did, and every later dip starts from "
                     "that floor. The failure at the bottom is ESP_ERR_HTTPD_TASK "
                     "and a boot loop. Find what the stage named above now "
                     "allocates; lowering tools/hil/boot_heap_budget.txt needs a "
                     "dated reason.")
        session.exitstatus = 1
    thin = validity.runtime_heap_breaches(state.get("runtime_heap") or {},
                                          state.get("runtime_heap_budget") or {})
    if thin:
        lines.append("")
        lines += ["RUNTIME HEAP BELOW FLOOR: %s read %d B, floor %d B" % row
                  for row in thin]
        lines.append("Internal SRAM ran lower during this boot than the floor "
                     "allows. What needs internal SRAM at that moment — a new "
                     "task stack, httpd, DMA, TLS — fails. The serial line "
                     "`internal SRAM low-water` names the moment; lowering "
                     "tools/hil/runtime_heap_budget.txt needs a dated reason.")
        session.exitstatus = 1
    config._hil_validity_report = lines
    _write_summary(config, lines)
