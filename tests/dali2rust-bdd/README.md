# dali2rust-bdd

Host-side [cucumber-rs](https://github.com/cucumber-rs/cucumber) suite: the product's
acceptance scenarios, run against the production composition with mock ports.

Scope: how to run the suite and where its parts live. The conventions (layers,
black-box boundary, tags, step traceability, gates) live in
[`documentation/architecture/05-testing-and-bdd.md`](../../documentation/architecture/05-testing-and-bdd.md);
ID prefixes in
[`documentation/product-design/bdd/ids-registry.md`](../../documentation/product-design/bdd/ids-registry.md).

## Layout

- `features/` — the Gherkin scenarios; `src/steps/` — the step definitions; how both
  are organised: *Feature tree* and *Step definitions* in `05-testing-and-bdd.md`.
- `src/main.rs` — the World and the runner. The stack is composed by
  `dali2rust-adapters::build_http_test_stack` over `MockDaliTransport`.
- `benches/` — a bus microbenchmark, not a behaviour test.

## Running

```bash
just bdd          # or: cargo bdd
just bdd-check    # ID and coverage gates, then the suite
```

- `ONLY_FEATURES=<substring>` runs only the feature files whose path contains the
  substring, for example `ONLY_FEATURES=pd-discovery just bdd`.
- `ONLY_STAGE=<stage>` (what `just bdd-stage R6` sets) runs the scenarios of one stage:
  a scenario's own `@stage-*` tag overrides its feature's.
- A process runs its scenarios one at a time. `just bdd_shards=4 bdd` splits the
  feature files over four processes, balanced by scenario count; `BDD_SHARD=<i>/<n>` is
  what each process reads. `just bdd` keeps one process unless told otherwise.
- The runner skips `@wip` scenarios. A cucumber `--name` or `--tags` filter replaces the
  harness filter (the `@wip` skip, `ONLY_FEATURES` and `ONLY_STAGE`), so a tag expression
  that should keep the skip says so:
  `cargo bdd -- --tags '(@id:PD-027 or @id:PD-028) and not @wip'`.
- The gates the suite answers to are listed in `05-testing-and-bdd.md`; all of them run
  in `just verify`.
