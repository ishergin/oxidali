import re
from collections import Counter
from pathlib import Path

NOT_MEASURED = -1
BUDGET_FILE = Path(__file__).resolve().parent.parent / "retry_budget.txt"
STACK_BUDGET_FILE = Path(__file__).resolve().parent.parent / "stack_budget.txt"
BOOT_HEAP_BUDGET_FILE = Path(__file__).resolve().parent.parent / "boot_heap_budget.txt"
RUNTIME_HEAP_BUDGET_FILE = Path(__file__).resolve().parent.parent / "runtime_heap_budget.txt"

BOOT_HEAP_RE = re.compile(r"boot heap \[([^\]]+)\]: internal free=(\d+)")
BEFORE_HTTPD_RE = re.compile(r"internal heap before httpd: free=(\d+)")

CENSUS_RE = re.compile(r"task stack hwm \(B free(?:,[^)]*)?\):\s*(.*)$")
OBSERVED_TASK_RE = re.compile(
    r"\b(httpd|mqtt_task)=(\d+)@0x[0-9a-f]+@(\d+):(fresh|stale)\b")


def load_budget(path=None):
    path = Path(path or BUDGET_FILE)
    budget = {}
    try:
        lines = path.read_text().splitlines()
    except OSError:
        return budget
    for line in lines:
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) >= 2:
            try:
                budget[parts[0]] = int(parts[1])
            except ValueError:
                continue
    return budget


def load_stack_budget(path=None):
    path = Path(path or STACK_BUDGET_FILE)
    budget = {}
    try:
        lines = path.read_text().splitlines()
    except OSError:
        return budget
    for line in lines:
        line = line.split("#", 1)[0].strip()
        parts = line.split()
        if len(parts) < 3:
            continue
        try:
            budget[parts[0]] = (int(parts[1]), int(parts[2]))
        except ValueError:
            continue
    return budget


def load_boot_heap_budget(path=None):
    path = Path(path or BOOT_HEAP_BUDGET_FILE)
    budget = {}
    try:
        lines = path.read_text().splitlines()
    except OSError:
        return budget
    for line in lines:
        line = line.split("#", 1)[0].strip()
        parts = line.split()
        if len(parts) < 2:
            continue
        try:
            budget[parts[0]] = int(parts[1])
        except ValueError:
            continue
    return budget


def load_runtime_heap_budget(path=None):
    return load_boot_heap_budget(path or RUNTIME_HEAP_BUDGET_FILE)


RUNTIME_HEAP_FIELDS = (
    ("internal_min_free_bytes", "internal_min_free"),
    ("internal_free_bytes", "internal_free"),
    ("rust_internal_live_bytes", "rust_internal_live"),
    ("rust_internal_peak_bytes", "rust_internal_peak"),
    ("rust_psram_peak_bytes", "rust_psram_peak"),
)


def runtime_heap_figures(stats):
    controller = (stats or {}).get("controller") or {}
    return {name: int(controller[field]) for field, name in RUNTIME_HEAP_FIELDS
            if controller.get(field) is not None}


def runtime_heap_breaches(figures, budget):
    out = []
    for name, floor in sorted(budget.items()):
        value = figures.get(name)
        if floor != NOT_MEASURED and value is not None and value < floor:
            out.append((name, value, floor))
    return out


def boot_heap_ladder(lines):
    out = {}
    def note(stage, free):
        if stage not in out or free < out[stage]:
            out[stage] = free
    for line in lines:
        found = BOOT_HEAP_RE.search(line)
        if found:
            note(found.group(1).strip().replace(" ", "_"), int(found.group(2)))
        found = BEFORE_HTTPD_RE.search(line)
        if found:
            note("before_httpd", int(found.group(1)))
    return out


def boot_heap_breaches(ladder, budget):
    out = []
    for stage, floor in sorted(budget.items()):
        free = ladder.get(stage)
        if free is not None and free < floor:
            out.append((stage, free, floor))
    return out


