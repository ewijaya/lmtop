# Contributing

> **Naming:** the product name is `lmtop`, always lowercase (like `top`,
> `htop`, `btop`). Keep name references in code going through
> `src/branding.rs`.

## Development setup

Stable Rust (1.85+ recommended) via [rustup](https://rustup.rs):

```bash
cargo build
cargo test --all
```

## Before sending changes

Run the full verification suite; CI-equivalent locally:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

## Ground rules

- **Privacy first.** Nothing may read, log, or persist prompt content,
  tool output, or credentials. Collectors extract usage metadata only.
  See `docs/privacy.md`; treat it as a contract, not documentation.
- **Layering.** The UI depends only on `src/domain/` types. Provider
  schemas live exclusively inside `src/collectors/`. If a widget needs a
  new fact, add it to the domain model, then teach collectors to fill it.
- **Never fabricate provider data.** Unknown quota windows stay unknown;
  unavailable capabilities render as unavailable. Do not infer quota
  percentages from observed tokens.
- **Fixtures, not real data.** Tests use synthetic session files under
  `tests/fixtures/`. Never commit real session logs, even redacted ones.
- **Deterministic time.** Anything time-dependent takes a clock/`ScanContext`
  argument; tests must not depend on the wall clock.
- **No new dependencies without a concrete need.**

## Adding a provider

1. Add a variant to `domain::Provider`.
2. Write a collector implementing `collectors::Collector`, feeding the
   shared `UsageStore`.
3. Declare only the capabilities the provider actually supports.
4. Add synthetic fixtures and integration tests.
5. Document the data source in `docs/data-sources.md`.
