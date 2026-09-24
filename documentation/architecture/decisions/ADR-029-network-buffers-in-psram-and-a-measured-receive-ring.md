# ADR-029: Network buffers live in PSRAM and the receive ring is sized by its drop counter

Status: Accepted
Date: 2026-09-24

## Context

After [ADR-028](ADR-028-task-stacks-in-psram-on-the-xip-image.md) the internal minimum
still fell under network load, and the network configuration rested on three premises
nobody had measured:

- **lwIP was kept internal** (`SPIRAM_TRY_ALLOCATE_WIFI_LWIP=n`) on the reading that a
  PSRAM miss during an EMAC DMA refill is a silent drop. In ESP-IDF 5.5.3 the EMAC's
  descriptors and buffers are allocated once, at driver install, with DMA and internal
  capabilities, and are never refilled from lwIP. The option moves only lwIP's own
  allocations: pbufs, queued segments, control blocks.
- **Received frames are not lwIP's allocation.** The EMAC receive task copies each frame
  out of the DMA ring into a plain `malloc` — internal SRAM below 16 KiB — and lwIP holds
  that buffer until the application reads it: in the TCP/IP queue, the socket mailbox and
  the out-of-order queue.
- **The receive ring could drop frames with every counter at zero.** The MAC counts the
  frames it discards for want of a descriptor, and those its FIFO could not take, in a
  read-to-clear register nothing read. `rx_dropped` counted only the stack refusing a
  frame the MAC had delivered.

Measuring the third exposed a fourth: the socket receive mailbox held 6 segments against
a window of 16. lwIP refuses a segment the mailbox cannot take, drops the ones behind it
while refused data is pending, and retries only from its 250 ms timer, so the sender
waits for a retransmit. Three concurrent uploads ran at a tenth of the rate of one.

Measured on the standby from the Wiren Board, 20 s per run: uploads are 60 KB bodies to a
path that answers `404` after reading them, downloads the 68 KB web bundle. Overruns are
`rx_ring_overruns_total` over the run; internal is the lowest free internal SRAM sampled.

| Configuration | 1 upload | 3 uploads | 3 up + 3 down | Internal under 3 + 3 |
| --- | --- | --- | --- | --- |
| Before (mailbox 6, lwIP internal, ring 1600 B × 20/10) | 737 KB/s, 0 | 94 KB/s, 0 | 100 + 72 KB/s, 0–4 | 335 KB |
| Mailbox 18 | 920 KB/s, 0 | 2 639 KB/s, 7 | 1 703 + 1 819 KB/s, 24 | 291 KB |
| + lwIP in PSRAM | 922 KB/s, 0 | 2 633 KB/s, 0 | 1 700 + 1 744 KB/s, 39 | 295 KB |
| + received frames in PSRAM | 891 KB/s, 0 | 2 549 KB/s, 7 | 1 593 + 1 673 KB/s, 33 | 368 KB (idle 369) |
| + ring 512 B × 30/15 | 857 KB/s, 79 | 1 860 KB/s, 431 | 1 304 + 1 319 KB/s, 462 | 392 KB |
| + ring 768 B × 30/15 | 879 KB/s, 0 | 2 452 KB/s, 106 | 1 481 + 1 619 KB/s, 191 | 379 KB |

## Decision

1. **lwIP allocates from PSRAM first** (`SPIRAM_TRY_ALLOCATE_WIFI_LWIP=y`). The
   out-of-order queue stays bounded at 4 segments per connection, the value it had.
2. **Every received frame is copied into PSRAM** by the netif input glue
   (`dali2rust_bsp::esp32p4::eth`) and the driver's internal buffer is freed at once. If
   PSRAM is exhausted the frame goes on in the driver's buffer.
   `DALI2RUST_ETH_RX_INTERNAL=1` hands lwIP the driver's buffer as before.
3. **The TCP windows stay 16 × MSS** (23 040 B) for sending and receiving, and the socket
   receive mailbox holds the whole window: window / MSS + 2 = 18, the size ESP-IDF
   documents for it.
4. **The DMA ring is 768-byte buffers, 30 for receive and 15 for transmit** — a full
   frame in two descriptors, so 15 full frames or 30 small ones — 34.5 KB of internal
   memory instead of 48 KB.
5. **The MAC's discard counters are drained into running totals on every link-stats read
   and published** as `network.rx_ring_overruns_total` and `rx_fifo_overflows_total` on
   `/api/v1/stats`, beside the packet and drop totals. The ring is sized against them.

### Rejected alternatives

- **512-byte buffers, 30 / 15** (23 KB): a single upload overran the ring, because 15 KB
  cannot hold one window's burst.
- **Keeping 1600-byte buffers, 20 / 10**: 12 KB of internal SRAM buys fewer overruns only
  with three or more concurrent uploads. The product makes one at a time — an update
  download, a replication pull — and the web UI's bulk traffic is outbound.
- **Smaller windows to bound what lwIP holds**: with its buffers and the received frames
  in PSRAM a window costs no internal SRAM, and an upload from a remote operator runs at
  window / round trip.
- **Ethernet flow control**: `ETH_SOFT_FLOW_CONTROL` only compiles the check; it acts
  after `ETH_CMD_S_FLOW_CTRL`, which the firmware never issued. The key is removed rather
  than left looking active, and pause frames stay off: a pause stops the switch port for
  every sender, and the product's traffic does not overrun the ring.

## Consequences

- Network load no longer moves the internal minimum: lwIP's buffers, the frames it holds
  and the segments it sends are in PSRAM. The DMA ring stays internal, as the driver
  requires.
- Each received frame costs one more copy, 3–5 % of upload throughput as measured.
- Concurrent uploads run at the rate of one instead of a tenth of it.
- Three or more concurrent uploads overrun the ring; TCP resends what it drops, and the
  counters show it. A new source of concurrent uploads is sized against them.