def stack_min_free(lines):
    out = {}
    for line in lines:
        found = CENSUS_RE.search(line)
        if not found:
            continue
        for field in found.group(1).split():
            name, _, value = field.partition("=")
            if not name or not value:
                continue
            try:
                free = int(value)
            except ValueError:
                continue
            if name not in out or free < out[name]:
                out[name] = free
        for name, free, _age, freshness in OBSERVED_TASK_RE.findall(found.group(1)):
            if freshness != "fresh":
                continue
            free = int(free)
            if name not in out or free < out[name]:
                out[name] = free
    return out


GATED_OBSERVED_TASKS = ("httpd",)


def observed_task_states(lines):
    states = {}
    for line in lines:
        found = CENSUS_RE.search(line)
        if not found or "| observed:" not in found.group(1):
            continue
        body = found.group(1)
        for name in ("httpd", "mqtt_task"):
            states.setdefault(name, set())
            if re.search(r"\b%s=missing\b" % name, body):
                states[name].add("missing")
        for name, _free, _age, freshness in OBSERVED_TASK_RE.findall(body):
            states.setdefault(name, set()).add(freshness)
    return states


def stack_observation_gaps(lines):
    states = observed_task_states(lines)
    return [name for name in GATED_OBSERVED_TASKS
            if name in states and "fresh" not in states[name]]


ON_DEMAND_TASKS = ("ws-client",)


def stack_budget_orphans(min_free, budget):
    if not min_free:
        return []
    return [task for task in sorted(budget)
            if task not in min_free and task not in GATED_OBSERVED_TASKS]


def stack_budget_dead_lines(min_free, budget):
    return [task for task in stack_budget_orphans(min_free, budget)
            if task not in ON_DEMAND_TASKS]


def stack_breaches(min_free, budget):
    out = []
    for task, (stack_bytes, max_used) in sorted(budget.items()):
        if max_used == NOT_MEASURED or task not in min_free:
            continue
        used = stack_bytes - min_free[task]
        if used > max_used:
            out.append((task, used, max_used))
    return out


def uptime_broke(prev, now_ts, now_uptime, slack_s):
    if prev is None:
        return None
    prev_ts, prev_uptime = prev
    expected = prev_uptime + (now_ts - prev_ts)
    shortfall = expected - (now_uptime + slack_s)
    return shortfall if shortfall > 0 else None


def tally(instruments):
    counted, optical, events, fallbacks = Counter(), Counter(), [], 0
    for instrument in instruments:
        if hasattr(instrument, "retry_causes"):
            optical.update(instrument.retry_causes)
            continue
        fallbacks += getattr(instrument, "witness_fallbacks", 0)
        counted.update(getattr(instrument, "retries", None) or {})
        events.extend(getattr(instrument, "retry_events", ()))
    return {"api": counted, "optical": optical, "witness_fallbacks": fallbacks}, events


def collect(counters):
    ledger = {}
    for name, value in (counters.get("api") or {}).items():
        ledger[name] = ledger.get(name, 0) + int(value)
    for cause, value in (counters.get("optical") or {}).items():
        ledger["optical_" + cause] = ledger.get("optical_" + cause, 0) + int(value)
    for name in ("witness_fallbacks", "bus_contended"):
        if counters.get(name):
            ledger[name] = ledger.get(name, 0) + int(counters[name])
    return ledger


def breaches(ledger, budget):
    out = []
    for name, count in sorted(ledger.items()):
        limit = budget.get(name, NOT_MEASURED)
        if limit != NOT_MEASURED and count > limit:
            out.append((name, count, limit))
    return out


PREVIOUS_STACK_FILE = Path(__file__).resolve().parent.parent / "state" / "stack_previous.json"


def run_identity(runs_dir):
    import json
    try:
        manifests = sorted(Path(runs_dir).glob("*/manifest.json"))
    except Exception:
        return {}
    for path in reversed(manifests):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            continue
        return {
            "commit": (data.get("git") or {}).get("commit"),
            "firmware": (data.get("firmware") or {}).get("sha256"),
        }
    return {}


