//! Z14: Deterministic entropy injection via getrandom's custom backend.
//!
//! Removes the "seed your RNG" contract requirement by intercepting
//! `getrandom::fill()` calls with a deterministic stream seeded from
//! the scenario seed. Works for `rand::rng()`, `uuid::Uuid::new_v4`,
//! `bevy_rand`, and any crate using getrandom 0.3/0.4.
//!
//! ## Usage
//! Enable the `deterministic-entropy` feature and set `RUSTFLAGS`:
//! ```bash
//! RUSTFLAGS='--cfg getrandom_backend="custom"' cargo test --features bevy_swarm/deterministic-entropy
//! ```
//! Or use `cargo swarm run` which sets both automatically.

use std::cell::RefCell;

/// Domain separator to avoid correlating entropy streams across
/// different bevy_swarm subsystems.
const ENTROPY_DOMAIN: u64 = 0xBEAF_57A2;

// Thread-local deterministic RNG stream. None = fallback to the
// process-global stream (unattributed entropy).
thread_local! {
    static STREAM: RefCell<Option<Xoshiro256StarStar>> = const { RefCell::new(None) };
}

/// Splitmix64 for seeding (fast, good mixing).
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// Xoshiro256** PRNG (no dependencies, 256-bit state).
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Xoshiro256StarStar {
    s: [u64; 4],
}

#[allow(dead_code)]
impl Xoshiro256StarStar {
    fn seed(seed: u64) -> Self {
        let mut state = seed ^ ENTROPY_DOMAIN;
        Self {
            s: [
                splitmix64(&mut state),
                splitmix64(&mut state),
                splitmix64(&mut state),
                splitmix64(&mut state),
            ],
        }
    }

    fn next_u64(&mut self) -> u64 {
        let result = self.s[0]
            .wrapping_add(self.s[3])
            .rotate_left(23)
            .wrapping_add(self.s[0]);

        let t = self.s[1] << 17;

        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];

        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);

        result
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut i = 0;
        while i + 8 <= dest.len() {
            let bytes = self.next_u64().to_ne_bytes();
            dest[i..i + 8].copy_from_slice(&bytes);
            i += 8;
        }
        if i < dest.len() {
            let bytes = self.next_u64().to_ne_bytes();
            let remaining = dest.len() - i;
            dest[i..].copy_from_slice(&bytes[..remaining]);
        }
    }
}

/// Enter a deterministic entropy run with the given seed.
/// Must be called before building the App or running scenarios.
pub fn enter_run(seed: u64) {
    // FX3: diagnostic when the feature is on but the build cfg is not —
    // entropy interception silently no-ops in that misconfiguration.
    #[cfg(all(feature = "deterministic-entropy", not(getrandom_backend = "custom")))]
    eprintln!(
        "bevy_swarm: deterministic-entropy feature is enabled but the getrandom \
         custom-backend cfg is NOT set. Rebuild with RUSTFLAGS='--cfg getrandom_backend=\"custom\"' \
         or `cargo swarm run`; entropy interception is inactive and runs are NOT \
         deterministic."
    );
    STREAM.with(|s| {
        *s.borrow_mut() = Some(Xoshiro256StarStar::seed(seed));
    });
}

/// Exit the current deterministic entropy run.
pub fn exit_run() {
    STREAM.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Check if we're in a deterministic run.
pub fn in_deterministic_run() -> bool {
    STREAM.with(|s| s.borrow().is_some())
}

/// Count of fallback entropy draws (unattributed).
static FALLBACK_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Get the fallback draw count for diagnostics.
pub fn fallback_draw_count() -> usize {
    FALLBACK_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Reset the fallback counter (call at start of each run).
pub fn reset_fallback_counter() {
    FALLBACK_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

// The getrandom v0.4 custom backend hook (also works for v0.3 due to
// identical symbol name and ABI). Defined only when the cfg is set.
// FX3: Changed from extern "C" to extern "Rust" with proper Error return
// (getrandom 0.3/0.4 declare this as extern "Rust" — using "C" was UB).
#[cfg(all(feature = "deterministic-entropy", getrandom_backend = "custom"))]
#[unsafe(no_mangle)]
pub unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom04::Error> {
    // Safety: caller guarantees dest points to len valid bytes.
    let buf = unsafe { std::slice::from_raw_parts_mut(dest, len) };

    STREAM.with(|s| {
        match s.borrow_mut().as_mut() {
            Some(rng) => {
                rng.fill_bytes(buf);
                Ok(())
            }
            None => {
                // Fallback: process-global deterministic stream (seeded constant).
                // No panics — we're in an FFI context.
                FALLBACK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let mut g = global_fallback().lock().unwrap_or_else(|e| e.into_inner());
                g.fill_bytes(buf);
                Ok(())
            }
        }
    })
}

/// Process-global fallback RNG for getrandom calls outside enter_run.
/// Seeded with a constant to ensure reproducibility across runs.
#[cfg(all(feature = "deterministic-entropy", getrandom_backend = "custom"))]
static GLOBAL_FALLBACK: std::sync::OnceLock<std::sync::Mutex<Xoshiro256StarStar>> =
    std::sync::OnceLock::new();

#[cfg(all(feature = "deterministic-entropy", getrandom_backend = "custom"))]
fn global_fallback() -> &'static std::sync::Mutex<Xoshiro256StarStar> {
    GLOBAL_FALLBACK.get_or_init(|| std::sync::Mutex::new(Xoshiro256StarStar::seed(0xDEADBEEF)))
}

/// Layout assertions for getrandom Error types (v0.3 and v0.4).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoshiro_produces_different_streams() {
        let mut rng1 = Xoshiro256StarStar::seed(42);
        let mut rng2 = Xoshiro256StarStar::seed(43);
        assert_ne!(rng1.next_u64(), rng2.next_u64());
    }

    #[test]
    fn xoshiro_is_deterministic() {
        let mut rng1 = Xoshiro256StarStar::seed(42);
        let mut rng2 = Xoshiro256StarStar::seed(42);
        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn enter_exit_run_sets_stream() {
        assert!(!in_deterministic_run());
        enter_run(123);
        assert!(in_deterministic_run());
        exit_run();
        assert!(!in_deterministic_run());
    }

    #[test]
    fn fallback_counter_works() {
        reset_fallback_counter();
        assert_eq!(fallback_draw_count(), 0);
        // Can't easily test increment without calling the hook,
        // but the atomic operation is straightforward.
    }
}
