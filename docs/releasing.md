# Releasing

Releases are cut by pushing a version tag. Ordinary pushes to `main` only run
CI (fmt, clippy, tests) — they never publish — so committing stays cheap and
cutting a version stays deliberate.

## Cutting a release

```bash
# 1. Bump the version (Cargo.toml is the single source of truth).
#    The code reads it via env!("CARGO_PKG_VERSION").
vim Cargo.toml
cargo check                 # keeps Cargo.lock in sync; CI builds --locked

# 2. Commit and tag. The tag must match the Cargo.toml version: the Homebrew
#    formula's test block asserts `lmtop --version` reports it.
git commit -am "chore: release v0.2.0"
git tag v0.2.0
git push origin main --tags
```

The tag push triggers `.github/workflows/release.yml`, which:

1. runs the test suite (a tag whose tests fail publishes nothing);
2. builds `x86_64` and `aarch64` for both Linux and macOS;
3. publishes a GitHub Release with tarballs and a `SHA256SUMS` file;
4. regenerates `Formula/lmtop.rb` in [`ewijaya/homebrew-tap`][tap].

Within a minute or so of the workflow finishing:

```bash
brew update && brew install ewijaya/tap/lmtop
```

## Tap authentication

The formula bump pushes to a second repository, which the default
`GITHUB_TOKEN` cannot reach. Auth is a **deploy key**, not a PAT:

- the tap holds the public half as a write-enabled deploy key;
- this repo holds the private half as the `TAP_DEPLOY_KEY` Actions secret.

A deploy key is scoped to the tap alone, where a PAT with `repo` scope would
carry write access to every repository on the account.

To rotate it:

```bash
ssh-keygen -t ed25519 -f tapkey -N "" -C "lmtop-release-tap-bump"
gh api repos/ewijaya/homebrew-tap/keys -X POST \
  -f title="lmtop release bump" -f key="$(cat tapkey.pub)" -F read_only=false
gh secret set TAP_DEPLOY_KEY --repo ewijaya/lmtop < tapkey
shred -u tapkey tapkey.pub
```

Then delete the old key from the tap's *Settings → Deploy keys*.

If `TAP_DEPLOY_KEY` is missing, the release still succeeds — the bump job
logs a warning and skips, leaving the formula pointing at the previous
version.

## Platform caveat

The macOS binaries are cross-built in CI and **not verified by a human on
macOS**. Compiling is not the same as rendering correctly in a terminal.
Before promoting macOS support, run the binary on a real Mac.

[tap]: https://github.com/ewijaya/homebrew-tap
