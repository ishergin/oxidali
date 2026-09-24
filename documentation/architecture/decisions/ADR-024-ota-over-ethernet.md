# ADR-024: Firmware updates over Ethernet

Status: Accepted, amended by ADR-025
Date: 2026-09-08

## Context

Flashing over a UART needs physical access to the board, and on the bench it depends on a
serial bridge hosted by another machine. A wired flash also has no rollback: a bad image
leaves a board that nobody can reach until someone arrives with a cable. Speed is not the
argument — on this chip most of a flash is erase and write time, which Ethernet cannot
shorten. What an update over the network buys is a controller nobody can reach with a
cable, independence from the bench transport, and a rollback the wired path cannot have
at any speed.

## Decision

### Layout: grow around `storage`, keep everything executable below 16 MB

The table and the rule for each partition are in [10](../10-build-release-and-tooling.md)
§Partitions. The decision:

- **Both app slots and `otadata` live below 16 MB**, because neither the bootloader nor
  an update started from a slot works with them above that line; data above it is fine.
- **`storage` does not move**, because moving it loses every persisted slice. The slots
  are sized around it; 6 MB leaves the current image room to grow, and the failure when
  it stops fitting is loud.

### The device pulls the image

`POST /api/v1/firmware/updates {"url": …}` answers `202` + operation id; the controller
fetches the image in its own task (`esp_http_client`), writes it to the inactive slot,
selects it and reboots. `GET /api/v1/firmware` reports the running slot, the rollback
state and progress. Accepting the image as a POST body would hold the single httpd task
for the whole download and write, during which no other request is served; pulling also
gives a progress figure, and the transfer does not depend on the operator's connection
staying up.

- **The running slot is never the target.** `esp_ota_begin` erases before it writes, so
  `begin` compares the partition ESP-IDF names as next against the running one and refuses
  with `no_ota_slot` if they match. One comparison against a fault that costs a site
  visit.
- **The update path costs 3 KiB of stack while idle.** The subscriber thread only receives
  (`OTA_LISTENER_STACK`); the HTTP client, the TLS handshake and `esp_ota_write` run on a
  thread started for the update (`OTA_UPDATE_STACK`, 16 KiB) and released when it ends.
  Internal SRAM is the scarce resource
  ([07-memory-and-cores.md](../07-memory-and-cores.md)): an update path that permanently
  held a worker-sized stack would leave the httpd task unable to start, and a controller
  that cannot serve HTTP cannot be updated either. The spawn is fallible and an update
  that cannot get its stack fails instead of aborting.
- **An image proves itself before it is kept.** With bootloader rollback enabled, a new
  image marks itself valid only after it has composed and kept its Ethernet link up for a
  sustained streak of probes; a boot loop cannot satisfy that. An image that does not
  prove itself within three minutes is left pending, and the next reset returns to the
  previous slot.
- **Flash access follows the gate of [ADR-025](ADR-025-phy-interrupt-above-critical-sections.md)**
  ([08](../08-dali-phy-and-transport.md) §Flash does not stall the wire).
- **Background DALI work stands down during an update.** The poller and the HCL scheduler
  read a `MaintenanceHold` and pause, because the reboot at the end discards whatever they
  would learn or set (runtime state is not persisted). Operator commands are not held:
  the operator started the update, and refusing their input silently would be worse.

### Trust and transport

- **Unauthenticated and unsigned, by the maintainer's decision.** The API has no
  authentication, and OTA inherits the same trust model: whoever can open a TCP connection
  to the controller is trusted with it. OTA differs in consequence, not exposure. ESP-IDF's
  image validation (structure and the appended SHA-256) catches a truncated or corrupted
  download, and rollback undoes an image that fails to run; neither catches a hostile
  image. If an authentication posture arrives, OTA is an ordinary route and inherits it.
- **The image URL is a request parameter**, so the same implementation serves a laptop on
  the bench and a deployment's own server, and "where the image lives" is never a firmware
  change. Plain HTTP and HTTPS are both supported. HTTPS adds no library (mbedTLS and the
  CA bundle are already in the image) but it does add TLS heap for the duration of the
  download. Certificate dates are not checked (`CONFIG_MBEDTLS_HAVE_TIME_DATE` is off),
  which lets an update run in the first seconds after boot, before the clock is set, and
  equally accepts an expired certificate.
- **Who initiates stays manual.** A controller that polls for new images on its own is a
  separate product decision about unattended reboots and trusting a server.

### The wire stays, and stays required

OTA is additive. Only a wired flash can change the partition table, recover a board whose
both slots are bad, or provision a new board. `hil flash` remains the bench's flashing
path and the one that carries the bookkeeping (knob pinning, the ISR-IRAM gate, the run
manifest); a bench-side OTA transport, if added, is a transport choice inside it, not a
second mechanism.

### Rejected alternatives

- **POST the image to the HTTP server** — holds the httpd task for the whole update.
- **A faster UART** — the link is a small share of the cost; erase and write dominate.
- **Differential updates** — attack the erase cost but make rollback a much harder
  property to hold.
- **`ota_1` before `storage`** — moves `storage`, costing every persisted slice.

## Consequences

- Update state is product surface: progress, outcome, the running slot, and whether the
  running image has been marked valid.
- The web UI ships inside the image, so an update updates the UI too.
- The region above `storage` (about 16 MB) is free for data.
