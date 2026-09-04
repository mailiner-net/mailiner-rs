//! Adaptive IMAP partial-FETCH window for attachment streams.

use std::time::Duration;

/// Monotonic-enough mark for one FETCH window. `std::time::Instant` is
/// unimplemented on `wasm32-unknown-unknown`.
pub(crate) fn fetch_now() -> web_time::Instant {
    web_time::Instant::now()
}

/// First / floor window. Cheap first RTT; progress appears quickly.
pub(crate) const MIN_CHUNK: usize = 64 * 1024;
/// Ceiling so one FETCH literal stays within a few MiB of WASM heap.
pub(crate) const MAX_CHUNK: usize = 2 * 1024 * 1024;
/// Size the next window so a steady link spends about this long per FETCH.
const TARGET_MS: f64 = 250.0;

/// Per-stream FETCH length: start small, grow when fast, shrink when slow.
#[derive(Debug, Clone)]
pub(crate) struct FetchChunkSizer {
    size: usize,
}

impl Default for FetchChunkSizer {
    fn default() -> Self {
        Self { size: MIN_CHUNK }
    }
}

impl FetchChunkSizer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    /// Update from a **full** window (`bytes` == requested length).
    ///
    /// Faster than half of [`TARGET_MS`] doubles the window (good throughput,
    /// RTT is cheap relative to payload). Slower than twice the target halves
    /// it. In between, ease toward `throughput * TARGET_MS`.
    pub(crate) fn record(&mut self, bytes: usize, elapsed: Duration) {
        if bytes == 0 {
            return;
        }
        let ms = (elapsed.as_secs_f64() * 1000.0).max(1.0);
        let ideal = ((bytes as f64) * TARGET_MS / ms).round() as usize;

        let next = if ms < TARGET_MS * 0.5 {
            self.size.saturating_mul(2)
        } else if ms > TARGET_MS * 2.0 {
            self.size / 2
        } else {
            self.size / 2 + ideal / 2
        };

        self.size = next.clamp(MIN_CHUNK, MAX_CHUNK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizer_at(size: usize) -> FetchChunkSizer {
        FetchChunkSizer {
            size: size.clamp(MIN_CHUNK, MAX_CHUNK),
        }
    }

    #[test]
    fn starts_at_min() {
        assert_eq!(FetchChunkSizer::new().size(), MIN_CHUNK);
    }

    #[test]
    fn fast_window_doubles() {
        let mut s = FetchChunkSizer::new();
        s.record(MIN_CHUNK, Duration::from_millis(10));
        assert_eq!(s.size(), MIN_CHUNK * 2);
    }

    #[test]
    fn repeated_fast_windows_reach_max_and_stay() {
        let mut s = FetchChunkSizer::new();
        for _ in 0..16 {
            let n = s.size();
            s.record(n, Duration::from_millis(1));
        }
        assert_eq!(s.size(), MAX_CHUNK);
        s.record(MAX_CHUNK, Duration::from_millis(1));
        assert_eq!(s.size(), MAX_CHUNK);
    }

    #[test]
    fn slow_window_halves() {
        let mut s = sizer_at(MAX_CHUNK);
        s.record(MAX_CHUNK, Duration::from_millis(2_000));
        assert_eq!(s.size(), MAX_CHUNK / 2);
    }

    #[test]
    fn repeated_slow_windows_reach_min_and_stay() {
        let mut s = sizer_at(MAX_CHUNK);
        for _ in 0..16 {
            let n = s.size();
            s.record(n, Duration::from_secs(5));
        }
        assert_eq!(s.size(), MIN_CHUNK);
        s.record(MIN_CHUNK, Duration::from_secs(5));
        assert_eq!(s.size(), MIN_CHUNK);
    }

    #[test]
    fn mid_band_eases_toward_throughput_target() {
        // 256 KiB in 250 ms → ~1 MiB/s → ideal = 256 KiB. Stay put.
        let mut s = sizer_at(256 * 1024);
        s.record(256 * 1024, Duration::from_millis(250));
        assert_eq!(s.size(), 256 * 1024);

        // Same size in 200 ms (still in-band): ideal ≈ 320 KiB, blend to 288.
        s.record(256 * 1024, Duration::from_millis(200));
        assert_eq!(s.size(), 256 * 1024 / 2 + 320 * 1024 / 2);
    }

    #[test]
    fn empty_sample_is_ignored() {
        let mut s = FetchChunkSizer::new();
        s.record(0, Duration::from_millis(1));
        assert_eq!(s.size(), MIN_CHUNK);
    }

    #[test]
    fn sub_millisecond_counts_as_fast() {
        let mut s = FetchChunkSizer::new();
        s.record(MIN_CHUNK, Duration::from_nanos(1));
        assert_eq!(s.size(), MIN_CHUNK * 2);
    }
}
