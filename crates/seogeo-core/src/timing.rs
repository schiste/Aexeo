// Wasm-safe wrappers around std::time so the static evaluator and
// profiling helpers run on wasm32-unknown-unknown. That target has no
// platform clock; std::time::Instant::now and std::time::SystemTime::now
// trap. Profiling timings are best-effort, so on wasm32 these return
// zero and findings still produce.

#[cfg(not(target_arch = "wasm32"))]
pub use native::{Started, unix_seconds};

#[cfg(target_arch = "wasm32")]
pub use stub::{Started, unix_seconds};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    pub struct Started(Instant);

    impl Started {
        pub fn now() -> Self {
            Self(Instant::now())
        }

        pub fn elapsed_us(&self) -> u64 {
            self.0.elapsed().as_micros() as u64
        }
    }

    pub fn unix_seconds() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0)
    }
}

#[cfg(target_arch = "wasm32")]
mod stub {
    pub struct Started;

    impl Started {
        pub fn now() -> Self {
            Self
        }

        pub fn elapsed_us(&self) -> u64 {
            0
        }
    }

    pub fn unix_seconds() -> u64 {
        0
    }
}
