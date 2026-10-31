#!/usr/bin/env bash
#
# The last gate before a release tag: everything CI runs, plus what the
# release workflow and the family's rules check, moved to before the push.
# kepubverto's scripts/preflight.sh is the model.
#
# Usage:
#   scripts/preflight.sh          # check the version in Cargo.toml
#   scripts/preflight.sh 0.1.0    # check a version you are about to tag
#
# Every check runs even after one fails. The last line is the verdict:
# READY or NOT READY. Exit 0 = safe to `git push origin main vX.Y.Z`.
# Builds use all cores but two (Baris).

set -uo pipefail
cd "$(dirname "$0")/.."

PASS=0; FAIL=0; FAILED=()
T=$SECONDS
JOBS=$(( $(sysctl -n hw.ncpu 2>/dev/null || nproc) - 2 )); [ "$JOBS" -ge 1 ] || JOBS=1
took() { local d=$((SECONDS - T)); T=$SECONDS; printf ' [%ds]' "$d"; }
ok()   { printf '  ok    %s' "$1"; took; echo; PASS=$((PASS + 1)); }
bad()  { printf '  FAIL  %s' "$1"; took; echo; FAIL=$((FAIL + 1)); FAILED+=("$1"); }
skip() { printf '  --    %s' "$1"; took; echo; }

# check <label> <command...>: runs it, prints the tail only on failure.
check() {
  local label="$1"; shift
  local out
  if out=$("$@" 2>&1); then ok "$label"; else
    bad "$label"; printf '%s\n' "$out" | tail -15 | sed 's/^/        /'
  fi
}

CRATE_V=$(grep -m1 '^version = "' Cargo.toml | cut -d'"' -f2)
V="${1:-$CRATE_V}"
echo "version $V (crate $CRATE_V), $JOBS jobs"

# ------------------------------------------------------------------ version
[ "$CRATE_V" = "$V" ] && ok "Cargo.toml is $V" || bad "Cargo.toml is $CRATE_V, not $V"
WASM_V=$(grep -m1 '^version = "' imagoverto-wasm/Cargo.toml | cut -d'"' -f2)
[ "$WASM_V" = "$V" ] && ok "imagoverto-wasm is $V (the npm version)" \
  || bad "imagoverto-wasm/Cargo.toml is $WASM_V, not $V"
grep -q "^## \[$V\]" CHANGELOG.md && ok "CHANGELOG.md has [$V] (the release body)" \
  || bad "CHANGELOG.md has no [$V] section"
grep -q '^## \[Unreleased\]' CHANGELOG.md && bad "CHANGELOG.md still has [Unreleased]" \
  || ok "no [Unreleased] left in CHANGELOG.md"

# ------------------------------------------------------------------ tree
if [ -z "$(git status --porcelain)" ]; then ok "working tree clean (build.rs stamps .dirty)"
else bad "working tree dirty"; git status --short | sed 's/^/        /'; fi

git rev-parse -q --verify "refs/tags/v$V" >/dev/null && bad "tag v$V exists locally" || ok "tag v$V free locally"
if git fetch -q origin 2>/dev/null; then
  B=$(git rev-list --count HEAD..origin/main)
  [ "$B" = 0 ] && ok "main is not behind origin" || bad "main is $B commit(s) behind origin/main"
  git ls-remote -q --exit-code --tags origin "v$V" >/dev/null && bad "tag v$V exists on origin" \
    || ok "tag v$V free on origin"
else
  bad "git fetch failed"
fi

# A registry version can never be reused: check both before the tag.
UA="imagoverto-preflight (github.com/veripublica/imagoverto)"
curl -sf -A "$UA" "https://crates.io/api/v1/crates/imagoverto/$V" >/dev/null \
  && bad "imagoverto $V is already on crates.io" || ok "imagoverto $V not on crates.io yet"
[ -n "$(npm view "@veripublica/imagoverto-wasm@$V" version 2>/dev/null)" ] \
  && bad "@veripublica/imagoverto-wasm $V is already on npm" \
  || ok "@veripublica/imagoverto-wasm $V not on npm yet"

# The CLAUDE.md rule, checked rather than remembered.
LAST=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
if git log --format='%B' ${LAST:+$LAST..}HEAD | grep -qiE 'claude|anthropic|co-authored-by'; then
  bad "a commit since ${LAST:-the root} names Claude/Anthropic or has a Co-Authored-By"
else
  ok "no attribution in commits since ${LAST:-the root}"
fi

# ------------------------------------------------------------------ CI gates
check "cargo fmt --all --check" cargo fmt --all --check
check "cargo build --locked" cargo build -j "$JOBS" --locked
if out=$(cargo test -j "$JOBS" --workspace --locked -- --test-threads="$JOBS" 2>&1); then
  # Every `test result:` line, summed: a cut output can hide a failing target.
  sum=$(grep '^test result:' <<< "$out" | awk '{p+=$4; f+=$6; i+=$8} END {print p" passed, "f" failed, "i" ignored"}')
  ok "cargo test --workspace --locked ($sum)"
