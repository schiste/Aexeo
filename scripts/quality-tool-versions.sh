# Versions of the tools behind the quality gates, sourced by
# install-quality-tools.sh, check-deps.sh and ci.yml so they cannot disagree.
#
# Dependabot does not track these. Bump them by hand and check that
# `sh scripts/check-deps.sh` still passes: cargo-udeps links against rustc
# nightly internals, so it and the nightly date generally move together.
AEXEO_NIGHTLY_TOOLCHAIN=nightly-2026-11-01
AEXEO_CARGO_AUDIT_VERSION=0.22.2
AEXEO_CARGO_DENY_VERSION=0.20.2
AEXEO_CARGO_UDEPS_VERSION=0.1.61
AEXEO_CARGO_LLVM_COV_VERSION=0.9.1
