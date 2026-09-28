# ADR-032: One installation, two nodes: the controller's identity

Status: Accepted
Date: 2026-09-28

## Context

`GET /api/v1/controller` answered constants for `controller_id`, `network` and `cluster`.
The one identity the firmware kept was the Home Assistant settings' `controller_id`: seeded
from the Ethernet MAC, replicated to the standby, the root of every Home Assistant
`unique_id` and the MQTT client id ([ADR-018](ADR-018-controller-redundancy.md)). The two
boards of a pair therefore shared it, and nothing told them apart across a failover: the
role moves with the bus. The network saw no name at all — DHCP got the ESP-IDF default
`espressif` — and the cluster (I6) needs a per-node origin for loop suppression, which
`cluster_origin_id` reserves and leaves at 0.

## Decision

1. **The installation is `controller_id`.** `/api/v1/controller` reports the Home Assistant
   settings' `controller_id`: shared by the pair, set through that resource or by importing
   its slice, which is how the standby gets it.
2. **The node is `node_id`: `dali-` and the last three bytes of the node's own MAC**, in
   lower-case hex. It is read from the MAC the Ethernet driver reports and is never stored or
   replicated. On the device the MAC is always known; only a build that composes no network
   link, a host stack, reports `null`. One formatter builds this string for the node, the
   hostname and the default installation id, so a board that seeded the installation reports
   the same string twice until the installation is renamed.
3. **The hostname is the node id**, set once on the interface before the Ethernet driver
   starts, so the first DHCP request carries it. No setting changes it. `network` reports the
   hostname the interface carries, the link's MAC and the IPv4 address of its lease; a field
   with nothing behind it — the address before a lease, every field without a link — is
   `null`.
4. **The cluster's origin is the node.** When I6 composes, `cluster_origin_id` carries the
   node's MAC as a 48-bit integer; REST and logs show the node id. Until then
   `cluster.enabled` stays `false` and the field stays 0.
5. **No mDNS.** The responder costs internal SRAM, whose floor is ISSUE-71 in
   [known-issues](../../product-design/known-issues.md).

## Consequences

- The standby reports the installation it pulled from the active and its own node, so a tool
  that tells the boards apart keys on `node_id`, and one that proves they serve one
  installation compares `controller_id`.
- A Home Assistant discovery topic has a segment Home Assistant calls `node_id`; it carries
  the installation's `controller_id`, not this node id, and documents name it the
  discovery node segment.
- A host stack without a network link has no node: `node_id` and every `network` field are
  `null`, and the installation keeps its fallback `dali-controller`.
- This record is the explicit migration the
  [stability rules](../../product-design/rest-api/stability-and-versioning.md) ask for when
  `network.hostname` and `network.ip` go from a string to a string or `null`: they had been
  documented as unfilled constants, so no client depended on their values.