else
  bad "cargo test --workspace --locked"
  grep -E '^test .* FAILED|^failures:|panicked' <<< "$out" | head -15 | sed 's/^/        /'
fi
# The reference tools the tests compare against; a missing one passes with a
# note there, so say here which comparisons ran.
for t in dav1d djxl avifenc avifdec; do
  command -v "$t" >/dev/null && ok "reference tool $t present" || skip "reference tool $t missing (CI on Linux has it)"
done
check "cargo clippy (-D warnings)" cargo clippy -j "$JOBS" --workspace --all-targets --locked -- -D warnings
check "cargo doc (-D warnings)" env RUSTDOCFLAGS="-D warnings" cargo doc -j "$JOBS" --workspace --no-deps --locked
check "lib for wasm32-unknown-unknown" cargo build -j "$JOBS" --lib --target wasm32-unknown-unknown --locked
check "lib without default features" cargo check -j "$JOBS" --lib --no-default-features --locked
# What crates.io receives, built from the packaged files alone (no git
# dependency, nothing missing from `include`).
check "cargo publish --dry-run" cargo publish -j "$JOBS" --dry-run --locked
if command -v wasm-pack >/dev/null; then
  PKG=$(mktemp -d)
  check "wasm-pack build (bundler, as npm gets it)" \
    wasm-pack build imagoverto-wasm --target bundler --scope veripublica --out-dir "$PKG/pkg"
  if [ -f "$PKG/pkg/package.json" ]; then
    PNAME=$(jq -r .name "$PKG/pkg/package.json"); PVER=$(jq -r .version "$PKG/pkg/package.json")
    [ "$PNAME@$PVER" = "@veripublica/imagoverto-wasm@$V" ] && ok "npm package is $PNAME@$PVER" \
      || bad "npm package is $PNAME@$PVER, not @veripublica/imagoverto-wasm@$V"
    [ -f "$PKG/pkg/LICENSE" ] && ok "npm package has LICENSE" || bad "npm package lacks LICENSE"
  fi
  rm -rf "$PKG"
else
  bad "wasm-pack is not installed"
fi
if command -v wasmtime >/dev/null; then
  check "tests on wasm32-wasip1" env CARGO_TARGET_WASM32_WASIP1_RUNNER=wasmtime \
    cargo test -j "$JOBS" --lib --locked --target wasm32-wasip1 -- --test-threads=1
else
  skip "wasmtime missing: wasm32-wasip1 tests not run (CI gates them)"
fi

# CI's audit job runs on the push, which races the tag, and its verdict
# depends on the advisory database, not on our tree.
if command -v cargo-audit >/dev/null; then
  check "cargo audit (advisory database fetched now)" cargo audit --file Cargo.lock
else
  bad "cargo audit is not installed: cargo install cargo-audit --locked"
fi

MSRV=$(grep -m1 '^rust-version' Cargo.toml | cut -d'"' -f2)
if rustup run "$MSRV" rustc -V >/dev/null 2>&1; then
  check "compiles at the MSRV ($MSRV)" rustup run "$MSRV" cargo check -j "$JOBS" --locked
else
  skip "MSRV $MSRV not installed here (CI gates it)"
fi

# ------------------------------------------------------------------ release
# The binaries' archive copies these; a missing one fails the workflow late.
for f in LICENSE LICENSING.md README.md imagoverto-wasm/LICENSE imagoverto-wasm/README.md; do
  [ -f "$f" ] && ok "$f exists (goes in an archive or package)" || bad "$f missing"
done
cmp -s LICENSE imagoverto-wasm/LICENSE && ok "imagoverto-wasm/LICENSE is LICENSE" \
  || bad "imagoverto-wasm/LICENSE differs from LICENSE"
# Only the AVIF decoder's tests take zenravif from git, so the [patch] in
# Cargo.toml is ours alone: a user of the crate needs no [patch], and the
# README must not tell them to add one.
grep -q 'patch.crates-io' README.md && bad "README shows a [patch] line users do not need" \
  || ok "README asks for no [patch]"
cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | jq -e '.packages[] | select(.name == "imagoverto") | .dependencies[]
           | select(.source != null and (.source | startswith("git+")) and .kind != "dev")' >/dev/null \
  && bad "imagoverto has a git dependency outside [dev-dependencies]" \
  || ok "git dependencies are dev-only"

echo "  note: sweep README.md for stale numbers and claims before tagging"
echo
echo "$PASS passed, $FAIL failed, in $((SECONDS / 60)) min $((SECONDS % 60)) s"
if [ "$FAIL" -gt 0 ]; then
  echo "NOT READY:"; for f in "${FAILED[@]}"; do echo "  - $f"; done; exit 1
fi
echo "READY: git push origin main v$V"
