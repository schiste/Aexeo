#!/bin/sh
# Package the optional adapter and its examples without models or environments.
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
archive="$out/aexeo-syntax-adapter-v$version.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$root" LICENSE \
  adapters/syntax/README.md adapters/syntax/example.toml \
  adapters/syntax/example-parameters.json adapters/syntax/opening-pos-example.toml \
  adapters/syntax/opening-pos-parameters.json adapters/syntax/opening-pos-example-fr.toml \
  adapters/syntax/opening-pos-parameters-fr.json \
  adapters/syntax/opening-pos-sequence-example.toml adapters/syntax/opening-pos-sequence-parameters.json \
  adapters/syntax/opening-pos-sequence-example-fr.toml adapters/syntax/opening-pos-sequence-parameters-fr.json \
  adapters/syntax/clause-alternation-example.toml adapters/syntax/clause-alternation-parameters.json \
  adapters/syntax/clause-alternation-example-fr.toml adapters/syntax/clause-alternation-parameters-fr.json \
  adapters/syntax/coordinated-nouns-example.toml adapters/syntax/coordinated-nouns-parameters.json \
  adapters/syntax/coordinated-nouns-example-fr.toml adapters/syntax/coordinated-nouns-parameters-fr.json \
  adapters/syntax/shared-noun-markers-example.toml adapters/syntax/shared-noun-markers-parameters.json \
  adapters/syntax/shared-noun-markers-example-fr.toml adapters/syntax/shared-noun-markers-parameters-fr.json \
  adapters/syntax/review.py adapters/syntax/test_review.py
printf '%s\n' "$archive"
