#!/bin/sh
set -eu

# --skip-ripgrep is for CI, which has no use for rg and no root to install it.
skip_ripgrep=0
case "${1:-}" in
    --skip-ripgrep) skip_ripgrep=1 ;;
    "") ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
esac

. scripts/quality-tool-versions.sh

# ripgrep is a hard requirement of scripts/guard-staged.sh. It is a system
# package rather than a cargo binary, so it is installed separately from the
# cargo tools below. This step is deliberately non-fatal: a machine with no
# recognised system package manager must not abort the rest of setup, and
# guard-staged.sh already fails closed with install instructions when rg is
# missing.
ripgrep_status="not attempted"

install_ripgrep() {
    if command -v rg >/dev/null 2>&1; then
        ripgrep_status="already present: $(rg --version 2>/dev/null | head -n 1 || echo 'version unknown')"
        return 0
    fi

    if command -v brew >/dev/null 2>&1; then
        echo "==> installing ripgrep with Homebrew (brew install ripgrep)"
        if brew install ripgrep; then
            ripgrep_status="installed via Homebrew"
        else
            ripgrep_status="FAILED: 'brew install ripgrep' returned non-zero"
        fi
    elif command -v apt-get >/dev/null 2>&1; then
        echo "==> installing ripgrep with apt-get (apt-get install -y ripgrep)"
        if apt-get install -y ripgrep; then
            ripgrep_status="installed via apt-get"
        else
            ripgrep_status="FAILED: 'apt-get install -y ripgrep' returned non-zero"
        fi
    else
        ripgrep_status="SKIPPED: no 'brew' or 'apt-get' found; install ripgrep manually (macOS: brew install ripgrep, Debian/Ubuntu: apt-get install -y ripgrep)"
        return 0
    fi

    # Homebrew installs outside the default PATH on some setups.
    if command -v rg >/dev/null 2>&1; then
        return 0
    fi
    if [ -x /opt/homebrew/bin/rg ]; then
        PATH="/opt/homebrew/bin:$PATH"
        export PATH
    elif [ -x /usr/local/bin/rg ]; then
        PATH="/usr/local/bin:$PATH"
        export PATH
    fi
}

if [ "$skip_ripgrep" -eq 0 ]; then
    install_ripgrep
fi

cargo install --locked \
    "cargo-audit@$AEXEO_CARGO_AUDIT_VERSION" \
    "cargo-deny@$AEXEO_CARGO_DENY_VERSION" \
    "cargo-udeps@$AEXEO_CARGO_UDEPS_VERSION"
rustup toolchain install "$AEXEO_NIGHTLY_TOOLCHAIN" --profile minimal --component rust-src --component llvm-tools-preview

printf 'quality tools: cargo-audit %s, cargo-deny %s, cargo-udeps %s installed; %s with rust-src and llvm-tools-preview installed\n' \
    "$AEXEO_CARGO_AUDIT_VERSION" "$AEXEO_CARGO_DENY_VERSION" "$AEXEO_CARGO_UDEPS_VERSION" "$AEXEO_NIGHTLY_TOOLCHAIN"
printf 'ripgrep (rg, required by scripts/guard-staged.sh): %s\n' "$ripgrep_status"
