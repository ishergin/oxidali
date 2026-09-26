# HIL toolkit

pytest-based hardware-in-the-loop tests of the firmware on a real controller and a real
DALI line. The oracles are the controller's HTTP/WebSocket API and serial console, a
second DALI master as an independent bus sniffer and foreign master (a Wiren Board
WB-MDALI3, for example), and, when one is attached, a USB camera. This file is
the runbook: what a bench needs, how to configure it, and how to run and read the suite.

Scope: setting up, running and reading the suite. Strategy, the coverage map and
the issue watchlist live in [STRATEGY.md](STRATEGY.md) (Russian); firmware build
knobs and the version string in
[10-build-release-and-tooling.md](../../documentation/architecture/10-build-release-and-tooling.md).

## What a bench needs

| Instrument | Enables | Without it |
| --- | --- | --- |
| A controller running this firmware, reachable over HTTP (`HIL_BASE`) | every tier | nothing runs |
| Its serial console, on this host's USB or behind a serial bridge on another host | `serial`, reboot evidence, `hil monitor`, `hil flash` | `hil preflight` reports it; deselect `serial` |
| A DALI line with bus power and at least one lamp the suite may drive | every test that changes the light | `HIL_LAMPS_READ_ONLY=1` keeps the light untouched |
| A UVC camera on a fixed mount with the lamps in view | `optical` ([Optical tests](#optical-tests)) | `--no-camera` turns optical tests into skips |
| A second DALI master reachable over ssh and MQTT (WB-MDALI3 on a Wiren Board) | `sniffer`, `foreign`, and `ha_bridge` through its broker | the `sniffer` fixture skips; deselect `foreign` and `ha_bridge` |
| A second controller on the same line (`HIL_PEER_BASE`) | `redundancy` | those tests skip |

macOS is the tested host: camera discovery goes through AVFoundation, `setup.sh` builds
`uvc-util` with the macOS frameworks, and the frame server runs in Terminal. The rest is
Python 3.9 or later, pyserial and esptool.

## Configuration

Every variable is read from the environment when a command starts. `bench.env` (copied
from `bench.env.example`) holds only the firmware build knobs that `hil flash` enforces;
it does not configure the toolkit. **The defaults name one particular bench.** Set at
least `HIL_BASE`, the serial variables and the three short-address sets before the first
run: every pytest session that collects a bench test reaches `HIL_BASE`, and
`bench_baseline` suspends that controller's poller and HCL schedules and sets its time
zone to `HIL_BENCH_TZ` for the session. A session whose every test is
[hardware-free](#writing-a-scenario) touches no network and no serial port.

| Variable | Meaning | Default |
| --- | --- | --- |
| `HIL_BASE` | the controller's URL | a reference bench's controller |
| `HIL_SERIAL_REMOTE` | `user@host:/dev/…` of the controller's UART behind a [serial bridge](#serial-ports); empty means this host's USB | a reference bench's Wiren Board |
| `HIL_SERIAL_PORT` | the local serial device when `HIL_SERIAL_REMOTE` is empty | found by scanning |
| `HIL_SERIAL_BAUD`, `HIL_FLASH_BAUD`, `HIL_SERIAL_BRIDGE_PORT` | console baud, esptool baud, the bridge's local port | `115200`, `1500000`, `4444` |
| `HIL_LAMP_SHORTS` | short addresses a test may drive or write (`0,2,3`, `0-3`); the client refuses the rest ([lamp guard](#the-installation-is-production)); empty allows none | `0,2,3` |
| `HIL_GEAR_SHORTS` | gear the read tiers target; empty means every gear in the registry | empty |
| `HIL_OPTICAL_SHORTS` | lamps in the camera's view, calibrated and measured; always narrowed to `HIL_LAMP_SHORTS` | `HIL_LAMP_SHORTS` |
| `HIL_LAMPS_READ_ONLY=1` | report the light, never drive it: the client refuses every visible action | off |
| `HIL_NO_CAMERA=1` (`--no-camera`) | optical tests skip instead of failing | off |
| `HIL_CAMERA_NAME`, `HIL_CAMERA_ID` | which camera to open | `USB Camera` |
| `HIL_SKIP_CALIBRATION=1` (`--skip-calibration`), `HIL_CALIBRATION_TTL_S` | reuse the saved calibration; how old it may be | recalibrate; `900` s |
| `HIL_LAMP_ROSTER=registry` | take the lamp roster from the registry's bound virtual lamps | the calibration |
| `HIL_WB_SSH`, `HIL_WB_DEVICE`, `HIL_WB_BUS` | the sniffer host over ssh, its DALI device and bus | a reference bench's Wiren Board |
| `HIL_MQTT_BROKER_HOST`, `HIL_MQTT_BROKER_PORT` | the broker the `ha_bridge` tier uses | a reference bench's Wiren Board, `1883` |
| `HIL_PEER_BASE`, `HIL_PEER_SERIAL_PORT`, `HIL_PEER_SERIAL_REMOTE`, `HIL_PEER_SERIAL_BRIDGE_PORT` | the pair's [second controller](#the-second-controller-hil---peer) | unset, unset, a reference bench's Wiren Board, `4446` |
| `HIL_ALLOW_DESTRUCTIVE=1`, `HIL_POWER_CUT=1`, `HIL_BUS_SHORT=1` | allow the destructive and the person-at-the-rig tests | off |
| `HIL_STATE_GUARD=0` | turn the [save and restore](#save-and-restore) guard off | on |
| `HIL_BENCH_TZ` | the time zone `bench_baseline` holds the controller in | `MSK-3` |
| `HIL_GEAR_SIM_PORT` | the [gear emulator](../dali-gear-sim/README.md)'s serial port | unset |
| `HIL_ADAPTER`, `HIL_BOARD`, `HIL_RUN_DIR` | the DALI adapter id, the board, the artifact directory | `0`, `esp32p4`, `runs/current` |

## First run

```bash
tools/hil/setup.sh                                   # .venv, the hil package, uvc-util
cp tools/hil/bench.env.example tools/hil/bench.env   # firmware knobs `hil flash` enforces
cd tools/hil
export HIL_BASE=http://<controller> HIL_SERIAL_REMOTE= HIL_SERIAL_PORT=<device>
export HIL_LAMP_SHORTS=<lamps> HIL_GEAR_SHORTS=<gear> HIL_OPTICAL_SHORTS=<lamps in view>
.venv/bin/hil preflight                              # read-only readiness check
.venv/bin/python3 -m pytest -m smoke --no-camera     # the instruments themselves
.venv/bin/python3 -m pytest --no-camera \
  -m "not destructive and not slow and not sniffer and not foreign and not ha_bridge and not redundancy"
```

Drop `--no-camera` and the markers of the instruments the bench has. The toolkit's own
unit tests need no hardware and no controller; pointing `HIL_BASE` at a closed port
keeps a regressed hook from reaching a bench:

```bash
HIL_BASE=http://127.0.0.1:9 HIL_SERIAL_REMOTE= HIL_NO_CAMERA=1 \
  .venv/bin/python3 -m pytest tests/*_unit.py                                           # tools/hil
```

## The installation is production

The suite is built to run on production lighting ([STRATEGY.md](STRATEGY.md) §2), and its
standing rules — the go-ahead for visible actions, what is always allowed, which
lamps, save and restore, reboots, destructive tests, exclusive use — are STRATEGY §4.
`HIL_LAMP_SHORTS` names the lamps a test may drive; set `HIL_GEAR_SHORTS` on every
run — empty, the default, means every gear in the registry.

The lamp guard (`hil/lamp_guard.py`) enforces `HIL_LAMP_SHORTS` and
`HIL_LAMPS_READ_ONLY` in the toolkit, whatever a test or a script selects. Every request `Client` sends (`_http`, `raw_request`,
`raw_response`) and every frame the WB foreign master sends passes it first, and a
refusal raises `LampNotAllowed`, naming the address and the reason, before anything
reaches the wire:

- A physical-device, virtual-lamp (by its binding) or identify target, or a diagnostic
  frame that writes the gear (DAPC, an opcode below `0x90`, `ACTIVATE`), must address
  a short in `HIL_LAMP_SHORTS`. Queries always pass.
- A group or broadcast frame, a group target-state and a scene recall pass only when
  every present gear on the segment is in `HIL_LAMP_SHORTS`.
- Under `HIL_LAMPS_READ_ONLY=1` every visible action is refused: target-state, identify,
  scene recall, and a frame that is DAPC, an arc-power command (`GO TO SCENE`
  included), `RESET`, `IDENTIFY DEVICE` or `ACTIVATE`. Configuration writes to an
  allowed lamp still pass (STRATEGY §4).
- A `/api/v1/dali/*` request whose body names no frame the guard can read is refused.
- HCL schedules, rules and MQTT commands drive lamps inside the controller, past the
  guard: a test that uses them picks its targets from the `lamps` fixture.

## Save and restore

The session-scoped autouse fixture `production_state` (`hil/prod_state.py`) runs
for every session that reaches the controller:

- **Before** anything else it primes the registry with an attribute read of every
  gear (runtime state is unknown after a reboot) and snapshots controller
  configuration (settings, rules, HCL schedules, device metadata, virtual-lamp
  bindings, group and scene matrices), gear configuration (fade, power-on and
  system-failure level, min/max, dimming curve, the gear's own group membership and
  scene levels) and what every lamp shows.
- **After** the session it restores each layer, re-reads every gear before putting
  the light back (the diagnostic `/dali/*` path moves a lamp without telling the
  registry) and diffs against the snapshot. A non-empty residual prints
  `PRODUCTION STATE NOT RESTORED` and turns the run red.
- It cannot restore the colour a DT8 gear stored with a scene, or anything outside
  the controller (the Wiren Board's configuration, Home Assistant).
- The snapshot is also `state/production_state_last.json`. A session killed before
  its teardown is recovered with `hil state restore`; such a snapshot is kept, and
  every later session ends red until it has been restored. `hil state save` and
  `hil state diff [FILE]` take and compare snapshots by hand.
- Under `HIL_LAMPS_READ_ONLY=1` the light is reported, never driven — by this
  fixture, by `state_snapshot` and by `hil state restore`. A lamp outside
  `HIL_LAMP_SHORTS` is never driven back, and the lamp guard refuses a repair of its
  gear groups and scenes: a difference there stays a residual.
- Across several tiers each session snapshots its own "before": keep the first
  snapshot and `hil state diff` against it at the end. `HIL_STATE_GUARD=0` is the
  only opt-out.
- It compares and restores only the fields it names (the poller's and Home Assistant's
  `RESTORABLE` tuples, `prod_state`'s settings, gear-configuration and device-record
  lists), and so do the settings guards: a writable field the firmware adds is named
  there too, or a test that changes it leaks past the session with no residual.
- Fields an enabled HCL schedule drives (level, colour or both, on the members of its
  target groups by the matrix's applied column, or every device for broadcast) are
  neither compared nor restored: the schedule moves them during a session, and writing
  one back would be an API write the scheduler takes as an override.
- Part 103 control devices are neither snapshotted nor restored — their configuration
  lives in the panel's NVM — so a test writes a panel field only with the value the
  panel already holds.
- A request that can move a lamp through `/api/v1/dali/*` goes through `Client._http`
  (never `raw_response` or a bare session), which books the shorts it touches so
  `restore_states` re-reads them before comparing; `raw_response` passes the lamp
  guard but books nothing.
- A session fixture that changes the installation takes `production_state` as a
  parameter even when its body does not use it: pytest sets up one conftest's autouse
  fixtures in alphabetical order, and only that dependency puts the snapshot first.

Per-test guards (`state_snapshot`, `*_matrix_guard`, `hcl_guard`, …) restore what
one test changed. `bench_baseline` suspends the poller and HCL schedules for the
session and fails it on a device still named `hil-…` by an earlier run.

## Layout

| Path | Holds |
| --- | --- |
| `hil/` | the library and the `hil` CLI |
| `tests/` | the suites; `conftest.py` holds the session fixtures and the safety gates |
| `wb/serial_bridge.py` | the one file deployed to the Wiren Board |
| `corpus/` | frozen captures of the installation (`hil corpus`); local, and only the files host tests pin are tracked (`.gitignore`) |
| `*_budget.txt` | run-validity budgets (below) |
| `state/`, `runs/<ts>/` | gitignored bench state (calibration, monitor, serial log, pins, snapshots) and per-run artifacts; `runs/current` is the latest |

`state/` belongs to the checkout: a monitor started from another checkout or
worktree is invisible to `hil` run here, and a flash from here then fights it for
the port.

## Optical tests

The camera is the oracle for what a lamp really shows, whatever the controller reports.
The `optical` tests (`test_optical_*`, and the optical cases in `test_scenes`,
`test_scenarios`, `test_hcl_runtime` and `test_ha_bridge`) check that:

- each lamp in `HIL_OPTICAL_SHORTS` switches on and off;
- brightness rises strictly along a ladder of levels;
- RGB primaries come out with the right hue, and a colour sent without a level lights;
- 2700 K and 6500 K order by their red-to-blue ratio;
- a group recall, a scene, a scheduled HCL level and a colour temperature sent over MQTT
  reach the fixtures themselves.

The camera is a UVC camera on a fixed mount with the lamps, or the patches they light,
in view. `setup.sh` builds `uvc-util`, which locks exposure and white balance. macOS
denies an agent the camera, so frames come from a frame server (`hil camera-server
--spawn-terminal`, approve it in the Terminal). Exactly one may run; replace a wedged
one with `--restart`.

Calibration (`hil calibrate`, or automatically before the first test that needs optics)
locks the camera, takes two all-off baselines for the noise floor, lights each lamp alone
to find its mask, tunes the exposure against clipping, measures crosstalk thresholds and
a brightness ladder, records RGB and colour-temperature fingerprints and picks fiducials
that reveal a camera that moved. The profile is saved in `state/` and reused for
`HIL_CALIBRATION_TTL_S`; `--skip-calibration` reuses it regardless. **Calibration drives
every lamp in `HIL_OPTICAL_SHORTS`**, so it is a visible action.

A dead optical channel **fails** optical tests; `--no-camera` / `HIL_NO_CAMERA=1` turns
that into skips. With no calibrated fixture on the wire, `HIL_LAMP_ROSTER=registry`
takes the lamp roster from the registry's bound virtual lamps.

## Running tests

Always from `tools/hil/` (`testpaths` and `addopts` are relative):
`.venv/bin/python3 -m pytest …`. `just hil-preflight | hil-smoke | hil-default |
hil-slow` wrap the cwd.

- The default run excludes `destructive` and `slow`. An explicit `-m` **replaces**
  that filter (pytest keeps the last `-m`), so the destructive tier is also gated by
  `HIL_ALLOW_DESTRUCTIVE=1`: without it those tests skip whatever `-m` says. Every
  test that reboots or halts a controller is `destructive`: collection refuses one
  that requests `dut_reboot` or calls `remote_serial.control` (itself or through a
  helper in its module) without the marker.
- The tiers, their order and what each is for: STRATEGY §5. Add
  `--junitxml=runs/current/junit.xml --html=runs/current/report.html` for reports.
- `hil preflight` judges the serial channel by the log growing after a health request
  and the smoke tier by the firmware's `HTTP access: GET /api/v1/health …` line, so that
  line is part of the firmware's contract with the toolkit.
- Markers: `smoke`, `optical`, `sniffer`, `serial`, `foreign` (the WB master),
  `ha_bridge`, `redundancy`, `slow`, `destructive`, `needs_capability(name)`.
- `--fast-fade` sets the fade time of every lamp in `HIL_LAMP_SHORTS` to 0 after
  `production_state` has taken its snapshot, and `production_state` restores it at
  the end of the session; with `HIL_STATE_GUARD=0` the option is a usage error.
- Person-at-the-rig tests skip unless asked for: `HIL_POWER_CUT=1` (cut the
  controller's power mid-write), `HIL_BUS_SHORT=1` (short the bus — every lamp on
  the wire sees it).
- The `ha_bridge` tier uses the WB's mosquitto under test-only `hiltest*` prefixes;
  `ha_guard` restores the live settings and sweeps the test topics, never touching
  `broker_password`. Acceptance against the live Home Assistant is manual.
- Artifacts land in `runs/<ts>/<test-id>/`.

## Run validity — how a red run is classified

Every run ends with an `HIL validity` section (also in `runs/<ts>/summary.md`): frame
servers (FAIL on more than one), DUT uptime continuity (a reboot fails the test it
landed in, unless that test uses `dut_reboot`), the baseline and the gear-segment
restriction (a restricted run is *not an acceptance run*), the peer, the
production-state result, WB availability, the retry ledger, the controller's bus
drop counters, per-task stack headroom (a run whose log has no census says so; a
budget line no census task matches fails the run unless the task is spawned on
demand), the boot heap ladder and the runtime heap floor. **Classify a red run from this section, not from the serial log.**

| File | Bounds | Moves |
| --- | --- | --- |
| `retry_budget.txt` | what a session may absorb | down only |
| `stack_budget.txt` | how deep each firmware task may go | down only |
| `boot_heap_budget.txt` | internal SRAM left at each boot stage | up only (a floor) |
| `runtime_heap_budget.txt` | internal SRAM minimum since boot, read from `/api/v1/stats` at session end | up only (a floor) |

Changing a budget against its direction needs a dated measurement line in the file.
`-1` means "never measured with a counter attached": reported, not gated — replace it
with the first instrumented figure. A retry is counted, never silent; a dead
instrument (a blind colour sample) is charged to no budget. A red test keeps its
strict assert ([05](../../documentation/architecture/05-testing-and-bdd.md)); the budget
carries the rate the installation imposes.

## CLI (`.venv/bin/hil`)

| Command | Purpose |
| --- | --- |
| `preflight` | read-only readiness: DUT HTTP, serial bridge, monitor, WB ssh, broker, camera, calibration, host tools |
| `monitor start\|stop\|status\|tail` | persistent serial monitor; `start` raises the bridge if needed |
| `remote start [--restart]\|stop\|status\|ping\|bootloader\|run` | the WB serial bridge and its tunnel |
| `flash [--build-only] [--allow-nonbench-build] [--allow-red-isr] [--allow-stale-ui]` | the only flash path (below) |
| `state save\|restore\|diff [FILE]` | the installation snapshot |
| `api <sub> …` | manual API calls; exit 2 means the firmware lacks the capability |
| `corpus [parts…]` | freeze both boards' configuration, REST, wire replies and serial evidence into `corpus/`; slices in `SECRET_SLICES` (the Home Assistant slice carries the broker password) are withheld unless `--keep-secrets` writes into an untracked `--out` — a slice that comes to carry a secret joins that list, because a pinned file gets committed |
| `calibrate`, `camera-server`, `camera-bench`, `lamps [--no-baseline]` | optics; `lamps` switches every calibrated lamp **off** for its baseline unless `--no-baseline` |
| `decode <log>` | decode a sniffer capture |

`hil --peer <command>` runs any of them against the other controller; `--peer` goes
before the command. Every command parses its arguments strictly: an unknown or
misspelt flag (`--peer` after the command included), an unknown subcommand or a stray
argument exits 64 before anything runs. `hil api`'s exit 2 keeps its meaning.

## Serial ports

A board on this host's USB needs only `HIL_SERIAL_REMOTE=` (empty) and, to pin it,
`HIL_SERIAL_PORT`. On a reference bench both controllers hang off a Wiren Board's USB
instead, named by USB serial number under `/dev/serial/by-id/`;
`HIL_SERIAL_REMOTE` and `HIL_PEER_SERIAL_REMOTE` name them.
`wb/serial_bridge.py` (pyserial only; nothing is installed on the WB) holds each UART
open for its whole life and exposes two loopback listeners: data, speaking RFC2217
so esptool's baud change reaches the real UART, and one port above it a control
port (`ping`, `status`, `bootloader`, `run`). An ssh tunnel brings them here: the
primary is `rfc2217://127.0.0.1:4444`, the peer `…:4446`. The limitations of the USB
bridge, pyserial and esptool this set-up works around are
[ISSUE-148](../../documentation/product-design/known-issues.md).

- The bridge serves **one client** and refuses a second rather than evicting the
  first.
- Attaching to a live bridge resets nothing. **Starting a bridge that is down opens
  the port on the WB and resets the controller.** `hil remote start`, `hil monitor
  start` and `hil flash` all start it when needed (`--restart` always does); first
  check on the WB whether it still listens (`ss -ltn`) — if only the tunnel died,
  the start reuses it. A changed `wb/serial_bridge.py` is copied to the WB but a live
  bridge keeps running the old one until `--restart`, which resets the controller.
- A bridge whose port is gone — the node vanished, the board was enumerated again
  under the same name, or a read failed under a client — answers every control
  command `err port-gone: …`. `hil remote`, `hil monitor start`, `hil flash` and
  `hil preflight` fail on it instead of reusing a bridge that carries no data;
  `--restart` reopens the port and so resets the controller, which is the operator's
  decision.
- Reset sequences run on the WB.
- Logs: `state/persist/serial.log` (peer: `state/peer/persist/`).

## Flashing

`hil flash` is the only way to flash the installation; `cargo flash` and `just
p4-fw-flash` open a local port and cannot reach it. It refuses a build without the
`bench.env` knobs and a stale embedded UI bundle (`--allow-stale-ui` builds with the
previous one), runs the ISR-IRAM gate on the linked binary (refusing a red one),
records `runs/<ts>/manifest.json` (commit, binary sha, knobs, checks), resets the
chip into its ROM loader through the bridge, writes with esptool at
`HIL_FLASH_BAUD`, always asks for `run` afterwards (a board left in the loader looks
bricked from the LAN), waits for `/api/v1/health` and requires the reported version
to equal the one inside the image it wrote — a prefix-of-`HEAD` check only when the
image yields none — and records that version in the manifest. `MCU` and the sdkconfig
baseline are read from `.cargo/config.toml` and asserted, never set. From a host
outside the WB's LAN esptool never syncs: update over the network instead.

A variable set in `bench.env` overrides the same variable exported in the shell, so a
one-off knob given as `KNOB=… hil flash` takes effect only when `bench.env` does not set
it.

**Over the network (OTA)** — the second path; what only the wire can do is in
[ADR-024](../../documentation/architecture/decisions/ADR-024-ota-over-ethernet.md).

```bash
espflash save-image --chip esp32p4 --flash-size 32mb --partition-table partitions-p4.csv \
  target/riscv32imafc-esp-espidf/debug/dali2rust app.bin   # an app image, not a merged one
# serve app.bin over HTTP from a host the controller can reach, then:
curl -X POST http://$DUT/api/v1/firmware/updates -H 'Content-Type: application/json' \
  -d '{"url":"http://<host>:<port>/app.bin"}'               # 202 + operation id
curl -s http://$DUT/api/v1/firmware                          # progress, running slot
```

It passes when `running_slot` has flipped and `pending_verify` has cleared
([contract](../../documentation/product-design/rest-api/resources/firmware.md)). OTA
writes no manifest and checks no version — compare `/api/v1/health` by hand.

## The second controller (`hil --peer`)

`hil --peer <command>` is the same toolkit pointed at the pair's other board: its
URL (`HIL_PEER_BASE`, or the lease its firmware announces on serial, learned on the
first `hil --peer flash` into `state/peer/base.pin`), its serial bridge, and its own
`state/peer/` and `runs/peer/`, so nothing of the primary's is repointed. The
`redundancy` tier reaches it through `peer_api` and skips without one. A reboot the
suite provokes leaves the peer active and the rebooted board standby; `dut_reboot`
and the OTA test hand the role back (`hil/pair.py`) before the next assertion.

## Writing a scenario

```python
@pytest.mark.hil_id("HIL-DIAG-09")
@pytest.mark.sniffer
def test_status_query_reaches_the_wire(api, sniffer, paced, op_check):
    short = api.present_addrs()[0]          # through the gear filter, never a literal
    with sniffer.window() as win:           # independent bus witness
        paced(1.0)                          # the WB ring drops faster bursts
        op_check(api.wait_op(api.attr_read(short, groups="runtime_status")))
        win.expect_frame("QUERY STATUS")    # decoded DALI on the wire
```

Every scenario carries its `HIL-<DOMAIN>-NN` id as a `hil_id` marker (registered in
`pyproject.toml`; the suite runs with `--strict-markers`); find a scenario by grepping
its id, and `-m hil_id` selects every test that has one. Put anything the test writes
back through a guard. Keep visible actions out of tests that do not need them — each
one costs a go-ahead.

- **Targets.** A test that changes what a lamp shows takes its target from the `lamps`
  fixture or `cfg.lamp_short_set()`; `optical_addrs()`, `present_optical_addrs()`,
  `lamp_addrs()` and `off_all()` stay inside `HIL_LAMP_SHORTS` too. `addrs()`,
  `present_addrs()` and `devices()` honour only `HIL_GEAR_SHORTS`: they pick read
  targets. Whatever the selector, the [lamp guard](#the-installation-is-production)
  refuses a drive outside `HIL_LAMP_SHORTS`, and a group or broadcast action unless
  the whole segment is allowed; a gear-wide command such as `REMOVE FROM SCENE` is
  sent per lamp.
- **Reboots.** A test that reboots a controller carries `destructive`. A test or
  fixture that reboots one (reset, flash, OTA, power cut, bootloader request) does
  so inside `Client.expect_reboot()`: only there are the
  failed requests that follow booked as `http_reboot_race` and every client's pooled
  connections dropped; outside it they spend the gated `http_transport` budget.
- **`202` routes** return before their first frame is sent: wait for the operation
  (`api.wait_op`, the `*_checked` helpers) before asserting its outcome or reading the
  counters it should move.
- **The diagnostic path is not a transaction.** Separate `/api/v1/dali/*` requests can
  be split by the poller, HCL or another master overwriting a DTR, and a send-twice pair
  is one request with `repeat_count: 2`, never two; a raw DTR-armed write is proved by
  reading its effect back.
- **Skips and xfails.** A feature is skipped as absent only on the firmware's own
  evidence (a `404`, a block missing from `/api/v1/diagnostics`); a probe that fails on
  a build that has the feature fails the test. A known-failure hatch is a conditional
  `@pytest.mark.xfail(strict=True)`, never an imperative `pytest.xfail()`, which never
  reports XPASS and so hides a fixed defect and its regression alike.
- **Autouse fixtures** in `tests/conftest.py` never take `api` (or anything else that
  skips on an unreachable controller) as a parameter, because an autouse skip skips every
  collected test, the hardware-free ones included: they build their own client, degrade
  to a printed warning and record an instrument failure for the per-test fixture to fail
  or skip on. Nor do they take any other bench fixture, `hil_config` included: the
  hardware-free verdict below reads every test's fixture closure.
- **Hardware-free tests** request no bench fixture (`BENCH_FIXTURES` in `hil/tiers.py`);
  a `*_unit.py` test that requests one is a collection error. A session made only of
  them skips `production_state`, `bench_baseline`, `dut_continuity`, the serial-bridge
  probe, the session baselines and the validity report, and writes no `summary.md`.
  They build `HilConfig` with `serial_remote=""` and their own `base`: the defaults name
  a reference bench's controller and Wiren Board, and `serialmon.start` or `hil flash`
  would ssh there and may (re)start the bridge, which resets the controller.
- **Measuring threads** own their own `Client`; a shared one shares a `requests.Session`
  and the retry ledger. A sampler that an exception kills fails nothing by itself, so the
  test checks it.
- **Serial evidence**: a script that reads the serial log takes its path from
  `serialmon.log_path(cfg)` and treats a missing or non-growing file as no evidence,
  never as a clean window.
- **The sniffer decoder** names application-extended opcodes as DT8 commands whatever
  prelude preceded them: match another device type's extended command by its bytes and
  its `ENABLE DEVICE TYPE` frame.
- **Mirrored constants**: `RULES_SOURCE_LIMIT_BYTES` in `tests/conftest.py` mirrors
  `MAX_RULES_SOURCE_BYTES` in `dali2rust-contracts`, and `hil corpus`'s colour-value width
  and defined sets mirror `colour_value_is_wide` / `colour_value_is_defined` in the
  domain crate; change them together.

## Validate the instrument before believing a measurement

What each oracle proves, and where it stops, is [STRATEGY.md](STRATEGY.md) §3. At the
rig, measure an effect through a test's own fixtures — baseline once, then drive, then
measure — or run that one test alone.
