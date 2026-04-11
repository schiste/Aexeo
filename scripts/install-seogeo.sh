#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: sh scripts/install-seogeo.sh [--from-binary PATH] [--dest-dir DIR] [--binary-name NAME] [--no-smoke-test]

Installs a built seogeo binary into a deterministic destination directory and
optionally runs a post-install smoke test.
EOF
}

REPO_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
SOURCE_BIN="$REPO_ROOT/target/release/seogeo-cli"
DEST_DIR="${SEOGEO_INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="seogeo-cli"
RUN_SMOKE_TEST=1

while [ $# -gt 0 ]; do
    case "$1" in
        --from-binary)
            [ $# -ge 2 ] || {
                echo "missing value for --from-binary" >&2
                exit 2
            }
            SOURCE_BIN="$2"
            shift 2
            ;;
        --dest-dir)
            [ $# -ge 2 ] || {
                echo "missing value for --dest-dir" >&2
                exit 2
            }
            DEST_DIR="$2"
            shift 2
            ;;
        --binary-name)
            [ $# -ge 2 ] || {
                echo "missing value for --binary-name" >&2
                exit 2
            }
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
