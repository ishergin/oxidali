# ADR-034: The WebSocket socket belongs to a worker

Status: Accepted; amends ADR-002 and ADR-023
Date: 2026-11-07

## Context

One `httpd` task serves every socket, so every millisecond a handler holds it is downtime
for all clients ([02](../02-runtime-and-threading.md) §The HTTP task). A WebSocket frame
was written on that task: the sender thread handed a batch to `httpd_queue_work` and the
httpd task ran `httpd_ws_send_frame_async` for it. The coupling runs both ways.

Outward, the subscriber pays: while the task serves REST responses the queued send waits, and
a client's queue overflows. Everything cheap has been done — routed delivery
([ADR-015](ADR-015-routed-event-delivery.md)), coalescing by projection key
([ADR-023](ADR-023-named-subscribers-and-shared-coalescing.md)), a light device summary,
one work item per batch instead of one per frame — and the bench cannot tell the fixed
image from the unfixed one, because the event rate never fills the queue.

Inward, everyone else pays: one work item carries the whole batch, and each frame is two
blocking `send()` calls under `SO_SNDTIMEO`. A client reading slowly enough holds the task
that serves every socket for as long as its batch takes.

The way out looked like a second `esp_http_server` instance, and that was refused: it needs
its own `ctrl_port`, a second port against the Origin check, an indistinguishable task name
in the stack census, and sockets the system does not have — `max_open_sockets + 3` is
checked per instance, so with a pool of ten the one instance already asks for thirteen of
sixteen. All WebSocket clients would still have shared one task.

`httpd_req_async_handler_begin` is the mechanism ESP-IDF offers instead, and it was missed.
It raises `for_async_req`, and the server then stands down from that socket entirely: out
of the `select` set, skipped by the session loop, exempt from LRU purge. No second
instance, no extra descriptor.

## Decision

### The socket leaves the httpd task at the upgrade

The URI handler checks the Origin, calls `httpd_req_async_handler_begin`, and hands the
request copy to a worker. From that point the socket has one owner and the httpd task has
none. Nothing about a WebSocket runs on the httpd task afterwards — not the frame write,
not the frame read, not the subscription change, not the acknowledgement.

### Reading and writing are separate owners, and the frame is the unit

`LWIP_NETCONN_FULLDUPLEX` grants one reader, one writer and one closer on a socket, not two
writers. A frame is a header write followed by a payload write, so every outgoing frame —
an event batch, a Pong, a Close — goes through one path that holds a lock across the pair.
A reader runner per connection blocks on the socket; the sender runner it already had
drains the client queue.

### The frame codec is ours

`httpd_ws_recv_frame` cannot come along: it reads the second byte and takes type and FIN
from `httpd_req_aux`, so it assumes the private `httpd_ws_get_frame_type` consumed byte 0.
`httpd_req_recv` is no substitute, since it clamps to `remaining_len`, which an upgrade
leaves at 0. So the frame is parsed and built in `dali2rust-ws-runtime`, above a narrow
wire trait, which makes it host-testable for the first time: this path never was.

Fragmentation is refused rather than reassembled, which is a deliberate deviation from
RFC 6455 §5.4's "an endpoint MUST be capable of handling fragmented messages". Nothing here
joins fragments, so a non-final data frame or a Continuation frame is an error — accepting one
would hand the hub half a request and read the rest as a fresh header. The surface is a
512-byte command channel that no browser fragments; if one ever does, this is the record that
says why it is refused.

### Teardown is once, by the last owner

The reader and the sender each hold the connection. Whichever goes second returns the
socket: a full shutdown, then `httpd_req_async_handler_complete`. The sender wakes a
blocked reader with a read shutdown rather than completing under it, because a completed
request hands the descriptor back and lwIP may reuse its number. ESP-IDF is explicit that
an uncompleted async request eventually stops the server accepting connections, so a leak
here would cost REST and the web UI, not only WebSocket.

`complete` is called off the httpd task, which is what it is for, and the session is reaped
by the httpd task on its own path once the socket is shut. `httpd_sess_trigger_close` is
never called from a worker: it begins with an unlocked walk of the session table.

## Consequences

- Both coupled costs are gone by construction rather than mitigated: a REST response cannot
  delay a WebSocket frame, and a slow reader cannot delay a REST response, because they no
  longer share a task. The batch keeps its own bound.
- Leaving LRU purge also keeps load above the socket pool from closing a subscriber. It does
  not help ISSUE-55: a pool that must still purge now has only REST sessions to choose from,
  so exempting this socket makes the toolkit's keep-alive session a likelier victim, not a
  rarer one.
- The slot pool, the result slot and the deadline that existed only to cross onto the httpd
  task are gone, and with them a queued work item the control socket can silently drop.
- About 1.2 KB of internal SRAM per live connection for the request copy, held for its life,
  against the one-off ~3 KB of the refused second instance. The reader's own 8 KiB stack costs
  more than either: `StackHome::ExternalOnXip` resolves to internal SRAM whenever the XIP image
  is off or stacks are forced internal, so on such an image the five reader slots are 40 KiB of
  the scarce pool once a reconnect has used them all. The bench image is not such an image —
  it reports worker and httpd stacks in PSRAM — so there the slots cost the scarce pool
  nothing. Both numbers land on ISSUE-71.
- A reader runner per client plus one slot of reconnect headroom, named `ws-reader`, and the
  client-request parse moves off the 20 KiB httpd stack onto it. The parse is the reader's
  deepest frame and the reader's budget follows the census. The sender comes out shallower
  than the path it replaces and still above the gate ISSUE-208 holds for it, so that gate is
  the one this change does not settle.
- Leaving the LRU purge also removes the one thing that reclaimed a vanished peer, so the
  socket carries TCP keepalive (30 s idle, 10 s apart, three probes) and nothing else stands
  behind it — ISSUE-211. That is a transport probe, not the WebSocket ping the device still
  never sends. A receive timeout and a dead peer must stay distinguishable for it to work:
  lwIP reports the timeout as `EWOULDBLOCK` and the dead peer as something else.
- The upgrade still answers `101` before the handler (ISSUE-135), so a refusal is a Close
  frame, now written through the same codec.
- Only the bench can prove it: no host test reaches `esp_http_server`. An event rate the
  bench can raise never fills a client's queue, so the stimulus that tells this image from
  the one before it is a subscriber that reads a frame a second (`HIL-WS-10`): its own queue
  overflows either way, but the earlier image spent the block on the httpd task, so REST and
  the other subscriber paid for it, and here neither does. What the installation still cannot
  judge is `HIL-WS-07`, which needs an image whose PHY interrupt sits at level 3, and the
  keepalive reclaim of a vanished peer, which is ISSUE-211.