def _describe_identity(identity):
    commit = (identity or {}).get("commit") or "?"
    devices = (identity or {}).get("devices")
    return "version %s, flashed commit %s, %s devices" % (
        (identity or {}).get("version") or "?",
        commit[:12],
        "?" if devices is None else devices,
    )


def peer_continuity(start, end, slack_s):
    if start is None:
        return None, "uptime unverified: the peer did not answer at session start"
    if end is None:
        breach = ("the peer stopped answering by session end (it was up %.0fs at the "
                  "start) — a reboot presents exactly this way" % start[1])
        return breach, breach
    shortfall = uptime_broke(start, end[0], end[1], slack_s)
    if shortfall is None:
        return None, "uptime continuous (%.0fs -> %.0fs)" % (start[1], end[1])
    breach = ("the peer REBOOTED during the session: uptime %.0fs -> %.0fs across "
              "%.0fs" % (start[1], end[1], end[0] - start[0]))
    return breach, breach


def _load_previous_stacks():
    try:
        import json
        data = json.loads(PREVIOUS_STACK_FILE.read_text(encoding="utf-8"))
        return data.get("used", {}), data.get("identity", {})
    except Exception:
        return {}, {}


def _save_stacks(used, identity):
    try:
        import json
        PREVIOUS_STACK_FILE.parent.mkdir(parents=True, exist_ok=True)
        PREVIOUS_STACK_FILE.write_text(
            json.dumps({"used": used, "identity": identity or {}}, indent=1),
            encoding="utf-8",
        )
    except Exception:
        pass


def _boot_heap_lines(state):
    ladder = state.get("boot_heap") or {}
    budget = state.get("boot_heap_budget") or {}
    if not ladder:
        return ["boot baseline: no boot in this run's serial window "
                "(nothing rebooted, or the monitor was down) — not measured"]
    order = ["composition_start", "before_hydrate", "after_hydrate",
             "before_ota_worker", "after_ota_worker", "after_workers",
             "before_httpd"]
    seen = [k for k in order if k in ladder] + \
           [k for k in sorted(ladder) if k not in order]
    out = ["boot baseline (internal B free at each stage):"]
    for stage in seen:
        free = ladder[stage]
        floor = budget.get(stage)
        verdict = ""
        if floor is not None:
            verdict = "  floor %d%s" % (floor, "  *** BELOW ***" if free < floor else "")
        out.append("  %-20s %8d%s" % (stage, free, verdict))
    return out


def _runtime_heap_lines(state):
    figures = state.get("runtime_heap") or {}
    budget = state.get("runtime_heap_budget") or {}
    if not figures:
        return ["runtime heap: no /api/v1/stats reading at session end — not measured"]
    out = ["runtime heap at session end (bytes; minimum is since the last boot):"]
    for field, name in RUNTIME_HEAP_FIELDS:
        if name not in figures:
            continue
        floor = budget.get(name)
        verdict = ""
        if floor == NOT_MEASURED:
            verdict = "  floor unmeasured"
        elif floor is not None:
            verdict = "  floor %d%s" % (floor, "  *** BELOW ***" if figures[name] < floor else "")
        out.append("  %-20s %8d%s" % (name, figures[name], verdict))
    return out


