#!/bin/sh
# Package the optional revision adapter and its examples without models or environments.
set -eu
if [ "$#" -ne 1 ]; then
  echo "usage: $0 OUTPUT_DIRECTORY" >&2
  exit 2
fi
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
out=$1
mkdir -p "$out"
out=$(CDPATH= cd -- "$out" && pwd)
version=$(awk '
  /^\[workspace\.package\]/ { in_section = 1; next }
  /^\[/ { in_section = 0 }
  in_section && /^version[[:space:]]*=/ {
    sub(/^[^=]*=[[:space:]]*/, ""); gsub(/"/, ""); print; exit
  }
' "$root/Cargo.toml")
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "invalid workspace version" >&2
  exit 1
fi
archive="$out/aexeo-revision-adapter-v$version.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$root" LICENSE \
  adapters/revision/README.md adapters/revision/example.toml \
  adapters/revision/example-parameters.json adapters/revision/example-fr.toml \
  adapters/revision/example-fr-parameters.json adapters/revision/combined-writing-review-fr.toml adapters/revision/review.py adapters/revision/test_review.py
printf '%s\n' "$archive"
