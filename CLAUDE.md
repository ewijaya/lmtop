# lmtop — working agreements

## Always bump the version

**Every change to `src/` bumps the patch version in `Cargo.toml`, in the
same commit as the change.** No exceptions, and no waiting to be asked.

```bash
# 0.3.0 -> 0.3.1, in the same commit as the code change
vim Cargo.toml          # version = "0.3.1"
cargo check             # keeps Cargo.lock in sync; CI builds --locked
```

Why: the version is how Edward tells whether the binary he is running
(`~/.cargo/bin/lmtop`, installed with `cargo install --path . --locked`)
contains a given change. Two edits shipped under one version number, and
he could not tell his binary was stale from the header alone. The header
renders `env!("CARGO_PKG_VERSION")`, so bumping is all it takes to make
the running build identifiable.

Scope:

- **`src/` changed → bump.** Features, fixes, refactors, anything.
- **Docs / screenshots / CI only → no bump.** Nothing about the binary
  changed, so the number should not move.
- Use a minor bump (0.3.x → 0.4.0) for a substantial feature batch, and
  say so; patch is the default.

After a bump, remind Edward to reinstall if he wants the change in his
running binary:

```bash
cargo install --path . --locked   # then quit and relaunch lmtop
```

## Versions are not releases

Bumping is cheap and constant; **releasing is deliberate and Edward's
call**. Pushing a `v*` tag triggers `.github/workflows/release.yml`, which
publishes a public GitHub Release and rewrites `Formula/lmtop.rb` in
`ewijaya/homebrew-tap` — that is what `brew install` then serves.

So: bump freely, **never push a tag without being asked**. Most versions
will never be tagged, which is expected. See `docs/releasing.md`.

## Before committing

CI gates on all three, and two commits have already slipped through
failing `fmt`:

```bash
cargo test && cargo clippy --all-targets && cargo fmt --check
```

## Honesty about data

This codebase's whole value is that its numbers can be trusted. Keep to
the rules the docs already state:

- Never convert observed tokens into quota percentages, or vice versa —
  providers weight models and caching in ways they do not publish.
- Unavailable data renders as *unavailable*, never as a guess or a zero.
- Estimates carry their confidence; stale data says it is stale.
- When a source cannot supply something (e.g. the Codex state DB has no
  input/output split), surface the gap rather than papering over it.
