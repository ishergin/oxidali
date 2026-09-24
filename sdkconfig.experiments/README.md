# sdkconfig experiment fragments

Fragments layered over `sdkconfig.p4.defaults` for a bench experiment, so the baseline is
never edited. Scope: the fragments only; the baseline's keys and reasons are in
[`10-build-release-and-tooling.md`](../documentation/architecture/10-build-release-and-tooling.md).

- `esp-idf-sys` reads `ESP_IDF_SDKCONFIG_DEFAULTS` as a `;`-separated list, later files
  overriding earlier ones.
- `.cargo/config.toml` forces that variable, so an exported or inline one does not
  override it; set it through cargo's own config layer:
  `cargo --config 'env.ESP_IDF_SDKCONFIG_DEFAULTS.value="sdkconfig.p4.defaults;sdkconfig.experiments/<fragment>"' fw`.
  `hil flash` always builds the baseline.
- Check the generated `sdkconfig` — the authority, as 10 says — before trusting a run.
- One flip per build; record every configuration, including those that change nothing.

| Fragment | Tests |
| --- | --- |
| `gptimer-isr-cache-safe-off.defaults` | The inverse control for `GPTIMER_ISR_CACHE_SAFE`. It changes behaviour only on the driver-owned interrupt (`DALI2RUST_PHY_ISR_LEVEL=3`) of an image that does not execute from PSRAM. |
