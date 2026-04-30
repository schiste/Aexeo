#!/bin/sh
# bootstrap-seogeo.sh — vendored bootstrap for the seogeo CLI.
#
# WHAT THIS DOES:
#   1. Reads .seogeo-version (caret or exact pin, e.g. "^0.7" or "v0.7.0").
#   2. Reads .seogeo-version.lock if it exists (resolved tag, e.g. "v0.7.4").
#   3. If lock is missing or no longer satisfies the constraint:
#        - in CI ($CI=true): fails. Re-run locally and commit the lock.
#        - locally:          re-resolves against the GitHub Releases API
#                            and writes a new lock.
#   4. Downloads the binary for $(uname) into $HOME/.cache/seogeo/<tag>/.
#   5. Verifies SHA256 against the release's SHA256SUMS.txt.
#   6. Prints the absolute path of the verified binary on stdout. Idempotent
#      across re-runs at the same tag.
#
# REQUIREMENTS:
#   $GITHUB_TOKEN — fine-grained PAT or GitHub App installation token with
#                   contents:read on the seogeo source repo. In CI, get it
#                   from actions/create-github-app-token@v1.
#   curl, jq, shasum — standard on macOS and Ubuntu runners.
#
# CONFIG:
#   $SEOGEO_RELEASE_REPO — defaults to schiste/Aexeo.
#   $SEOGEO_CACHE_DIR    — defaults to $HOME/.cache/seogeo.
#   $CI                  — when "true", refuses to mutate .seogeo-version.lock.
#
# BUMPING THE PIN:
#   1. Edit .seogeo-version (e.g. ^0.7 → ^0.8).
#   2. rm .seogeo-version.lock
#   3. ./scripts/bootstrap-seogeo.sh
#   4. git add .seogeo-version .seogeo-version.lock && commit.
set -eu

REPO="${SEOGEO_RELEASE_REPO:-schiste/Aexeo}"
CACHE_DIR="${SEOGEO_CACHE_DIR:-$HOME/.cache/seogeo}"
HERE=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
VERSION_FILE="$HERE/.seogeo-version"
LOCK_FILE="$HERE/.seogeo-version.lock"

err() { printf '%s\n' "$*" >&2; }
die() { err "bootstrap-seogeo: $*"; exit 1; }

[ -f "$VERSION_FILE" ] || die "missing $VERSION_FILE; create it with a constraint like ^0.7"

CONSTRAINT=$(tr -d '[:space:]' <"$VERSION_FILE")
[ -n "$CONSTRAINT" ] || die ".seogeo-version is empty"

# Reject syntaxes we don't support so users get a clear error instead of a
# silent fall-through. Caret + exact only.
case "$CONSTRAINT" in
    \^*|v[0-9]*|[0-9]*) ;;
    \~*)
        without_tilde=$(printf '%s' "$CONSTRAINT" | sed 's/^~//')
        die "tilde constraints (~) are not supported; use caret (e.g. ^${without_tilde}) instead"
        ;;
    \>*|\<*|=*) die "range constraints are not supported; use caret (^X.Y) or exact (vX.Y.Z)" ;;
    *) die "unrecognized constraint '$CONSTRAINT'; use caret (^X.Y) or exact (vX.Y.Z)" ;;
esac

# satisfies CONSTRAINT TAG → 0 if TAG satisfies CONSTRAINT, 1 otherwise.
# Implements caret semantics matching cargo: for 0.x, ^0.X.Y allows
# 0.X.>=Y (minor pins major-zero releases). For >=1.x, ^X.Y.Z allows
# anything <(X+1).0.0 and >=X.Y.Z.
satisfies() {
    sat_c="$1"; sat_v="$2"
    sat_v="${sat_v#v}"
    sat_v_major=$(printf '%s' "$sat_v" | cut -d. -f1)
    sat_v_minor=$(printf '%s' "$sat_v" | cut -d. -f2)
    sat_v_patch=$(printf '%s' "$sat_v" | cut -d. -f3)
    case "$sat_c" in
        \^*)
            sat_cc="${sat_c#^}"
            sat_c_major=$(printf '%s' "$sat_cc" | cut -d. -f1)
            sat_c_minor=$(printf '%s' "$sat_cc" | cut -d. -f2)
            sat_c_patch=$(printf '%s' "$sat_cc" | cut -d. -f3)
            [ -n "$sat_c_minor" ] || sat_c_minor=0
            [ -n "$sat_c_patch" ] || sat_c_patch=0
            if [ "$sat_c_major" = "0" ]; then
                # ^0.M(.P) allows 0.M.>=P only.
                [ "$sat_v_major" = "0" ] && \
                [ "$sat_v_minor" = "$sat_c_minor" ] && \
                [ "$sat_v_patch" -ge "$sat_c_patch" ]
            else
                # ^M.N(.P) allows >=M.N.P, <(M+1).0.0.
                [ "$sat_v_major" = "$sat_c_major" ] || return 1
                if [ "$sat_v_minor" -gt "$sat_c_minor" ]; then return 0; fi
                if [ "$sat_v_minor" -lt "$sat_c_minor" ]; then return 1; fi
                [ "$sat_v_patch" -ge "$sat_c_patch" ]
            fi
            ;;
        v*|[0-9]*)
            sat_cv="${sat_c#v}"
            [ "$sat_cv" = "$sat_v" ]
            ;;
        *) return 1 ;;
    esac
}