def _stack_lines(state):
    min_free = state.get("stack_min_free") or {}
    budget = state.get("stack_budget") or {}
    if not min_free:
        return ["stack headroom: NO per-task census in the serial log for this "
                "run — the budgets gated nothing, AND the httpd freshness "
                "check gated nothing either, because both read the same lines. "
                "Look for `task stack hwm` in the serial log: absent means the "
                "monitor missed the window, and `MEASURED NOTHING` means the "
                "firmware could not take the census at all"]
    identity = state.get("stack_identity") or {}
    known = bool(identity) and all(v is not None for v in identity.values())
    previous, previous_identity = _load_previous_stacks()
    comparable = known and identity == previous_identity
    if previous and not comparable:
        previous = {}
    current = {}
    out = ["stack headroom (budget from stack_budget.txt):"]
    for task, free in sorted(min_free.items()):
        row = budget.get(task)
        if row is None:
            out.append("  %-20s %6d B free   (no budget line)" % (task, free))
            continue
        stack_bytes, max_used = row
        used = stack_bytes - free
        current[task] = used
        if max_used == NOT_MEASURED:
            verdict = "unmeasured — set a budget"
        elif used > max_used:
            verdict = "OVER BUDGET %d" % max_used
        else:
            verdict = "budget %d" % max_used
        was = previous.get(task)
        trend = "" if was is None else "  (prev %d, %+d)" % (was, used - was)
        out.append("  %-20s %6d used of %-6d %s%s"
                   % (task, used, stack_bytes, verdict, trend))
    for task in stack_budget_orphans(min_free, budget):
        why = ("spawned on demand, none this run" if task in ON_DEMAND_TASKS
               else "*** it gates nothing ***")
        out.append("  %-20s budget line matches no task in the census — %s"
                   % (task, why))
    if not comparable and not previous_identity and not previous:
        out.append("  (no trend: no census stored yet — the next run has one to "
                   "compare against)")
    elif not comparable:
        out.append(
            "  (no trend: stored census is %s, this run is %s — a delta across "
            "two different measurements is not a slope)"
            % (
                _describe_identity(previous_identity) if previous_identity
                else "unidentified",
                _describe_identity(identity) if identity else "unidentified",
            )
        )
    out.extend(_observed_task_lines(state))
    _save_stacks(current, identity)
    return out


def _observed_task_lines(state):
    states = state.get("observed_task_states") or {}
    if not states:
        return []
    out = ["  observed (own-callback probes):"]
    for name in sorted(states):
        seen = ", ".join(sorted(states[name])) or "never seen"
        gated = "gated" if name in GATED_OBSERVED_TASKS else "reported"
        out.append("    %-12s %-24s %s" % (name, seen, gated))
    return out


def bus_drop_counters(diagnostics):
    if not diagnostics:
        return {}
    out = {}
    bus = diagnostics.get("bus") or {}
    for channel in ("commands", "confirmations", "events"):
        overflow = (bus.get(channel) or {}).get("ingress_overflow")
        if overflow is not None:
            out["bus_%s_ingress_overflow" % channel] = int(overflow)
    worker = diagnostics.get("dali_worker") or {}
    for field, name in (
        ("event_publish_failed", "dali_event_publish_failed"),
        ("confirmation_publish_failed", "dali_confirmation_publish_failed"),
    ):
        if worker.get(field) is not None:
            out[name] = int(worker[field])
    if worker.get("event_publish_retried") is not None:
        out["dali_event_publish_retried"] = int(worker["event_publish_retried"])
    subscribers = (diagnostics.get("bus") or {}).get("event_subscribers") or []
    dropped = sum(int(s.get("receiver_overflow") or 0) for s in subscribers)
    out["bus_event_subscriber_overflow"] = dropped
    return out


def event_subscriber_losses(diagnostics: dict) -> list:
    subscribers = (diagnostics.get("bus") or {}).get("event_subscribers") or []
    losses = [
        (s.get("name") or "#%d" % i, int(s.get("receiver_overflow") or 0))
        for i, s in enumerate(subscribers)
    ]
    return sorted([row for row in losses if row[1] > 0], key=lambda row: -row[1])


