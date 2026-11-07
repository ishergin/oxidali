# ADR-034: The WebSocket socket belongs to a worker

Status: Accepted; amends ADR-002 and ADR-023
Date: 2026-11-07

## Context

One `httpd` task serves every socket, so every millisecond a handler holds it is downtime
for all clients ([02](../02-runtime-and-threading.md) §The HTTP task). A WebSocket frame
was written on that task: the sender thread handed a batch to `httpd_queue_work` and the
httpd task ran `httpd_ws_send_frame_async` for it. The coupling runs both ways.

Outward, it is ISSUE-39: while the task serves REST responses the queued send waits, and a
client's queue overflows. Everything cheap has been done — routed delivery
([ADR-015](ADR-015-routed-event-delivery.md)), coalescing by projection key
([ADR-023](ADR-023-named-subscribers-and-shared-coalescing.md)), a light device summary,
one work item per batch instead of one per frame — and the bench cannot tell the fixed
image from the unfixed one, because the event rate never fills the queue.

Inward, it is ISSUE-210: one work item now carries the whole batch, and each frame is two
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

Fragmentation is refused rather than reassembled. Nothing here joins fragments, so a
non-final data frame or a Continuation frame is an error — accepting one would hand the hub
half a request and read the rest as a fresh header.

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

- ISSUE-39's root is gone by construction rather than mitigated: a REST response cannot
  delay a WebSocket frame, because they no longer share a task. ISSUE-210's httpd hold goes
  with it; what is left of that record is the batch's own bound.
- Leaving LRU purge also keeps load above the socket pool from closing a subscriber, and
  removes the LRU candidate of ISSUE-55.
- The slot pool, the result slot and the deadline that existed only to cross onto the httpd
  task are gone, and with them the work item ISSUE-206 said ESP-IDF can silently lose.
- About 1.2 KB of internal SRAM per live connection, held for its life, against the one-off
  ~3 KB of the refused second instance: dearer at four clients, and it lands on ISSUE-71.
- A reader runner per client, named `ws-reader`, with a stack budget that has never been
  measured. The client-request parse moves off the 20 KiB httpd stack onto it.
- The upgrade still answers `101` before the handler (ISSUE-135), so a refusal is a Close
  frame, now written through the same codec.
- Only the bench can prove it: no host test reaches `esp_http_server`, and the acceptance is
  `HIL-WS-07` and `HIL-WS-09`.