# Determine the tag we'll install.
TAG=""
if [ -f "$LOCK_FILE" ]; then
    LOCKED=$(tr -d '[:space:]' <"$LOCK_FILE")
    if [ -n "$LOCKED" ] && satisfies "$CONSTRAINT" "$LOCKED"; then
        TAG="$LOCKED"
    fi
fi

if [ -z "$TAG" ]; then
    if [ "${CI:-}" = "true" ]; then
        die "lockfile $LOCK_FILE missing or stale for constraint '$CONSTRAINT'; run bootstrap-seogeo.sh locally and commit the updated lock"
    fi

    : "${GITHUB_TOKEN:?GITHUB_TOKEN required to resolve $CONSTRAINT against $REPO releases}"
    command -v jq >/dev/null || die "jq required to resolve constraint"
    command -v curl >/dev/null || die "curl required to resolve constraint"

    err "resolving $CONSTRAINT against $REPO releases…"
    # First page only — fine for the first 30 releases. Add pagination if
    # the release count grows past that.
    RELEASES=$(curl -fsSL \
        -H "Authorization: Bearer $GITHUB_TOKEN" \
        -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/$REPO/releases?per_page=100")

    # All tags newest-first.
    CANDIDATES=$(printf '%s' "$RELEASES" | jq -r '.[].tag_name')
    [ -n "$CANDIDATES" ] || die "no releases found in $REPO"

    # Filter to satisfying versions, then pick the highest by sort -V.
    BEST=""
    # shellcheck disable=SC2034
    OLDIFS="$IFS"; IFS='
'
    for c in $CANDIDATES; do
        if satisfies "$CONSTRAINT" "$c"; then
            if [ -z "$BEST" ]; then
                BEST="$c"
            else
                BEST=$(printf '%s\n%s\n' "$BEST" "$c" | sort -V | tail -n1)
            fi
        fi
    done
    IFS="$OLDIFS"

    [ -n "$BEST" ] || die "no published release of $REPO satisfies constraint '$CONSTRAINT'"
    TAG="$BEST"
    printf '%s\n' "$TAG" >"$LOCK_FILE"
    err "wrote $LOCK_FILE → $TAG"
fi

# Resolve target triple in the same vocabulary as the release assets.
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
case "$(uname -m)" in
    arm64|aarch64) ARCH="arm64" ;;
    x86_64|amd64)  ARCH="x86_64" ;;
    *) die "unsupported host arch: $(uname -m)" ;;
esac
TARGET="$OS-$ARCH"
ASSET="seogeo-cli-$TARGET"

INSTALL_DIR="$CACHE_DIR/$TAG"
INSTALL_BIN="$INSTALL_DIR/seogeo-cli"

if [ -x "$INSTALL_BIN" ]; then
    # Already installed at this tag. Re-runs are no-ops.
    printf '%s\n' "$INSTALL_BIN"
    exit 0
fi

: "${GITHUB_TOKEN:?GITHUB_TOKEN required to download $ASSET from $REPO@$TAG}"
command -v jq >/dev/null || die "jq required for asset id lookup"
command -v curl >/dev/null || die "curl required for download"

mkdir -p "$INSTALL_DIR"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

err "downloading $ASSET from $REPO@$TAG…"
RELEASE_JSON=$(curl -fsSL \
    -H "Authorization: Bearer $GITHUB_TOKEN" \
    -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/$REPO/releases/tags/$TAG")

for name in "$ASSET" "SHA256SUMS.txt"; do
    asset_id=$(printf '%s' "$RELEASE_JSON" \
        | jq -r --arg n "$name" '.assets[] | select(.name == $n) | .id')
    [ -n "$asset_id" ] && [ "$asset_id" != "null" ] \
        || die "asset $name not present in release $TAG"
    curl -fsSL \
        -H "Authorization: Bearer $GITHUB_TOKEN" \
        -H "Accept: application/octet-stream" \
        -o "$TMP/$name" \
        "https://api.github.com/repos/$REPO/releases/assets/$asset_id"
done

SUMS_LINE=$(grep -E "  $ASSET\$" "$TMP/SHA256SUMS.txt" || true)
[ -n "$SUMS_LINE" ] || die "SHA256SUMS.txt has no entry for $ASSET in $TAG"
( cd "$TMP" && printf '%s\n' "$SUMS_LINE" | shasum -a 256 -c - >/dev/null ) \
    || die "checksum verification failed for $ASSET"

cp "$TMP/$ASSET" "$INSTALL_BIN"
chmod 0755 "$INSTALL_BIN"

# Smoke-test before declaring success so a corrupt download doesn't poison
# the cache silently.
"$INSTALL_BIN" --help >/dev/null || die "smoke test failed for $INSTALL_BIN"

printf '%s\n' "$INSTALL_BIN"