def isr_timing_counters(stats):
    if not stats:
        return {}
    dali = stats.get("dali") or {}
    out = {}
    for field, name in (
        ("isr_ticks_lost_total", "isr_ticks_lost"),
        ("isr_ticks_extra_total", "isr_ticks_extra"),
        ("isr_late_ticks_total", "isr_late_ticks"),
        ("isr_max_gap_us", "isr_max_gap_us"),
        ("backward_undecodable_total", "backward_undecodable"),
        ("backward_frame_size_total", "backward_frame_size"),
        ("backward_incomplete_total", "backward_incomplete"),
        ("backward_early_rejected_total", "backward_early_rejected"),
        ("backward_late_rejected_total", "backward_late_rejected"),
        ("backward_multi_answer_total", "backward_multi_answer"),
        ("isr_ticks_deficit_raw_total", "isr_ticks_deficit_raw"),
        ("isr_ticks_surplus_raw_total", "isr_ticks_surplus_raw"),
        ("console_log_dropped_total", "console_log_dropped"),
        ("console_log_busy_total", "console_log_busy"),
        ("console_log_truncated_total", "console_log_truncated"),
        ("console_uart_errors_total", "console_uart_errors"),
        ("answer_staged_total", "answer_staged"),
        ("answer_stage_late_total", "answer_stage_late"),
        ("answer_stage_max_ticks", "answer_stage_max_ticks"),
        ("sniff_poll_late_total", "sniff_poll_late"),
        ("sniff_poll_gap_max_us", "sniff_poll_gap_max_us"),
        ("persist_flush_total", "persist_flush"),
        ("persist_flush_slow_total", "persist_flush_slow"),
        ("persist_flush_ms_total", "persist_flush_ms"),
        ("persist_flush_max_ms", "persist_flush_max_ms"),
        ("persist_gate_waits_total", "persist_gate_waits"),
        ("persist_gate_timeouts_total", "persist_gate_timeouts"),
    ):
        if dali.get(field) is not None:
            out[name] = int(dali[field])
    return out


def absolute_counters(stats):
    if not stats:
        return {}
    dali = stats.get("dali") or {}
    out = {}
    for field, name in (("console_log_unavailable_total", "console_log_unavailable"),):
        if dali.get(field) is not None:
            out[name] = int(dali[field])
    return out


GAUGES = (
    "isr_max_gap_us",
    "answer_stage_max_ticks",
    "sniff_poll_gap_max_us",
    "persist_flush_max_ms",
)


def counter_deltas(current, baseline, gauges=GAUGES, same_boot=True):
    if not current or not baseline or not same_boot:
        return {}
    return {
        name: value if name in gauges else (int(value) - int(baseline[name])) & 0xFFFF_FFFF
        for name, value in current.items()
        if name in baseline
    }


RATE_COUNTERS = (
    "isr_ticks_lost",
    "isr_ticks_extra",
    "isr_ticks_deficit_raw",
    "answer_stage_late",
    "sniff_poll_late",
    "persist_flush_slow",
)

MIN_RATE_EXPOSURE_MS = 10 * 60 * 1000


