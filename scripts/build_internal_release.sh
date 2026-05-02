#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: sh scripts/build_internal_release.sh [--target <os-arch>]

Builds the aexeo-cli release binary and stages it under dist/ with a
target-specific filename plus matching .sha256 file.

  --target <os-arch>   Output suffix. One of:
                         darwin-arm64
                         linux-x86_64
                       Defaults to the host triple if omitted (used by
                       local-dev release builds).

  --allow-cross        Override the host-vs-target safety check. Without
                       this, the script refuses when --target does not
                       match the host triple, because cargo will silently
                       produce a host-arch binary mislabeled with the
                       requested target suffix. Pass this only when you
                       are deliberately producing a mislabeled artifact
                       for testing the packaging pipeline itself.

Output:
  dist/aexeo-cli-<os-arch>
  dist/aexeo-cli-<os-arch>.sha256

The release workflow concatenates per-target .sha256 files into a single
SHA256SUMS.txt asset on the GitHub Release.
EOF
}

TARGET=""
ALLOW_CROSS=0

while [ $# -gt 0 ]; do
    case "$1" in
        --target)
            [ $# -ge 2 ] || { echo "missing value for --target" >&2; exit 2; }
            TARGET="$2"
            shift 2
            ;;
        --allow-cross)
            ALLOW_CROSS=1
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

# Resolve the host triple in the same vocabulary as our --target values.
HOST_OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$(uname -m)" in
    arm64|aarch64) HOST_ARCH="arm64" ;;
    x86_64|amd64)  HOST_ARCH="x86_64" ;;
    *)             HOST_ARCH="unknown" ;;
esac
HOST_TRIPLE="$HOST_OS-$HOST_ARCH"

if [ -z "$TARGET" ]; then
    TARGET="$HOST_TRIPLE"
fi

if [ "$TARGET" != "$HOST_TRIPLE" ] && [ "$ALLOW_CROSS" -ne 1 ]; then
    echo "build_internal_release.sh: refusing target=$TARGET on host=$HOST_TRIPLE" >&2
    echo "  (cross-compilation is not supported; run on a native runner," >&2
    echo "   or pass --allow-cross to deliberately produce a mislabeled artifact)" >&2
    exit 2
fi

# Release builds run the build directly. The full quality gate
# (cargo-audit, cargo-deny, cargo-udeps with nightly + rust-src) is a
# pre-merge concern and runs on ci.yml against PRs and main pushes; by
# the time a v* tag is pushed, the code at HEAD has already been gated
# there. Re-running the gate per release matrix job would add minutes
# of extra dependency installs on every runner without producing any
# additional signal.
cargo build --release

mkdir -p dist
OUT_BIN="dist/aexeo-cli-$TARGET"
cp target/release/aexeo-cli "$OUT_BIN"
shasum -a 256 "$OUT_BIN" > "$OUT_BIN.sha256"

# Smoke test the produced binary so a broken build never reaches a release.
"$OUT_BIN" --help >/dev/null

printf 'built: %s\nsha256: %s\n' "$OUT_BIN" "$(cat "$OUT_BIN.sha256")"
