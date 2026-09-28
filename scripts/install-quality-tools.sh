#!/bin/sh
set -eu

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

install_ripgrep

cargo install cargo-audit cargo-deny cargo-udeps
rustup toolchain install nightly
rustup component add --toolchain nightly rust-src llvm-tools-preview

printf 'quality tools: cargo-audit, cargo-deny, cargo-udeps installed; nightly toolchain with rust-src and llvm-tools-preview installed\n'
printf 'ripgrep (rg, required by scripts/guard-staged.sh): %s\n' "$ripgrep_status"
