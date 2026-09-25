# Contributing to Oxidali

Changes reach `main` through pull requests only; CI runs on every one. The project's
rules — crate boundaries, threading, contracts, testing and merge gates — are in
[`CLAUDE.md`](CLAUDE.md), and the recipes for a new command, endpoint or worker are in
[`11-extension-recipes.md`](documentation/architecture/11-extension-recipes.md).

## Before you open a pull request

```bash
(cd web/app && npm ci)   # once: the gates type-check and test the web UI
just ci                  # clippy, host tests, BDD, merge gates, firmware type-check
```

New behaviour starts with a failing BDD scenario. Keep one logical change per commit,
and say in the message why it is needed.

## Changing the web UI

Run the UI against the simulated bus of the host dev server:

```bash
cargo run --target <your host triple> -p dali2rust-adapters --example host_dev_server
cd web/app && npm run dev    # http://localhost:5173
```

A UI pull request carries sources only:

- the code in `web/app`;
- in the same pull request, the card in `web/design-system/` of every screen or
  component whose look changes. A card is self-contained HTML: it inlines the `:root`
  block of `tokens.css` verbatim, and every class the app uses is defined in some card;
- a change to screens, components or `app.css` that leaves what the user sees as it
  was (a refactor, a data fix) says so in a commit trailer: `UI-Design: unchanged`.

Two things are the maintainer's sync, done when a release is flashed, and are not part
of your pull request:

- the embedded bundle in `crates/dali2rust-firmware/assets/web/`. Build it with
  `bash scripts/build_web_ui.sh` to try your UI on a board of your own; committing it is
  optional;
- `web/design-system/.pushed.sha256`, which records what was pushed to the
  maintainer's Claude Design project.

The details of the UI build chain are in
[`10-build-release-and-tooling.md`](documentation/architecture/10-build-release-and-tooling.md)
§Embedded web UI, and the screen rules in
[`web-ui/README.md`](documentation/product-design/web-ui/README.md).
