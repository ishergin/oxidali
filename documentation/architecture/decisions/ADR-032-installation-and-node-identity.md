# ADR-032: One installation, two nodes: the controller's identity

Status: Accepted
Date: 2026-09-28

## Context

`GET /api/v1/controller` answers constants for `controller_id`, `network` and `cluster`.
The one identity the firmware keeps is the Home Assistant settings' `controller_id`: seeded
from the Ethernet MAC, replicated to the standby, the root of every Home Assistant
`unique_id` and the MQTT client id ([ADR-018](ADR-018-controller-redundancy.md)). The two
boards of a pair therefore share it, and nothing tells them apart across a failover: the
role moves with the bus. The network sees no name at all — DHCP gets the ESP-IDF default
`espressif` — and the cluster (I6) needs a per-node origin for loop suppression, which
`cluster_origin_id` reserves and leaves at 0.

## Decision

1. **The installation is `controller_id`.** `/api/v1/controller` reports the Home Assistant
   settings' `controller_id`: shared by the pair, changed only through that resource.
2. **The node is `node_id`: `dali-` and the last three bytes of the node's own MAC**, in
   lower-case hex. It is read from the MAC the Ethernet driver reports, never stored or
   replicated, and `null` where there is no link. One formatter builds this string for the
   node, the hostname and the default installation id, so a board that seeded the
   installation reports the same string twice until the installation is renamed.
3. **The hostname is the node id**, set on the interface before the Ethernet driver starts,
   so the first DHCP request carries it. No setting changes it. `network` reports it with
   the link's IPv4 address and MAC, each `null` when unknown.
4. **The cluster's origin is the node.** When I6 composes, `cluster_origin_id` carries the
   node's MAC as a 48-bit integer; REST and logs show the node id. Until then
   `cluster.enabled` stays `false` and the field stays 0.
5. **No mDNS.** The responder costs internal SRAM, whose floor is ISSUE-71.

## Consequences

- The standby reports the installation it pulled from the active and its own node, so a tool
  that tells the boards apart keys on `node_id`, and one that proves they serve one
  installation compares `controller_id`.
- A Home Assistant discovery topic has a segment Home Assistant calls `node_id`; it carries
  the installation's `controller_id`, not this node id, and documents name it the
  discovery node segment.
- A host stack without a network link has no node: `node_id` and `network` are `null`, and
  the installation keeps its fallback `dali-controller`.
