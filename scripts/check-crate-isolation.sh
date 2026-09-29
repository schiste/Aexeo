#!/bin/sh
# Build every workspace crate in isolation.
#
# `cargo test --workspace` unifies features across all members, so a crate that
# declares a dependency with `default-features = false` and then uses something
# that only the *other* crates enable still builds fine in a workspace run. It
# fails on its own, which is where it matters: `cargo publish` builds per-crate,
# and so does any downstream consumer that depends on that one crate.
#
# This is not hypothetical. `aexeo-emdash-bridge` depends on `aexeo-core` with
# `default-features = false` so the WASM build has no networking. A `#[cfg]`
# attribute attached to the wrong module took the `net` feature gate off
# `aexeo-core`'s `runtime` module, `runtime` started requiring `reqwest`
# unconditionally, and the bridge stopped compiling by itself — while
# `cargo test --workspace` kept exiting 0 and reporting 596 passing tests.
#
# `cargo check -p` is used rather than `cargo test -p` so this stays a build
# check: the tests already run, together, in the workspace job above. This
# only asserts that each crate can be built without its siblings' features
# leaking in.
set -eu

. scripts/timing-lib.sh

prefix=${AEXEO_TIMINGS_SCOPE_PREFIX:-}

# Read crate names out of the workspace manifest rather than hardcoding a list,
# so a new member is covered the day it is added instead of the day someone
# remembers.
crates=$(
    sed -n '/^\[workspace\]/,/^\[/p' Cargo.toml \
        | sed -n 's/^[[:space:]]*"\(crates\/[^"]*\)".*/\1/p'
)

if [ -z "$crates" ]; then
    echo "could not read workspace members from Cargo.toml" >&2
    exit 1
fi

for crate in $crates; do
    name=$(basename "$crate")
    aexeo_run_timed "${prefix}cargo-check-${name}" "cache-sensitive" \
        cargo check -p "$name" --all-targets
done
