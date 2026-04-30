#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: sh scripts/install-seogeo.sh
           [--from-binary PATH | --from-release vX.Y.Z]
           [--dest-dir DIR] [--binary-name NAME] [--no-smoke-test]

Installs a seogeo binary into a deterministic destination directory and
optionally runs a post-install smoke test.

Source modes (mutually exclusive):
  --from-binary PATH     Copy from a local path (default: target/release/seogeo-cli)
  --from-release TAG     Download the release asset for the host triple from
                         https://github.com/$SEOGEO_RELEASE_REPO/releases/tag/<TAG>
                         and verify its SHA256 against SHA256SUMS.txt.
                         $SEOGEO_RELEASE_REPO defaults to schiste/Aexeo.

Auth for --from-release:
  Uses gh CLI if available (gh auth must be active). Otherwise requires
  GITHUB_TOKEN in env, plus jq on PATH for asset id lookup.
EOF
}

REPO_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
SOURCE_BIN="$REPO_ROOT/target/release/seogeo-cli"
DEST_DIR="${SEOGEO_INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="seogeo-cli"
RUN_SMOKE_TEST=1
RELEASE_TAG=""
RELEASE_REPO="${SEOGEO_RELEASE_REPO:-schiste/Aexeo}"

while [ $# -gt 0 ]; do
    case "$1" in
        --from-binary)
            [ $# -ge 2 ] || { echo "missing value for --from-binary" >&2; exit 2; }
            SOURCE_BIN="$2"
            shift 2
            ;;
        --from-release)
            [ $# -ge 2 ] || { echo "missing value for --from-release" >&2; exit 2; }
            RELEASE_TAG="$2"
            shift 2
            ;;
        --dest-dir)
            [ $# -ge 2 ] || { echo "missing value for --dest-dir" >&2; exit 2; }
            DEST_DIR="$2"
            shift 2
            ;;
        --binary-name)
            [ $# -ge 2 ] || { echo "missing value for --binary-name" >&2; exit 2; }
            BINARY_NAME="$2"
            shift 2
            ;;
        --no-smoke-test)
            RUN_SMOKE_TEST=0
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

# When pulling from a release, the local SOURCE_BIN gets overwritten with
# the downloaded path further down. Refuse the conflicting combo so a
# stray --from-binary doesn't get silently ignored.
if [ -n "$RELEASE_TAG" ] && [ "$SOURCE_BIN" != "$REPO_ROOT/target/release/seogeo-cli" ]; then
    echo "--from-binary and --from-release cannot be combined" >&2
    exit 2
fi

detect_target() {
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    case "$(uname -m)" in
        arm64|aarch64) arch="arm64" ;;
        x86_64|amd64)  arch="x86_64" ;;
        *) echo "unsupported host arch: $(uname -m)" >&2; return 1 ;;
    esac
    printf '%s-%s' "$os" "$arch"
}

download_release_with_gh() {
    tag="$1"; target="$2"; out="$3"
    asset="seogeo-cli-$target"
    gh release download "$tag" \
        --repo "$RELEASE_REPO" \
        --pattern "$asset" \
        --pattern "SHA256SUMS.txt" \
        --dir "$out"
}

download_release_with_curl() {
    tag="$1"; target="$2"; out="$3"
    asset="seogeo-cli-$target"
    : "${GITHUB_TOKEN:?GITHUB_TOKEN required when gh CLI is unavailable}"
    command -v jq >/dev/null || { echo "jq required for curl-based release fetch" >&2; return 2; }

    api="https://api.github.com/repos/$RELEASE_REPO/releases/tags/$tag"
    release_json=$(curl -fsSL \
        -H "Authorization: Bearer $GITHUB_TOKEN" \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "$api")

    for name in "$asset" "SHA256SUMS.txt"; do
        asset_id=$(printf '%s' "$release_json" \
            | jq -r --arg n "$name" '.assets[] | select(.name == $n) | .id')
        [ -n "$asset_id" ] && [ "$asset_id" != "null" ] || {
            echo "asset not found in $tag: $name" >&2
            return 1
        }
        curl -fsSL \
            -H "Authorization: Bearer $GITHUB_TOKEN" \
            -H "Accept: application/octet-stream" \
            -o "$out/$name" \
            "https://api.github.com/repos/$RELEASE_REPO/releases/assets/$asset_id"
    done
}

if [ -n "$RELEASE_TAG" ]; then
    target=$(detect_target)
    asset="seogeo-cli-$target"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    if command -v gh >/dev/null 2>&1; then
        download_release_with_gh "$RELEASE_TAG" "$target" "$tmp"
    else
        download_release_with_curl "$RELEASE_TAG" "$target" "$tmp"
    fi

    # Verify the downloaded asset against the line for our target in
    # SHA256SUMS.txt. Refuse to proceed if the line is missing or the
    # checksum does not match.
    sums_line=$(grep -E "  $asset\$" "$tmp/SHA256SUMS.txt" || true)
    [ -n "$sums_line" ] || {
        echo "no SHA256SUMS.txt entry for $asset in release $RELEASE_TAG" >&2
        exit 1
    }
    ( cd "$tmp" && printf '%s\n' "$sums_line" | shasum -a 256 -c - >/dev/null )

    SOURCE_BIN="$tmp/$asset"
fi

if [ ! -f "$SOURCE_BIN" ]; then
    echo "source binary does not exist: $SOURCE_BIN" >&2
    exit 1
fi

mkdir -p "$DEST_DIR"
INSTALL_PATH="$DEST_DIR/$BINARY_NAME"
cp "$SOURCE_BIN" "$INSTALL_PATH"
chmod 0755 "$INSTALL_PATH"

if [ "$RUN_SMOKE_TEST" -eq 1 ]; then
    "$INSTALL_PATH" --help >/dev/null
fi

printf '%s\n' "$INSTALL_PATH"
