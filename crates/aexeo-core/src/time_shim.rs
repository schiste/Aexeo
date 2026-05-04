//! Wasm-safe `Instant`.
//!
//! On non-wasm targets this is a transparent re-export of
//! `std::time::Instant`. On `wasm32-unknown-unknown` — the target
//! the emdash bridge compiles to — `std::time::Instant::now()`
//! panics with "time not implemented on this platform" and the
//! WASM module traps with a bare `unreachable`. Cloudflare
//! Workers' `workerd` runtime appears to provide a clock shim
//! that masks the issue in production, but Node's experimental
//! WASM ESM runner (Astro 6.1.3 + Vite 7.3.1 SSR runner) does
//! not, so 0.8.7+ in `pnpm dev` traps on every refresh with
//! `wasm_error: unreachable`.
//!
//! The eval path uses `Instant` exclusively for diagnostic
//! timing (per-rule-group elapsed_us reported in
//! `SiteCheckProfile`). Returning `Duration::ZERO` on wasm
//! drops the timing detail but keeps the eval correct — and the
//! CLI / native tests still report real timings because they
//! run on non-wasm targets where the `std::time` re-export is
//! in effect.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(target_arch = "wasm32")]
pub use wasm_shim::Instant;

#[cfg(target_arch = "wasm32")]
mod wasm_shim {
    use std::time::Duration;

    /// No-op `Instant` for `wasm32-unknown-unknown`. Every method
    /// returns `Duration::ZERO`. Constructors are pure (no syscalls).
    #[derive(Copy, Clone, Debug, Default)]
    pub struct Instant(());

    impl Instant {
        pub fn now() -> Self {
            Self(())
        }

        pub fn elapsed(&self) -> Duration {
            Duration::ZERO
        }

        pub fn duration_since(&self, _earlier: Instant) -> Duration {
            Duration::ZERO
        }

        pub fn checked_duration_since(&self, _earlier: Instant) -> Option<Duration> {
            Some(Duration::ZERO)
        }

        pub fn saturating_duration_since(&self, _earlier: Instant) -> Duration {
            Duration::ZERO
        }
    }

    impl std::ops::Sub<Duration> for Instant {
        type Output = Instant;

        fn sub(self, _other: Duration) -> Self {
            self
        }
    }

    impl std::ops::Sub<Instant> for Instant {
        type Output = Duration;

        fn sub(self, _other: Instant) -> Duration {
            Duration::ZERO
        }
    }
}
