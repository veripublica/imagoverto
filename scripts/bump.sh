#!/usr/bin/env bash
#
# Step 1 of a release: the manifest version, Cargo.lock, and the CHANGELOG
# heading the release body is taken from (kepubverto's scripts/bump.sh).
#
# Usage: scripts/bump.sh 0.1.0 [YYYY-MM-DD]   (date defaults to today)
#
# Writes only the two manifests (imagoverto, imagoverto-wasm: one version,
# because the npm package is built from the tag), Cargo.lock and
# CHANGELOG.md; commits nothing. The
# README sweep for stale numbers stays a reading job.

set -euo pipefail
cd "$(dirname "$0")/.."

V="${1:?usage: scripts/bump.sh X.Y.Z [YYYY-MM-DD]}"
D="${2:-$(date +%F)}"
[[ "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $V" >&2; exit 2; }

grep -q '^## \[Unreleased\]' CHANGELOG.md \
  || { echo "CHANGELOG.md has no [Unreleased] section to release" >&2; exit 1; }
! grep -q "^## \[$V\]" CHANGELOG.md \
  || { echo "CHANGELOG.md already has a [$V] section" >&2; exit 1; }

# The first `version =` in each manifest is the package's own.
for m in Cargo.toml imagoverto-wasm/Cargo.toml; do
  awk -v v="$V" '!done && /^version = "/ { print "version = \"" v "\""; done=1; next } { print }' \
    "$m" > "$m.tmp" && mv "$m.tmp" "$m"
done
sed -i '' "s/^## \[Unreleased\]/## [$V] - $D/" CHANGELOG.md

# Cargo.lock follows the manifests (the two workspace entries only).
cargo build -q -j 8 --workspace

grep -H -m1 '^version' Cargo.toml imagoverto-wasm/Cargo.toml
grep -n "^## \[$V\]" CHANGELOG.md
git diff --stat
