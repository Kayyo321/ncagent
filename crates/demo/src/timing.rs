/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Simulated waiting: the level-pass and tool delays of `--speed` (PROJECT.md, "Demo backend").
//!
//! The delays are not measurements. `instant` skips them, and every test uses `instant`.

use std::thread;
use std::time::Duration;

use ncm::backend::cancel_token_t;

/// Longest single sleep, so a cancel is seen within this many milliseconds.
const SliceMs: u64 = 10;

/// Simulated delays, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct timing_t {
    /// Delay of one level pass.
    pub level_ms: u64,
    /// Delay of one simulated tool call.
    pub tool_ms: u64,
}

impl timing_t {
    /// `--speed normal`: 700 ms per level pass, 400 ms per tool.
    pub fn normal() -> Self {
        timing_t {
            level_ms: 700,
            tool_ms: 400,
        }
    }

    /// `--speed fast`: 150 ms per level pass, 100 ms per tool.
    pub fn fast() -> Self {
        timing_t {
            level_ms: 150,
            tool_ms: 100,
        }
    }

    /// `--speed instant`: no waiting.
    pub fn instant() -> Self {
        timing_t {
            level_ms: 0,
            tool_ms: 0,
        }
    }
}

/// Sleep for `ms` milliseconds in slices of at most 10 ms.
///
/// Returns `true` when the full time passed and the token was not set. Returns `false` as soon as
/// the token is seen set, which can be before the first slice.
pub fn sleep_checked(ms: u64, cancel: &cancel_token_t) -> bool {
    let mut remaining = ms;
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        if remaining == 0 {
            return true;
        }
        let slice = remaining.min(SliceMs);
        thread::sleep(Duration::from_millis(slice));
        remaining -= slice;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_match_the_speed_flag() {
        assert_eq!(
            timing_t::normal(),
            timing_t {
                level_ms: 700,
                tool_ms: 400
            }
        );
        assert_eq!(
            timing_t::fast(),
            timing_t {
                level_ms: 150,
                tool_ms: 100
            }
        );
        assert_eq!(
            timing_t::instant(),
            timing_t {
                level_ms: 0,
                tool_ms: 0
            }
        );
    }

    #[test]
    fn zero_delay_completes_without_waiting() {
        assert!(sleep_checked(0, &cancel_token_t::new()));
    }

    #[test]
    fn a_set_token_stops_before_sleeping() {
        let cancel = cancel_token_t::new();
        cancel.cancel();
        // The long delay is never entered, so this returns at once.
        assert!(!sleep_checked(60_000, &cancel));
    }
}
