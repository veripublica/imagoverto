#!/usr/bin/env bash
#
# After a release: verify from the artefacts, not the workflows. Read-only:
# it downloads into a temporary directory and changes nothing anywhere.
#
# Usage: scripts/release-verify.sh 0.1.0
#
# Prints one line per item (ok / FAIL) and a verdict; exit 0 only if all pass.

set -uo pipefail
cd "$(dirname "$0")/.."

V="${1:?usage: scripts/release-verify.sh X.Y.Z}"
REPO=veripublica/imagoverto
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
PASS=0; FAIL=0
ok()  { echo "  ok    $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL  $1"; FAIL=$((FAIL + 1)); }

# 8 archives + SHA256SUMS.txt.
N=$(gh release view "v$V" --repo "$REPO" --json assets -q '.assets | length' 2>/dev/null || echo 0)
[ "$N" = 9 ] && ok "release v$V has 9 assets" || bad "release v$V has $N assets, not 9"
BODY=$(gh release view "v$V" --repo "$REPO" --json body -q .body 2>/dev/null || echo "")
[ -n "$BODY" ] && ok "release v$V has a body" || bad "release v$V has an empty body"
LATEST=$(gh release view --repo "$REPO" --json tagName -q .tagName 2>/dev/null || echo "")
[ "$LATEST" = "v$V" ] && ok "v$V is the latest release" || bad "latest release is '$LATEST', not v$V"

TRIPLE=$(rustc -vV | sed -n 's/^host: //p')
A="imagoverto-$TRIPLE.tar.gz"
if gh release download "v$V" --repo "$REPO" -D "$W" -p SHA256SUMS.txt -p "$A" 2>/dev/null; then
  (cd "$W" && shasum -a 256 -c SHA256SUMS.txt --ignore-missing >/dev/null 2>&1) \
    && ok "$A matches SHA256SUMS.txt" || bad "$A does not match SHA256SUMS.txt"
  gh attestation verify "$W/$A" --repo "$REPO" >/dev/null 2>&1 \
    && ok "$A provenance attestation verifies" || bad "$A attestation does not verify"
  mkdir -p "$W/x" && tar xzf "$W/$A" -C "$W/x"
  for f in LICENSE LICENSING.md README.md; do
    [ -f "$W/x/imagoverto-$TRIPLE/$f" ] && ok "archive has $f" || bad "archive lacks $f"
  done
  BIN="$W/x/imagoverto-$TRIPLE/imagoverto"
  GOT=$("$BIN" --version 2>/dev/null || echo "no binary")
  if [[ "$GOT" == "imagoverto $V+"* && "$GOT" != *dirty* ]]; then ok "binary reports '$GOT'"
  else bad "binary reports '$GOT' (want imagoverto $V+<sha>, no .dirty)"; fi
else
  bad "could not download $A and SHA256SUMS.txt"
fi

# crates.io: the version is served, with the licence and no stray file.
UA="imagoverto-release-verify (github.com/veripublica/imagoverto)"
if curl -sfL -A "$UA" -o "$W/crate.tgz" "https://crates.io/api/v1/crates/imagoverto/$V/download"; then
  ok "crates.io serves imagoverto $V"
  LIST=$(tar tzf "$W/crate.tgz")
  for f in LICENSE LICENSING.md README.md Cargo.toml src/lib.rs; do
    grep -qx "imagoverto-$V/$f" <<< "$LIST" && ok "crate has $f" || bad "crate lacks $f"
  done
  grep -qE '/(examples|tests|scripts|imagoverto-wasm|\.github)/' <<< "$LIST" \
    && bad "crate carries files outside include" || ok "crate carries only the library"
else
  bad "crates.io does not serve imagoverto $V"
fi

# npm: the version, the latest tag, and the files a user imports.
NPM=@veripublica/imagoverto-wasm
if [ "$(npm view "$NPM@$V" version 2>/dev/null)" = "$V" ]; then
  ok "npm serves $NPM@$V"
  [ "$(npm view "$NPM" dist-tags.latest 2>/dev/null)" = "$V" ] \
    && ok "npm latest is $V" || bad "npm latest is not $V"
  if (cd "$W" && npm pack "$NPM@$V" --silent >/dev/null 2>&1); then
    PL=$(tar tzf "$W"/veripublica-imagoverto-wasm-"$V".tgz)
    for f in LICENSE imagoverto_wasm.d.ts imagoverto_wasm_bg.wasm package.json; do
      grep -qx "package/$f" <<< "$PL" && ok "npm package has $f" || bad "npm package lacks $f"
    done
  else
    bad "could not download $NPM@$V"
  fi
else
  bad "npm does not serve $NPM@$V"
fi

# The web page.
PAGE=$(curl -sfL https://veripublica.github.io/imagoverto/ || echo "")
grep -q "imagoverto" <<< "$PAGE" && ok "web page is up" || bad "web page is not up"

echo
echo "$PASS ok, $FAIL failed"
[ "$FAIL" = 0 ] && echo "VERIFIED: imagoverto $V" || { echo "NOT VERIFIED"; exit 1; }
