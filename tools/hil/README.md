# HIL toolkit

pytest-based hardware-in-the-loop tests of the firmware on a real controller and a real
DALI line. The oracles are the controller's HTTP/WebSocket API and serial console, a
second DALI master as an independent bus sniffer and foreign master (for example a Wiren
Board WB-MDALI3 under `wb-mqtt-dali` on `python3-dali`, source in the board's
`/usr/lib/python3/dist-packages/dali/`), and, when one is attached, a USB camera.

Scope: what a bench needs, setting up, running and reading the suite. Strategy, the
coverage map and the watchlist: [STRATEGY.md](STRATEGY.md) (Russian); build knobs and the
version string: [10](../../documentation/architecture/10-build-release-and-tooling.md).

## What a bench needs

| Instrument | Enables | Without it |
| --- | --- | --- |
| A controller running this firmware, reachable over HTTP (`HIL_BASE`) | every tier | nothing runs |
| Its serial console, on this host's USB or behind a serial bridge on another host | `serial`, reboot evidence, `hil monitor`, `hil flash` | `hil preflight` reports it; `serial` tests skip |
| A DALI line with bus power and at least one lamp the suite may drive | every test that changes the light | `HIL_LAMPS_READ_ONLY=1` keeps the light untouched |
| A UVC camera on a fixed mount with the lamps in view | `optical` ([Optical tests](#optical-tests)) | `--no-camera` turns optical tests into skips |
| A second DALI master reachable over ssh and MQTT (WB-MDALI3 on a Wiren Board) | `sniffer`, `foreign`, and `ha_bridge` through its broker | the `sniffer` and `foreign` fixtures skip; deselect `ha_bridge` |
| A second controller on the same line (`HIL_PEER_BASE`) | `redundancy` | those tests are not collected |

macOS is the tested host: camera discovery uses ffmpeg's AVFoundation input, `setup.sh`
builds `uvc-util` with the macOS frameworks, the frame server runs in Terminal. The rest is
Python 3.9+ with the packages of `pyproject.toml` (`setup.sh` installs them) and espflash,
which makes `hil flash`'s images.

## Configuration

Variables are read from the environment when a command starts. `bench.env` (from
`bench.env.example`) holds only the firmware knobs `hil flash` enforces. **The defaults name one particular bench.** Set at
least `HIL_BASE`, the serial variables and the three short-address sets before the first
run: every pytest session that collects a bench test reaches `HIL_BASE`, and
`bench_baseline` suspends that controller's poller for the session; a run that drives
lamps also suspends its HCL schedules (so does a virtual-gear run) and sets its time zone
to `HIL_BENCH_TZ`, a run that drives none leaves both alone. A session whose every test is
[hardware-free](#writing-a-scenario) touches no network and no serial port.

| Variable | Meaning | Default |
| --- | --- | --- |
| `HIL_BASE` | the controller's URL | a reference bench's controller |
| `HIL_SERIAL_REMOTE` | `user@host:/dev/…` of the controller's UART behind a [serial bridge](#serial-ports); empty means this host's USB | a reference bench's Wiren Board |
| `HIL_SERIAL_PORT` | the local serial device when `HIL_SERIAL_REMOTE` is empty | found by scanning |
| `HIL_SERIAL_BAUD`, `HIL_FLASH_BAUD`, `HIL_SERIAL_BRIDGE_PORT` | console baud, esptool baud, the bridge's local port | `115200`, `1500000`, `4444` |
| `HIL_LAMP_SHORTS` | short addresses a test may drive or write (`0,2,3`, `0-3`); the client refuses the rest ([lamp guard](#the-installation-is-production)); empty allows none | empty: the reference bench has no lamp a test may drive |
| `HIL_GEAR_SHORTS` | gear the read tiers target; empty means every gear in the registry | empty |
| `HIL_OPTICAL_SHORTS` | lamps in the camera's view, calibrated and measured; always narrowed to `HIL_LAMP_SHORTS` | `HIL_LAMP_SHORTS` |
| `HIL_LAMPS_READ_ONLY` | on: report the light, never drive it, and the client refuses every visible action; `0` with lamps in `HIL_LAMP_SHORTS` is a run with the owner's go-ahead | on |
| `HIL_NO_CAMERA=1` (`--no-camera`) | optical tests skip instead of failing | off |
| `HIL_CAMERA_NAME`, `HIL_CAMERA_ID` | which camera to open | `USB Camera` |
| `HIL_SKIP_CALIBRATION=1` (`--skip-calibration`), `HIL_CALIBRATION_TTL_S` | reuse the saved calibration; how old it may be | recalibrate; `900` s |
| `HIL_LAMP_ROSTER=registry` | take the lamp roster from the registry's bound virtual lamps | the calibration |
| `HIL_WB_SSH`, `HIL_WB_DEVICE`, `HIL_WB_BUS` | the sniffer host over ssh, its DALI device and bus | a reference bench's Wiren Board |
| `HIL_MQTT_BROKER_HOST`, `HIL_MQTT_BROKER_PORT` | the broker the `ha_bridge` tier uses | a reference bench's Wiren Board, `1883` |
| `HIL_PEER_BASE`, `HIL_PEER_SERIAL_PORT`, `HIL_PEER_SERIAL_REMOTE`, `HIL_PEER_SERIAL_BRIDGE_PORT` | the pair's [second controller](#the-second-controller-hil---peer) | unset, unset, a reference bench's Wiren Board, `4446` |
| `HIL_ALLOW_DESTRUCTIVE=1`, `HIL_POWER_CUT=1`, `HIL_BUS_SHORT=1` | allow the destructive and the person-at-the-rig tests | off |
| `HIL_STATE_GUARD=0` | turn the [save and restore](#save-and-restore) guard off | on |
| `HIL_BENCH_TZ` | the time zone `bench_baseline` holds the controller in during a run that drives lamps | a reference bench's zone |
| `HIL_VIRTUAL_GEAR=1` | run the [virtual-gear](#virtual-gear) tier on the gear the peer emulates | off |
| `HIL_VIRTUAL_PARK` | the emulated park's shape: DT6, DT8 Tc and DT8 RGB+Tc gear, built in that order on the first free addresses above the reserve | `4,4,4` |
| `HIL_OWNER_SHORTS` | live lamps a scan cannot see (unpowered), added to the virtual-gear reserve | empty |
| `HIL_ALLOW_VIRTUAL_COMMISSIONING=1` | let the virtual-gear tier commission emulated gear (a go-ahead per run) | off |
| `HIL_ALLOW_RULE_COMMITS=1` | select the tests that commit the rules document (`rules_guard`; a go-ahead per run, STRATEGY §4) | off |
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

Drop `--no-camera` and the markers of instruments the bench has. The toolkit's unit tests
need no hardware; `HIL_BASE` on a closed port keeps a regressed hook off any bench:

```bash
HIL_BASE=http://127.0.0.1:9 HIL_SERIAL_REMOTE= HIL_NO_CAMERA=1 \
  .venv/bin/python3 -m pytest tests/*_unit.py                                           # tools/hil
```

## The installation is production

The suite runs on production lighting ([STRATEGY.md](STRATEGY.md) §2) under the standing
rules of STRATEGY §4. `HIL_LAMP_SHORTS` names the lamps a test may drive; set
`HIL_GEAR_SHORTS` on every run — empty, the default, means every gear in the registry.

The lamp guard (`hil/lamp_guard.py`) enforces `HIL_LAMP_SHORTS` and
`HIL_LAMPS_READ_ONLY`, whatever a test or a script selects. Every request
`Client` sends and every frame of the WB foreign master passes it first, and a refusal
raises `LampNotAllowed`, naming its cause, before anything reaches the wire; during a
bench test's setup or run it is reported as a skip that quotes it:

- A target-state, identify or attribute write, or a diagnostic frame that writes the gear,
  must address a short in `HIL_LAMP_SHORTS`; queries always pass. An extended command is
  read by the last `ENABLE DEVICE TYPE` the client sent (`dali/raw`, the WB master; with
  none, an unknown write) or the one the firmware sends itself (`dali/command`). A virtual
  lamp whose binding cannot be read, or an identify naming no short, is refused.
- A group or broadcast frame, a group target-state, a scene recall, a policy apply and a
  scan while apply-on-discovery is armed pass only when every present gear on the segment
  is in `HIL_LAMP_SHORTS`.
- Under `HIL_LAMPS_READ_ONLY=1` every visible action is refused: target-state, identify,
  scene recall, and every frame or attribute write that changes what a lit lamp shows
  (`VISIBLE_*` in `hil/lamp_guard.py`; a new level or Tc limit moves a lit lamp into
  range). Other configuration writes to an allowed lamp still pass (STRATEGY §4). No
  restart or handover passes either — a reboot (`Client.expect_reboot`), firmware update,
  switchover, `application_active` or `settings/redundancy` write: the unit active
  afterwards republishes its HCL point over a lamp set by hand. A destructive run needs
  `HIL_LAMPS_READ_ONLY=0`.
- A `/api/v1/dali/*` body with no frame the guard can read is refused, and so is a write
  whose path the client or the firmware would respell: `%` (bar an encoded rule name),
  `#`, `+`, an empty or dot segment. A control-device forget and a slice import always
  fail; a device forget writes its short and, like a virtual-lamp delete, never passes
  read-only outside the [virtual-gear](#virtual-gear) teardown.
- Commissioning is refused — the steps, address changes, replacements, a
  `commission_unaddressed` run, the input-device commission and every special frame but
  TERMINATE, DTR0–2, PING and `ENABLE DEVICE TYPE` — except by the virtual tier's fence
  with `HIL_ALLOW_VIRTUAL_COMMISSIONING=1`.
- A group or scene apply passes only when every virtual lamp whose row it writes is bound
  to a lamp of `HIL_LAMP_SHORTS`, and never under read-only.
- A 24-bit frame from the WB master is refused under read-only, since a forged input event
  reaches automations the toolkit cannot see, when an
  enabled owner rule could fire on its event, and when it is a command.
- HCL schedules, rules and MQTT commands drive lamps past the guard, so a test using them
  asks it first (`drive_allowed`, `allowed_bound_lamp`) for each lamp reached — the whole
  segment for a broadcast, the HA scene select or any group but `free_group` (no registered
  gear or owner rule uses it, no group query answers, only the test's lamps join).
- Every `light` test first skips when any rule of the document, enabled or not, names its
  lamps, their virtual lamps or their groups. `clock_guard` moves the controller's clock
  only when a test calls it, never at setup, and each call skips the test under read-only
  or while an owner rule fires at a time of day or at the sun. With the go-ahead too, a
  restart or handover is refused while an owner rule fires when a controller starts or
  becomes active.

A run with no `HIL_*` flag writes only this, each put back by its test or guard: the
adapter's name; group 15's and a scene's name and Home Assistant exposure (HIL-GRP-03,
HIL-SCN-03); the Home Assistant bridge's settings, moved to `hiltest` (the owner's
entities are unavailable meanwhile), and a virtual lamp's exposure (`ha_guard`); the poller's settings (`test_poller`, `poller_guard`); and the owner's
poller and armed apply-on-discovery, off for the session (`bench_baseline`). HIL-INP-02 and
03 rewrite the panel's instance group membership with the value it holds; scans refresh the
registry.

## Save and restore

The session-scoped autouse fixture `production_state` (`hil/prod_state.py`) runs
for every session that reaches the controller:

- **Before** anything else it primes the registry with an attribute read of every
  gear (runtime state is unknown after a reboot) and snapshots the controller's and
  gear's configuration (the fields it names, below) and what every lamp shows.
- **During** the session every request `Client` sends and every frame of the WB master
  is noted, by resource and patched field, in a write log next to the snapshot; an MQTT
  command, a forged input event, a rule run, a schedule, clock or role change and a
  reboot count as moving any lamp.
- **After** it each layer writes back only what that log names (a refused one is logged
  and the next goes on); it re-reads every gear
  before putting the light back and diffs against the snapshot. What the toolkit wrote and
  could not put back prints `PRODUCTION STATE NOT RESTORED`, turns the run red and keeps
  the session open; a change someone else made is printed as such and left.
- It cannot restore the colour a DT8 gear stored with a scene, a last active level, or
  anything outside the controller (the Wiren Board, Home Assistant).
- It restores the rules document only where the difference is whole blocks of rules
  named `hil-…`, against the revision it read, so an owner's edit stays and is reported;
  while the owner's delayed actions are pending (STRATEGY §4.9) it leaves the test rules in
  place, switched off, and says so. A commit resets toggles to the text, so it puts back
  those the commit changed; any other toggle that differs from the snapshot is reported —
  a residual when the session's log names it, otherwise someone else's change.
- Each session keeps `state/production_state-<time>.json` and its `.writes.json`
  (`production_state_last.json`: the latest snapshot). `hil state restore` finishes a
  left virtual-gear session ([Virtual gear](#virtual-gear)), then restores every open
  session, newest first, through its own log; without a log it only reports, and `--all`
  writes the whole snapshot back — under read-only never a schedule, override or zone. A
  session closes only when no newer open session's log names what it wrote, and a FILE is
  refused while a newer session is open. An unreadable session file or log, or one of
  another controller, is named and its session left open, and the command exits non-zero
  while a session is open. Every later session ends red until then.
- Under `HIL_LAMPS_READ_ONLY=1` the light is reported, never driven — by this
  fixture, `state_snapshot` and `hil state restore`. A lamp outside
  `HIL_LAMP_SHORTS` is never driven back, and the lamp guard refuses a repair of its
  gear groups and scenes: a difference there stays a residual. So does the light of a lamp
  the log names, keeping the session open until a restore with `HIL_LAMPS_READ_ONLY=0` and
  it in `HIL_LAMP_SHORTS`.
- Across several tiers `hil state diff` the first session's file at the end.
  `HIL_STATE_GUARD=0` is the only opt-out.
- It compares and restores only the fields it names (the poller's and Home Assistant's
  `RESTORABLE` tuples, `prod_state`'s settings, gear-configuration and device-record
  lists), and so do the settings guards: a writable field the firmware adds is named
  there too, or a test that changes it leaks past the session with no residual.
- Fields an enabled HCL schedule drives (level, colour or both, on its targets'
  members) are neither compared nor restored: the schedule moves them during a session,
  and writing one back is an API write the scheduler takes as an override.
- Part 103 control devices are neither snapshotted nor restored — their configuration
  lives in the panel's NVM — so a test writes a panel field only with the value the
  panel already holds.
- A request that can move a lamp through `/api/v1/dali/*` goes through `Client._http`
  (never `raw_response` or a bare session), which books the shorts it touches so
  `restore_states` re-reads them before comparing; `raw_response` passes the lamp
  guard but books nothing.
- A session fixture that changes the installation takes `production_state` as a
  parameter, used or not: only that dependency orders it after the snapshot.

Per-test guards (`state_snapshot`, `*_matrix_guard`, `hcl_guard`, …) restore what one test
changed; `bench_baseline` fails the session on a device still named `hil-…`.

## Layout

| Path | Holds |
| --- | --- |
| `hil/` | the library and the `hil` CLI |
| `tests/` | the suites; `conftest.py` lists the `hil_*.py` plugins that hold the fixtures and the safety gates, one per domain (session hooks, instruments, session and per-test guards, optics, run validity, the virtual-gear session) |
| `wb/serial_bridge.py` | the serial bridge, copied to the Wiren Board's `/mnt/data/dali2rust/`, where `hil flash --via wb\|ota` also stages images and `--via wb` its esptool |
| `corpus/` | frozen captures of the installation (`hil corpus`); local, and only the files host tests pin are tracked (`.gitignore`) |
| `*_budget.txt` | run-validity budgets (below) |
| `state/`, `runs/<ts>/` | gitignored bench state (calibration, monitor, serial log, pins, snapshots) and per-run artifacts; `runs/current` is the latest |

`state/` belongs to the checkout: a monitor started from another worktree is unseen here,
and a flash from here then fights it for the port.

## Optical tests

The camera is the oracle for what a lamp really shows.
The `optical` tests (`test_optical_*`, and the optical cases in `test_scenes`,
`test_scenarios`, `test_hcl_runtime` and `test_ha_bridge`) check on the fixtures on and
off, a strict brightness ladder, RGB hue, colour-only lighting, CCT order, and a group
recall, scene, HCL level and MQTT colour reaching them.

The camera is a UVC camera on a fixed mount with the lamps or their patches in view. `setup.sh` builds `uvc-util`, which locks exposure and white balance. macOS
denies an agent the camera, so frames come from a frame server (`hil camera-server
--spawn-terminal`, approve it in the Terminal). Exactly one may run, and
`--spawn-terminal` leaves a live one as it is. A server that is alive but serves no
frames holds a camera AVFoundation no longer hands it: run `vendor/uvc-util/uvc-util
-d`, which enumerates the device again, then `hil camera-server --restart`.

Calibration (`hil calibrate`, or before the first test that needs optics)
locks the camera, takes all-off baselines for the noise floor, finds each lamp's mask,
tunes the exposure against clipping, measures crosstalk and a brightness ladder, records
colour fingerprints and picks fiducials that reveal a camera that moved. The profile is
saved in `state/`; a session calibrates anew, and again before a test once the profile is
older than `HIL_CALIBRATION_TTL_S`, unless `--skip-calibration` reuses the saved one.
**Calibration drives every lamp in `HIL_OPTICAL_SHORTS`**, so it is a visible action.

A dead optical channel **fails** optical tests; `--no-camera` / `HIL_NO_CAMERA=1` turns
that into skips.

## Running tests

Always from `tools/hil/` (`testpaths` and `addopts` are relative):
`.venv/bin/python3 -m pytest …`. `just hil-preflight | hil-smoke | hil-default |
hil-slow` wrap the cwd.

- The default run excludes `destructive` and `slow`. An explicit `-m` **replaces**
  that filter (pytest keeps the last `-m`), so the destructive tier is also gated by
  `HIL_ALLOW_DESTRUCTIVE=1`: without it those tests skip whatever `-m` says, and with it
  a session selecting two is a usage error. Every
  test that reboots or halts a controller is `destructive`: collection refuses one
  that requests `dut_reboot` or calls `remote_serial.control` (itself or through a
  helper in its module) without the marker.
- The tiers, their order and what each is for: STRATEGY §5. Add
  `--junitxml=runs/current/junit.xml --html=runs/current/report.html` for reports.
- `hil preflight` judges the serial channel by the log growing after a health request
  and the smoke tier by the firmware's `HTTP access: GET /api/v1/health …` line, so that
  line is part of the firmware's contract with the toolkit.
- Markers: `smoke`, `optical`, `sniffer`, `serial`, `foreign` (the WB master),
  `ha_bridge`, `redundancy`, `slow`, `destructive`, `needs_capability(name)`,
  `virtual_gear` (the gear the peer emulates), `light` (can change what a lamp shows).
- Whatever `-m` says, collection deselects a `light` test unless `HIL_LAMP_SHORTS` names
  a lamp and `HIL_LAMPS_READ_ONLY=0`, and a test that commits the rules document
  (it requests `rules_guard`) unless `HIL_ALLOW_RULE_COMMITS=1`.
- `--fast-fade` sets the fade time of every lamp in `HIL_LAMP_SHORTS` to 0 after
  `production_state` has taken its snapshot, and `production_state` restores it at
  the end of the session; with `HIL_STATE_GUARD=0` the option is a usage error.
- Person-at-the-rig tests skip unless asked for: `HIL_POWER_CUT=1` (cut the
  controller's power mid-write), `HIL_BUS_SHORT=1` (short the bus; every lamp sees it).
- The `ha_bridge` tier uses the WB's mosquitto under `hiltest*` prefixes; `ha_guard`
  restores the live settings and sweeps the test topics, never touching
  `broker_password`. Acceptance against the live Home Assistant is manual.
- Artifacts land in `runs/<ts>/<test-id>/`.

## Run validity — how a red run is classified

Every run ends with an `HIL validity` section (also in `runs/<ts>/summary.md`): frame
servers (more than one is flagged; `hil preflight` fails on it), DUT uptime continuity
(a reboot fails the test it
landed in, unless that test uses `dut_reboot`), the baseline and the gear-segment
restriction (a restricted run is *not an acceptance run*), the peer and its uptime
from session start to end (a reboot, or a peer that stops answering, fails the run),
the production-state result, WB availability, the retry ledger, the controller's bus
drop counters, per-task stack headroom (a run whose log has no census says so; a
budget line no census task matches fails the run unless the task is spawned on
demand; the trend compares runs of one image, named by the version `/api/v1/health`
reports, so an image written over the network starts a new one), the boot heap
ladder and the runtime heap floor. **Classify a red run from this section, not from the serial log.**

| File | Bounds | Moves |
| --- | --- | --- |
| `retry_budget.txt` | what a session may absorb | down only |
| `stack_budget.txt` | how deep each firmware task may go | down only |
| `boot_heap_budget.txt` | internal SRAM left at each boot stage | up only (a floor) |
| `runtime_heap_budget.txt` | internal SRAM minimum since boot, read from `/api/v1/stats` at session end | up only (a floor) |

A budget moves against its direction only with the justifying measurement in the
commit. `-1` means "never measured with a counter attached": reported, not gated — replace
it with the first instrumented figure. A `watched` row counts what a workaround changed — a
re-read that disagreed with the first answer, a repair spent after a wrong one — and has
no budget line: it never gates, and the workaround goes once it stays at zero. A retry is
counted, never silent; a dead instrument (blind colour sample) charges no budget.
A red test keeps its strict assert
([05](../../documentation/architecture/05-testing-and-bdd.md)); the budget carries the
rate the installation imposes.

## CLI (`.venv/bin/hil`)

| Command | Purpose |
| --- | --- |
| `preflight` | read-only readiness: DUT HTTP, serial bridge, monitor, WB ssh, broker, camera, calibration, host tools |
| `monitor start\|stop\|status\|tail` | persistent serial monitor; `start` raises the bridge if needed |
| `remote start [--restart]\|stop\|status\|ping\|bootloader\|run\|release\|reacquire` | the WB serial bridge and its tunnel |
| `flash [--build-only] [--allow-nonbench-build] [--allow-red-isr] [--allow-stale-ui] [--via rfc2217\|wb\|ota]` | the only flash path (below) |
| `--peer role gear-sim [--via ota\|wb]\|controller [--via ota\|wb]\|status` | lend the peer to the gear emulator and take it back (below) |
| `state save\|restore [--all\|--retire FILE]\|diff [FILE]` | the installation snapshot; `--retire FILE` closes a session that never closes once every older open one's logged fields hold their snapshot; never delete an older session file by hand while a newer one is open |
| `api <sub> …` | manual API calls; exit 2 means the firmware lacks the capability |
| `corpus [parts…]` | freeze both boards' configuration, REST, wire replies and serial evidence into `corpus/`; slices in `SECRET_SLICES` (the Home Assistant slice carries the broker password) are withheld unless `--keep-secrets` writes into an `--out` outside every git work tree — a slice that comes to carry a secret joins that list, because a pinned file gets committed |
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
`HIL_SERIAL_REMOTE` and `HIL_PEER_SERIAL_REMOTE` name them. `wb/serial_bridge.py`
(pyserial only) holds each UART open for life behind two loopback listeners: data, in
RFC2217 so esptool's baud change reaches the UART, and one port above it, control: `ping`, `status`, `bootloader`, `run`, and for a write on the WB itself `release`,
`reacquire` and `write <line>` (a line to the board's console); `status` lists the last
three. An ssh tunnel brings them here: the
primary is `rfc2217://127.0.0.1:4444`, the peer `…:4446`. The limitations of the USB
bridge, pyserial and esptool this set-up works around are
[ISSUE-148](../../documentation/product-design/known-issues.md).

- The bridge serves **one client** and refuses a second rather than evicting the
  first.
- Attaching to a live bridge resets nothing. **Starting a bridge that is down resets
  the controller.** `hil remote start`, `hil monitor
  start` and `hil flash` all start it when needed (`--restart` always does); first
  check on the WB whether it still listens (`ss -ltn`) — if only the tunnel died,
  the start reuses it. A changed `wb/serial_bridge.py` is copied to the WB but a live
  bridge keeps running the old one until `--restart`, which resets the controller.
- A bridge whose port is gone (node vanished, board enumerated again, a read failed)
  answers every control command `err port-gone: …`. `hil remote`, `hil monitor start`, `hil flash` and
  `hil preflight` fail on it instead of reusing a bridge that carries no data;
  `--restart` reopens the port and so resets the controller, which is the operator's
  decision.
- Reset sequences run on the WB.
- Logs: `state/persist/serial.log` (peer: `state/peer/persist/`), lines stamped in UTC
  (`…Z`) like `runs/<ts>`.

## Flashing

`hil flash` is the only way to flash the installation (`cargo flash` and `just
p4-fw-flash` open a local port). It refuses a build without the
`bench.env` knobs and a stale embedded UI bundle (`--allow-stale-ui` builds with the
previous one), runs the ISR-IRAM gate on the linked binary (refusing exit 1, red, and
an unchecked 2, a tool — sought in `.embuild/espressif/tools`, then `~/.espressif` — or the
ELF missing, or 3, no entry point or section found; an entry outside IRAM is red),
records `runs/<ts>/manifest.json` (commit, binary sha, knobs, checks), resets the
chip into its ROM loader through the bridge, writes with esptool at
`HIL_FLASH_BAUD`, always asks for `run` afterwards (a board left in the loader looks
bricked from the LAN), waits for `/api/v1/health` and requires the reported version
to equal the one inside the image it wrote — a prefix-of-`HEAD` check only when the
image yields none — and records that version in the manifest. `MCU` and the sdkconfig
baseline are read from `.cargo/config.toml` and asserted, never set. Off the WB's LAN
esptool over the bridge never syncs: use `--via wb|ota`.

`bench.env` overrides the shell's exports, so a one-off `KNOB=… hil flash` applies only
when `bench.env` does not set it.

**Delivery** — `hil flash --via rfc2217|wb|ota` picks it; what only the wire can do is in
[ADR-024](../../documentation/architecture/decisions/ADR-024-ota-over-ethernet.md).

- `wb` stages the merged image on the Wiren Board, checks its sha256 and writes it with a
  vendored esptool on the board's own serial device at 921 600 baud, detached from the ssh
  session: its exit code comes back through a file, so the host's link to it does not
  decide whether the write lands. The bridge releases the port for the write and
  reacquires it after, which is the one reset; a bridge that cannot release is refused
  before any reset (`hil remote start --restart` runs the current one, and resets the board).
- Before a `wb` write, the board the port names must fall quiet in its ROM loader while the
  other board keeps its uptime. With neither witness the write is refused, and any failure
  before esptool starts sends `run`.
- `ota` builds the application image instead of the merged one, serves it from the Wiren
  Board, posts it to `/api/v1/firmware/updates` and waits for `ready_to_reboot`, with the
  gates, manifest and version check above. It does not wait for `pending_verify` to clear,
  and until then a reset boots the previous slot
  ([contract](../../documentation/product-design/rest-api/resources/firmware.md)).
- `rfc2217` (default): esptool over the bridge above, or espflash on this host's USB.

The gear emulator's image goes only through `hil --peer role gear-sim` (below).

## The second controller (`hil --peer`)

`hil --peer <command>` points the toolkit at the pair's other board: its URL
(`HIL_PEER_BASE`, or the lease its firmware announces on serial, learned by the first
`hil --peer flash` into `state/peer/base.pin`), its bridge, its own `state/peer/` and
`runs/peer/`. The
`redundancy` tier is collected only when `HIL_PEER_BASE` names the peer; preflight, the
run-validity peer check, the hand-back below, the gear emulator, `hil corpus` and the
`--via wb` witness also find it through the pin. A reboot the suite provokes leaves the
peer active and the rebooted board standby; `dut_reboot` and the OTA test hand the role
back (`hil/pair.py`) before the next assertion.

`hil --peer role gear-sim [--via ota|wb] | controller | status` lends the peer to the gear
emulator and takes it back ([ADR-031](../../documentation/architecture/decisions/ADR-031-gear-emulator-on-the-standby.md)):

- **`--via ota`** (the default) installs the emulator in the peer's inactive slot for one
  boot. The ready line must say `pending_verify`, and any reset hands the controller back.
- **`--via wb`** writes it by wire, and the role survives resets.

The peer must be the standby, its monitor must run before the switch (starting a bridge that
is down resets the board), no other flasher may run here or on the WB, and no virtual-gear
ledger may be open. The emulator's image is built with [`tools/dali-gear-sim`](../dali-gear-sim/README.md),
merged with the controller's bootloader, and checked by its ready line; the console prints it
again on `ready`, so a monitor that missed the boot still reads it.

`state/peer/role.pin` records the role once the image is delivered (`confirmed: false`
until the ready line checks out). `role controller` refuses while a ledger is open or the
peer's bridge is the DUT's, and asks the board itself:

- a peer that answers as a controller is only recorded;
- otherwise the emulator is asked for its ready line through the peer's bridge, which proves
  the board that bridge resets. `pending_verify` resets it, while the DUT's uptime witnesses
  that the reset did not reach the DUT; anything else writes the controller back by wire,
  building with the committed UI bundle even when it is stale;
- a peer that answers neither needs `--via`, which also overrides the ready line.

Then, once the DUT's slices match, a standby whose registry still holds entries they
dropped is reset once, witnessed the same way. Preflight, the virtual-gear session and the
run-validity report read the pin.

## Virtual gear

`HIL_VIRTUAL_GEAR=1 .venv/bin/python3 -m pytest --no-camera` runs the tests marked
`virtual_gear` — and only those — on the gear the peer emulates, on the installation's wire.
Its rules are STRATEGY §4.8; the mechanics:

1. **Before any config loads**, the plugin (`tests/hil_virtual.py`) reads the reserve and
   sets `HIL_LAMP_SHORTS`, `HIL_GEAR_SHORTS` and `HIL_OPTICAL_SHORTS` to the park, so every
   selector follows it.
2. **After the owner's installation is snapshotted** (without the snapshot the session
   refuses), `hil/virtual_gear.py` opens the session:
   - proves the free groups silent against a positive control;
   - checks that no owner rule, compiled or in text, names a free group, a session lamp
     (by id or name) or a park device;
   - gives the emulator the reserve, checks the mask it echoes, builds the fleet and enables
     it on a ladder once the first gear has answered the controller cleanly and the
     emulator's answer counters moved without a late or expired answer;
   - hides the free groups from Home Assistant;
   - scans, reads each park gear's identity bank (a GTIN not the emulator's refuses the
     session), and binds the park to new virtual lamps with HA off, writing each lamp's id
     to the ledger before creating it.
3. **During the session**, the fence on the tests' client (`lamp_guard.VirtualFence`) passes
   only the routes and frames STRATEGY §4.8 lists. After every test the tripwire waits for
   the DUT's log to fall quiet, sends the barrier and judges the test's `DALI PHY TX` lines
   with the same fence.
4. **The teardown follows** `state/virtual_gear.json`, step by step:
   - silence the fleet;
   - delete the session's lamps still named for their park address and forget the park —
     the guard passes these only here, a forget only above 15, outside the reserve, of a
     record with the emulator's GTIN;
   - restore group flags;
   - compare the registry, the WB list and the retained MQTT topics with the start.

   A teardown that leaves residue keeps the ledger: the next session and `hil --peer role
   controller` refuse until `hil state restore` finishes it. Check a park record it may not
   forget (no emulator GTIN) by hand; if emulated, `curl -X DELETE` its
   `/api/v1/adapters/0/physical-devices/<short>`, which ends the residue.

The session exits 3 when it cannot open, 4 on a `SAFETY` stop, 5 when the peer came back
as a controller, 6 when the tripwire went blind. The emulator
takes one reserve per boot: a later session on the same boot proceeds with the same reserve,
and another reserve needs the emulator restarted — under the OTA role that returns the
controller.

The oracle is the emulator's `C` lines (`hil/gearsim.py`: `GearOracle.expect`,
`untouched`) and, inside `GearOracle.hearing()`, the `F` lines of frames it heard, which
vouch for the wire only when every frame the DUT logged as sent there is among them
(`gearsim.unheard`); `HIL validity` prints a `VIRTUAL GEAR` block: answer counters, any
`SAFETY` stop, what the teardown left.

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
`pyproject.toml`; the suite runs with `--strict-markers`); grep the id to find it, and
`-m hil_id` selects them all. Put back anything the test writes through a guard, and keep
visible actions out of tests that do not need them — each costs a go-ahead.

- **Targets.** A test that changes what a lamp shows takes its target from the `lamps`
  fixture or `cfg.lamp_short_set()`; `optical_addrs()`, `present_optical_addrs()`,
  `lamp_addrs()` and `off_all()` stay inside `HIL_LAMP_SHORTS` too. `addrs()`,
  `present_addrs()` and `devices()` honour only `HIL_GEAR_SHORTS`: they pick read
  targets. Whatever the selector, the [lamp guard](#the-installation-is-production)
  refuses a drive outside `HIL_LAMP_SHORTS`, and a group or broadcast action unless
  the whole segment is allowed; a gear-wide command such as `REMOVE FROM SCENE` is
  sent per lamp.
- **Reboots.** A test or fixture that reboots a controller (reset, flash, OTA, power
  cut, bootloader request) does so inside `Client.expect_reboot()`: only there are the
  failed requests that follow booked as `http_reboot_race` and every client's pooled
  connections dropped; outside it they spend the gated `http_transport` budget.
- **`202` routes** return before their first frame: wait for the operation (`api.wait_op`,
  the `*_checked` helpers) before asserting its outcome or the counters it moves.
- **The diagnostic path is not a transaction.** Separate `/api/v1/dali/*` requests can
  be split by the poller, HCL or another master overwriting a DTR, and a send-twice pair
  is one request with `repeat_count: 2`, never two; a raw DTR-armed write is proved by
  reading its effect back.
- **Retries.** A step a test repeats goes through the ledger: `api.count_retry(kind,
  detail)` for a request or a read, the oracle's `count_retry(cause)` for a
  measurement; a sniffer `resend=` counts itself. A new kind gets a line of 0 in
  `retry_budget.txt`. A conditional write is never repeated blindly: after a lost
  answer `rules_replace` reads `/rules` back and repeats only a write that did not land.
- **Skips and xfails.** A feature is skipped as absent only on the firmware's evidence (a
  `404`, a block missing from `/api/v1/diagnostics`); a failing probe on a build that has it
  fails the test. A known-failure hatch is a conditional
  `@pytest.mark.xfail(strict=True)`, never an imperative `pytest.xfail()`, which never
  reports XPASS and so hides a fixed defect and its regression alike.
- **Autouse fixtures** in the `tests/hil_*.py` plugins never take `api` (or anything
  else that skips on an unreachable controller) as a parameter, because an autouse skip
  skips every collected test, the hardware-free ones included: they build their own
  client, degrade to a printed warning and record an instrument failure for the per-test
  fixture to fail or skip on (`optical_session` skips optical tests on an unreachable DUT). Nor do they take any other bench fixture, `hil_config`
  included: the hardware-free verdict below reads every test's fixture closure.
- **Hardware-free tests** request no bench fixture (`BENCH_FIXTURES` in `hil/tiers.py`);
  a `*_unit.py` test that requests one is a collection error, and a guard refusal or a
  skip no `skip` or `skipif` marker declares fails it. A session made only of
  them skips `production_state`, `bench_baseline`, `dut_continuity`, the bridge probe, the
  session baselines, the validity report and `summary.md`.
  They build `HilConfig` with `serial_remote=""` and their own `base`: with the defaults,
  `serialmon.start` or `hil flash` could (re)start a reference bench's bridge, resetting
  its controller.
- **Measuring threads** own their `Client` (a shared one shares a `requests.Session` and
  the retry ledger); a sampler an exception kills fails nothing, so the test checks it.
- **Serial evidence**: a script takes the serial log's path from `serialmon.log_path(cfg)`
  and reads a missing or non-growing file as no evidence, never a clean window.
- **The sniffer decoder** names every application-extended opcode as a DT8 command:
  match another device type's by its bytes and its `ENABLE DEVICE TYPE` frame.
- **Mirrored constants**: `RULES_SOURCE_LIMIT_BYTES` in `tests/hil_test_guards.py` mirrors
  `MAX_RULES_SOURCE_BYTES` in `dali2rust-contracts`, and `hil corpus`'s colour-value width
  and defined sets mirror `colour_value_is_wide` / `colour_value_is_defined` in the
  domain crate; `hil/seriallog.py`'s `LATE_REPORT_BYTES` and `HEARTBEAT_PERIOD_S`, and
  the report interval, client limit and bridge inbox depth in `test_ws.py` and
  `test_ha_bridge.py`, mirror the firmware's, and `EMULATED_GTIN_BASE` in
  `hil/lamp_guard.py` the gear model's `build_bank0`; change them together.

## Validate the instrument before believing a measurement

What each oracle proves, and where it stops: [STRATEGY.md](STRATEGY.md) §3. At the rig,
measure an effect through a test's own fixtures (baseline, drive, measure) or run that
test alone.
