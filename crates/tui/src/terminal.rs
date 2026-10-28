/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Terminal setup and restore. The guard restores the terminal exactly once: on a normal
//! return, on an error return (both through `Drop`), and on a panic (through a hook that runs
//! before unwinding and then calls the previous hook). The sequences are written through a
//! generic writer, so tests check them against an in-memory buffer.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::queue;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Write the sequences that set up the screen: alternate screen, bracketed paste, hidden
/// cursor, and mouse capture when asked.
pub fn write_enter<W: Write>(w: &mut W, mouse: bool) -> io::Result<()> {
    queue!(w, EnterAlternateScreen, EnableBracketedPaste, Hide)?;
    if mouse {
        queue!(w, EnableMouseCapture)?;
    }
    w.flush()
}

/// Write the sequences that undo [`write_enter`], in reverse order.
pub fn write_leave<W: Write>(w: &mut W, mouse: bool) -> io::Result<()> {
    if mouse {
        queue!(w, DisableMouseCapture)?;
    }
    queue!(w, DisableBracketedPaste, LeaveAlternateScreen, Show)?;
    w.flush()
}

/// Something that can put a terminal into the UI's mode and back.
pub trait terminal_control_t: Send {
    fn enter(&mut self) -> io::Result<()>;
    fn leave(&mut self) -> io::Result<()>;
}

/// The real terminal on stdout.
pub struct crossterm_control_t {
    pub mouse: bool,
}

impl terminal_control_t for crossterm_control_t {
    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        write_enter(&mut io::stdout(), self.mouse)
    }

    fn leave(&mut self) -> io::Result<()> {
        let written = write_leave(&mut io::stdout(), self.mouse);
        let raw = disable_raw_mode();
        written.and(raw)
    }
}

/// Shared between the guard and the panic hook.
pub struct restore_state_t<C: terminal_control_t> {
    control: Mutex<C>,
    done: AtomicBool,
}

impl<C: terminal_control_t> restore_state_t<C> {
    /// Leave the UI mode once. Later calls do nothing.
    pub fn restore(&self) -> io::Result<()> {
        if self.done.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        match self.control.try_lock() {
            Ok(mut c) => c.leave(),
            Err(TryLockError::Poisoned(p)) => p.into_inner().leave(),
            // Another thread holds the control mid-call; it cannot be restored from here.
            Err(TryLockError::WouldBlock) => Ok(()),
        }
    }

    pub fn restored(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }
}

pub struct terminal_guard_t<C: terminal_control_t + 'static> {
    state: Arc<restore_state_t<C>>,
}

impl<C: terminal_control_t + 'static> terminal_guard_t<C> {
    /// Enter the UI mode. If entering fails part way, what was done is undone.
    pub fn enter(mut control: C) -> io::Result<Self> {
        if let Err(err) = control.enter() {
            let _ = control.leave();
            return Err(err);
        }
        Ok(terminal_guard_t {
            state: Arc::new(restore_state_t {
                control: Mutex::new(control),
                done: AtomicBool::new(false),
            }),
        })
    }

    pub fn restore(&self) -> io::Result<()> {
        self.state.restore()
    }

    /// Restore the terminal before any panic message is printed, then run the previous hook.
    pub fn install_panic_hook(&self) {
        let state = Arc::clone(&self.state);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = state.restore();
            previous(info);
        }));
    }

    pub fn state(&self) -> &restore_state_t<C> {
        &self.state
    }
}

impl<C: terminal_control_t + 'static> Drop for terminal_guard_t<C> {
    fn drop(&mut self) {
        let _ = self.state.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone, Default)]
    struct recording_t {
        bytes: Arc<Mutex<Vec<u8>>>,
        enters: Arc<AtomicUsize>,
        leaves: Arc<AtomicUsize>,
    }

    struct recording_control_t {
        rec: recording_t,
        fail_enter: bool,
    }

    impl terminal_control_t for recording_control_t {
        fn enter(&mut self) -> io::Result<()> {
            self.rec.enters.fetch_add(1, Ordering::SeqCst);
            if self.fail_enter {
                return Err(io::Error::other("no terminal"));
            }
            let mut b = self
                .rec
                .bytes
                .lock()
                .map_err(|_| io::Error::other("lock"))?;
            write_enter(&mut *b, true)
        }

        fn leave(&mut self) -> io::Result<()> {
            self.rec.leaves.fetch_add(1, Ordering::SeqCst);
            let mut b = self
                .rec
                .bytes
                .lock()
                .map_err(|_| io::Error::other("lock"))?;
            write_leave(&mut *b, true)
        }
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn sequences_enter_and_leave() {
        let mut enter = Vec::new();
        write_enter(&mut enter, true).ok();
        assert!(contains(&enter, b"\x1b[?1049h"));
        assert!(contains(&enter, b"\x1b[?2004h"));
        assert!(contains(&enter, b"\x1b[?25l"));
        assert!(contains(&enter, b"\x1b[?1000h"));
        let mut leave = Vec::new();
        write_leave(&mut leave, true).ok();
        assert!(contains(&leave, b"\x1b[?1049l"));
        assert!(contains(&leave, b"\x1b[?2004l"));
        assert!(contains(&leave, b"\x1b[?25h"));
        assert!(contains(&leave, b"\x1b[?1000l"));
        let mut no_mouse = Vec::new();
        write_enter(&mut no_mouse, false).ok();
        assert!(!contains(&no_mouse, b"\x1b[?1000h"));
    }

    #[test]
    fn drop_restores_exactly_once() {
        let rec = recording_t::default();
        {
            let guard = terminal_guard_t::enter(recording_control_t {
                rec: rec.clone(),
                fail_enter: false,
            });
            assert!(guard.is_ok());
        }
        assert_eq!(rec.leaves.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn explicit_restore_is_idempotent() {
        let rec = recording_t::default();
        let guard = terminal_guard_t::enter(recording_control_t {
            rec: rec.clone(),
            fail_enter: false,
        });
        if let Ok(g) = guard {
            assert!(g.restore().is_ok());
            assert!(g.restore().is_ok());
            assert!(g.state().restored());
        }
        assert_eq!(rec.leaves.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_enter_undoes_and_errors() {
        let rec = recording_t::default();
        let guard = terminal_guard_t::enter(recording_control_t {
            rec: rec.clone(),
            fail_enter: true,
        });
        assert!(guard.is_err());
        assert_eq!(rec.leaves.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn panic_restores_once_and_chains_the_previous_hook() {
        let rec = recording_t::default();
        let rec_inner = rec.clone();
        let previous_calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&previous_calls);
        let old = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let result = std::panic::catch_unwind(move || {
            let guard = terminal_guard_t::enter(recording_control_t {
                rec: rec_inner,
                fail_enter: false,
            });
            if let Ok(g) = &guard {
                g.install_panic_hook();
            }
            panic!("boom inside the UI");
        });
        // Put the default hook back before asserting, so a failure prints normally.
        let _ours = std::panic::take_hook();
        std::panic::set_hook(old);
        assert!(result.is_err());
        assert_eq!(rec.leaves.load(Ordering::SeqCst), 1);
        assert!(previous_calls.load(Ordering::SeqCst) >= 1);
        let bytes = rec.bytes.lock().map(|b| b.clone()).unwrap_or_default();
        assert!(contains(&bytes, b"\x1b[?1049l"));
    }
}