def session_rates(deltas, elapsed_ms, names=RATE_COUNTERS):
    if not elapsed_ms or elapsed_ms < MIN_RATE_EXPOSURE_MS:
        return {}
    return {
        "%s_per_h" % name: -(-int(deltas[name]) * 3_600_000 // int(elapsed_ms))
        for name in names
        if name in deltas
    }


def same_boot_interval(start_host_s, start_uptime_ms, end_host_s, end_uptime_ms,
                       slack_s=30.0):
    if None in (start_host_s, start_uptime_ms, end_host_s, end_uptime_ms):
        return False
    host_elapsed_ms = (float(end_host_s) - float(start_host_s)) * 1000.0
    uptime_elapsed_ms = int(end_uptime_ms) - int(start_uptime_ms)
    if host_elapsed_ms < 0 or uptime_elapsed_ms < 0:
        return False
    return abs(uptime_elapsed_ms - host_elapsed_ms) <= float(slack_s) * 1000.0


UNGATED_BUS_COUNTERS = (
    "dali_event_publish_retried",
    "isr_late_ticks",
    "isr_max_gap_us",
    "backward_late_rejected",
    "backward_multi_answer",
    "backward_early_rejected",
    "isr_ticks_deficit_raw",
    "isr_ticks_surplus_raw",
    "console_log_dropped",
    "console_log_busy",
    "console_log_truncated",
    "console_uart_errors",
)


def _bus_drop_lines(state, budget):
    drops = state.get("bus_drops")
    if not drops:
        return ["bus drops: /api/v1/diagnostics not read for this run — the DUT's "
                "own drop counters are the ISSUE-50 evidence and this run has none"]
    lines = ["bus drops (budget from retry_budget.txt):"]
    for name, count in sorted(drops.items()):
        if name in UNGATED_BUS_COUNTERS:
            lines.append("  %-28s %5d   not gated (mechanism exercised)" % (name, count))
            continue
        limit = budget.get(name, NOT_MEASURED)
        if limit == NOT_MEASURED:
            verdict = "unmeasured — set a budget"
        elif count > limit:
            verdict = "OVER BUDGET %d" % limit
        else:
            verdict = "budget %d" % limit
        lines.append("  %-28s %5d   %s" % (name, count, verdict))
    for who, lost in state.get("bus_subscriber_losses") or []:
        lines.append("      lost by %-20s %5d" % (who, lost))
    return lines


def bus_drop_breaches(state, budget):
    drops = {
        name: count
        for name, count in (state.get("bus_drops") or {}).items()
        if name not in UNGATED_BUS_COUNTERS
    }
    return breaches(drops, budget)


def format_report(state, budget):
    lines = []
    lines.append("frame server: %s" % (state.get("frame_server") or "not checked"))
    reboots = state.get("reboots") or []
    lines.append("DUT uptime: %s" % (
        "continuous" if not reboots else
        "BROKEN — %d reboot(s) mid-run: %s" % (len(reboots), "; ".join(reboots))))
    baseline = state.get("baseline") or []
    observed = state.get("observed")
    lines.append("bench baseline: %s%s" % (
        "as expected" if not baseline else "; ".join(baseline),
        (" [%s]" % observed) if observed else ""))
    lines.append("gear segment: %s" % (
        state.get("gear_segment") or "not checked"))
    lines.append("peer controller: %s" % (state.get("peer") or "not checked"))
    lines.append("production state: %s" % (
        state.get("production_state") or "not guarded (no DUT in this session)"))
    lines.append("WB sniffer: %s" % (state.get("wb") or "not exercised by this run"))
    if state.get("camera_exposure"):
        lines.append("camera exposure: %s" % state["camera_exposure"])
    recals = state.get("recalibrations") or []
    lines.append("optics recalibrated mid-run: %s" % (
        "no (profile stayed inside the drift window)" if not recals
        else "%d x — %s" % (len(recals), "; ".join(recals))))

    lines += _boot_heap_lines(state)
    lines += _runtime_heap_lines(state)
    lines += _stack_lines(state)
    lines += _bus_drop_lines(state, budget)

    ledger = dict(state.get("ledger") or {})
    exchanges = ledger.pop("raw_exchanges", 0)
    if exchanges:
        unanswered = ledger.get("raw_unanswered", 0)
        lines.append("diagnostic raw exchanges: %d unanswered of %d (%.1f%%) "
                     "— expected 0 since 957a2cd"
                     % (unanswered, exchanges, 100.0 * unanswered / exchanges))
    if not ledger:
        lines.append("retries: none recorded")
        return lines
    lines.append("retries absorbed (budget from retry_budget.txt):")
    for name, count in sorted(ledger.items()):
        limit = budget.get(name, NOT_MEASURED)
        if limit == NOT_MEASURED:
            verdict = "unmeasured — set a budget"
        elif count > limit:
            verdict = "OVER BUDGET %d" % limit
        else:
            verdict = "budget %d" % limit
        lines.append("  %-28s %5d   %s" % (name, count, verdict))
    lines += _retry_event_lines(state)
    return lines


MAX_LISTED_RETRY_EVENTS = 12


def _retry_event_lines(state):
    events = list(state.get("retry_events") or [])
    if not events:
        return []
    lines = ["  what moved them:"]
    for event in events[:MAX_LISTED_RETRY_EVENTS]:
        lines.append("    %s" % event)
    if len(events) > MAX_LISTED_RETRY_EVENTS:
        lines.append("    ... and %d more" % (len(events) - MAX_LISTED_RETRY_EVENTS))
    return lines
