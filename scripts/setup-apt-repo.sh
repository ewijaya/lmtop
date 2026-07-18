#!/usr/bin/env bash
# One-time setup for the lmtop apt repository (ewijaya/apt), documented in
# docs/releasing.md. Run locally, where `gh` is authenticated as ewijaya
# and `gpg` is installed. Safe to re-run: existing pieces are skipped —
# except the signing key, which is guarded separately below.
set -euo pipefail

# Never drop into gh's interactive pager mid-script.
export GH_PAGER=cat

OWNER=ewijaya
APT_REPO="$OWNER/apt"
SRC_REPO="$OWNER/lmtop"
KEY_UID="lmtop APT repository <ewijaya@gmail.com>"

# 1. The repository. Public: GitHub Pages requires it on a free plan, and
#    a package repository is meant to be fetched anonymously anyway.
if gh repo view "$APT_REPO" > /dev/null 2>&1; then
  echo "✓ $APT_REPO already exists"
else
  gh repo create "$APT_REPO" --public \
    -d "APT repository for lmtop (https://github.com/$OWNER/lmtop)"
  echo "✓ created $APT_REPO"
fi

# An initial commit must exist before Pages can deploy from a branch (and
# before actions/checkout works). Done via the contents API rather than
# `gh repo create --add-readme`, which older gh versions don't have.
if [ "$(gh api "repos/$APT_REPO/branches" --jq 'length')" -gt 0 ]; then
  echo "✓ initial commit already exists"
else
  readme="# apt

APT repository for [lmtop](https://github.com/$OWNER/lmtop), published by
its release workflow and served via GitHub Pages. Install instructions:
https://github.com/$OWNER/lmtop#debian--ubuntu-apt
"
  gh api "repos/$APT_REPO/contents/README.md" -X PUT \
    -f message="Initial commit" \
    -f content="$(printf '%s' "$readme" | base64 | tr -d '\n')" > /dev/null
  echo "✓ initial commit created"
fi

# 2. GitHub Pages, serving the default branch's root.
if gh api "repos/$APT_REPO/pages" > /dev/null 2>&1; then
  echo "✓ Pages already enabled"
else
  branch=$(gh api "repos/$APT_REPO" --jq .default_branch)
  # Raw JSON body: older gh versions don't expand -f 'source[branch]=…'
  # into a nested object, which the Pages API 422s on.
  printf '{"source":{"branch":"%s","path":"/"}}' "$branch" \
    | gh api "repos/$APT_REPO/pages" -X POST --input - > /dev/null
  echo "✓ Pages enabled: https://$OWNER.github.io/apt"
fi

# 3. Deploy key (write access to ewijaya/apt only) -> APT_DEPLOY_KEY secret.
if gh api "repos/$APT_REPO/keys" --jq '.[].title' | grep -qx "lmtop release bump"; then
  echo "✓ deploy key already installed (rotate: docs/releasing.md)"
else
  ssh-keygen -t ed25519 -f aptkey -N "" -C "lmtop-release-apt-bump"
  gh api "repos/$APT_REPO/keys" -X POST \
    -f title="lmtop release bump" -f key="$(cat aptkey.pub)" -F read_only=false > /dev/null
  gh secret set APT_DEPLOY_KEY --repo "$SRC_REPO" < aptkey
  shred -u aptkey aptkey.pub 2> /dev/null || rm -f aptkey aptkey.pub
  echo "✓ deploy key installed, APT_DEPLOY_KEY secret set"
fi

# 4. Repo signing key -> APT_SIGNING_KEY secret. rsa4096 for compatibility
#    with older gpgv builds; no expiry (an expired key bricks `apt update`
#    for every enrolled user); no passphrase (CI signs non-interactively).
#    Guarded by the local keyring only — if you generated it on another
#    machine, this creates a SECOND key and overwrites the secret, which
#    invalidates the key existing users enrolled. See docs/releasing.md
#    on rotation before re-running this step deliberately.
if gpg --list-secret-keys "$KEY_UID" > /dev/null 2>&1; then
  echo "✓ signing key already in local keyring; not regenerating"
else
  gpg --batch --passphrase '' --quick-generate-key "$KEY_UID" rsa4096 sign never
  gpg --armor --export-secret-keys "$KEY_UID" \
    | gh secret set APT_SIGNING_KEY --repo "$SRC_REPO"
  echo "✓ signing key generated, APT_SIGNING_KEY secret set"
fi

echo
echo "Done. The next v* tag publishes to https://$OWNER.github.io/apt —"
echo "install instructions are in the README's Debian/Ubuntu section."
