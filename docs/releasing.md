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
2. builds `x86_64` and `aarch64` for both Linux and macOS, plus `.deb`
   packages (via [`cargo-deb`][cargo-deb]) from the Linux musl binaries;
3. publishes a GitHub Release with tarballs, `.deb` files, and a
   `SHA256SUMS` file;
4. regenerates `Formula/lmtop.rb` in [`ewijaya/homebrew-tap`][tap];
5. publishes the `.deb`s and signed repo metadata to [`ewijaya/apt`][apt],
   which GitHub Pages serves at <https://ewijaya.github.io/apt>.

Within a minute or so of the workflow finishing:

```bash
brew update && brew install ewijaya/tap/lmtop
# or, with the apt repo enrolled (see the README):
sudo apt update && sudo apt install lmtop
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

## APT repository

`bump-apt` pushes the `.deb` packages into [`ewijaya/apt`][apt], a plain
static APT repository (a `pool/` of packages plus signed indexes under
`dists/stable/`) that GitHub Pages serves at
`https://ewijaya.github.io/apt`. Users enroll it once (README has the
commands) and then `apt upgrade` tracks releases. Two secrets are involved,
and if either is missing the job warns and skips — the repo simply keeps
serving the previous version:

- **`APT_DEPLOY_KEY`** — private half of a deploy key on `ewijaya/apt`,
  same pattern and rationale as `TAP_DEPLOY_KEY`.
- **`APT_SIGNING_KEY`** — ASCII-armored GPG private key that signs the repo
  metadata (`Release.gpg` / `InRelease`); `apt` refuses unsigned repos. The
  workflow re-exports the public half to `lmtop.gpg` in the repo root on
  every publish, so clients always fetch the current key. The key must have
  **no passphrase** (the job signs non-interactively).

### One-time setup

All of the below is scripted, idempotently, as `scripts/setup-apt-repo.sh`
— run that locally (needs an authenticated `gh` and `gpg`). The individual
steps, for reference and rotation:

```bash
# 1. The repository: must exist with an initial commit, with GitHub Pages
#    enabled on main / root (Settings → Pages → Deploy from a branch).
gh repo create ewijaya/apt --public --add-readme \
  -d "APT repository for lmtop (https://github.com/ewijaya/lmtop)"
gh api repos/ewijaya/apt/pages -X POST \
  -f 'source[branch]=main' -f 'source[path]=/'

# 2. Deploy key, exactly like the tap's:
ssh-keygen -t ed25519 -f aptkey -N "" -C "lmtop-release-apt-bump"
gh api repos/ewijaya/apt/keys -X POST \
  -f title="lmtop release bump" -f key="$(cat aptkey.pub)" -F read_only=false
gh secret set APT_DEPLOY_KEY --repo ewijaya/lmtop < aptkey
shred -u aptkey aptkey.pub

# 3. Signing key. rsa4096 rather than ed25519 for maximum compatibility
#    with the gpgv shipped on older still-supported distro releases; no
#    expiry, because an expired key bricks `apt update` for every user
#    until they re-enroll.
gpg --batch --passphrase '' --quick-generate-key \
  "lmtop APT repository <ewijaya@gmail.com>" rsa4096 sign never
gpg --armor --export-secret-keys "lmtop APT repository" \
  | gh secret set APT_SIGNING_KEY --repo ewijaya/lmtop
```

To rotate either key, repeat the matching step (and delete the old deploy
key from `ewijaya/apt`'s *Settings → Deploy keys*). After rotating the
signing key, the next release publishes the new public key — but existing
users still hold the old one in `/usr/share/keyrings/lmtop.gpg` and their
`apt update` will fail signature verification until they re-run the
key-enrollment command from the README, so rotate only deliberately and
say so in the release notes.

Standing obligation, stated honestly: once people enroll the repo, the
Pages site and the signing key are infrastructure. Deleting `ewijaya/apt`,
disabling Pages, or losing the key turns every enrolled machine's
`apt update` into an error until they remove
`/etc/apt/sources.list.d/lmtop.list`.

## Platform caveat

The macOS binaries are cross-built in CI and **not verified by a human on
macOS**. Compiling is not the same as rendering correctly in a terminal.
Before promoting macOS support, run the binary on a real Mac.

[tap]: https://github.com/ewijaya/homebrew-tap
[apt]: https://github.com/ewijaya/apt
[cargo-deb]: https://github.com/kornelski/cargo-deb
