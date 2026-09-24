# Documentation

The map of dali2rust's documentation: which tree owns which kind of fact, and the rules
every document follows.

## Map

- [`architecture/`](architecture/README.md) — **how** the system is built, as it is:
  mechanisms, boundaries and the invariants code must keep; its README gives the
  reading order. English.
- [`architecture/decisions/`](architecture/decisions/README.md) — **why**: one ADR per
  locked decision, with its alternatives and consequences. English.
- [`product-design/`](product-design/README.md) — **what** the product does: external
  contracts (REST, WebSocket, MQTT), bus contract families, runtime modules, web UI,
  glossary and invariants, known issues, the issue registry, status and roadmap; its
  README is the package map. Russian.
- [`reference/`](reference/) — external standards: IEC 62386 and DiiA digests, the
  conformance gap list, and the crash-triage runbook. English.
- [`../tools/hil/`](../tools/hil/README.md) — the hardware bench: runbook (English),
  strategy and watchlist (Russian).
- `tests/dali2rust-bdd/features/` — the executable BDD canon.
- [`../CLAUDE.md`](../CLAUDE.md) — one-line rules for contributors and agents, pointing
  here.

## Rules

- **One home per fact.** A fact is stated once; everywhere else links to it. A home
  never defers its own topic to another document.
- **Code wins.** When a document and the code disagree, the code is canonical and the
  document is corrected.
- **General, not duplicated.** Documents state purpose, boundaries and invariants;
  details the code already holds — field lists, constants, enumerations — stay there.
- **One shape.** A document opens with its title, one to three lines of purpose and a
  `Scope:` line (`Границы:` in Russian) naming what it owns and linking what lives
  elsewhere, then short sections. The size cap is 40 KB; the exceptions and their
  reasons are in `scripts/doc_size_caps.txt`.
- **No history.** Documents state what is. Dates, measurement narratives and superseded
  designs live in git history; the exceptions are an ADR's `Date:` line, the opening
  date of an open known issue, and the bench watchlist.
- **Issue numbers** are allocated only in
  [`issue-ids-registry.md`](product-design/issue-ids-registry.md); a closed issue keeps
  only its registry row.

`scripts/verify_docs.py` holds the links, the size caps, the dates and repeated
passages ([10](architecture/10-build-release-and-tooling.md) §Merge gates).
