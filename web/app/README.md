# dali2rust web UI

Preact + TypeScript single-page app for the controller, embedded in the firmware and
served from flash. It reads and writes the REST API (`/api/v1/*`) and listens to the
WebSocket push channel (`/api/v1/ws`), both served identically by the device and by
the host dev server.

Scope: the app's code layout, dev workflow and code conventions. The screens and the
rules every screen follows are in
[`documentation/product-design/web-ui/`](../../documentation/product-design/web-ui/README.md);
the API contracts in
[`documentation/product-design/rest-api/`](../../documentation/product-design/rest-api/README.md);
how UI work is built, carded and shipped in
[`10-build-release-and-tooling.md`](../../documentation/architecture/10-build-release-and-tooling.md).

## Layout

One file per screen in `src/screens/`; routes in `src/router.ts`, sidebar groups
(`NAV_GROUPS`) and the route switch in `src/app.tsx`; the typed client and DTO mirror in
`src/api/`; shared components in `src/components/ui.tsx`. A screen appears only once its
endpoint exists on the device.

## Dev workflow

```bash
# 1. the composed product stack on a simulated DALI bus, 127.0.0.1:8080
cargo run --target <host-triple> -p dali2rust-adapters --example host_dev_server
# 2. Vite with hot reload on :5173, proxying /api (WebSocket included) to it
cd web/app && npm ci && npm run dev
# or against a real controller
DALI2RUST_API_TARGET=http://<controller> npm run dev
```

The dev server takes `DALI2RUST_DEV_SERVER_ADDR` (or `PORT`) to move, and
`DALI2RUST_DEV_SERVER_FLEET=bench` for a full 64-address bus; its stdin console
plays a foreign master and a wall panel (`press`, `release`, `foreign`,
`occupancy`). It reads the bundle once at startup: restart it after
`bash scripts/build_web_ui.sh`. Shipping a UI change to the device is the chain in
`10-build-release-and-tooling.md`.

## Conventions

- **Data** loads through `usePoll` (paused while the tab is hidden) or `useLive`,
  which adds WebSocket channels as refetch triggers; how the two share one data shape
  is [`web-ui/websocket.md`](../../documentation/product-design/web-ui/websocket.md).
- **Operations**: a `202 { operation_id }` is polled to a terminal state by
  `trackOp` / `runOp`, and anything that acts on the result asks `opCommitted`.
- **Inputs over polled data** use `EditableText` / `EditableName`; the colour control
  never sends `power`; free-running counters show a delta beside the absolute value —
  the rules behind all three are in
  [`web-ui/README.md`](../../documentation/product-design/web-ui/README.md).
- **No modals, no `prompt()`**: inline editors, two-step "Sure?" for destructive
  actions.
- **Every interactive control uses a class from `app.css`** (`btn`, `sel`, `field`,
  `act`, …); checkboxes stay native app-wide. `ADAPTER = 0`: one adapter per
  controller.

## Adding a screen

1. Route: an entry in `ROUTES` and the `Route['name']` union (`src/router.ts`).
2. API: mirror the DTOs in `src/api/types.ts`, add typed calls to `src/api/client.ts`.
3. Screen: `src/screens/<name>.tsx`, data through `usePoll` / `useLive`, actions
   through `runOp` / `notify`, layout from the shared components in
   `src/components/ui.tsx`.
4. Wire it: the case in `Screen()` and a line in `NAV_GROUPS` (`src/app.tsx`).
5. Its design card, then `npm run build` and the flow against the dev server.
