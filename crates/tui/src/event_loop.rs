/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The terminal loop: poll input with a short timeout, pump every session, draw, write any
//! queued terminal bytes, repeat. Animation time is the loop's monotonic clock, handed to the
//! app explicitly; drawing never reads the time itself.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event;
use harness::api::runtime_t;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

use crate::TuiError;
use crate::app::{app_options_t, app_t};
use crate::draw::draw;
use crate::terminal::{crossterm_control_t, terminal_guard_t};

/// Frame time while something animates or a session works.
const BusyTick: Duration = Duration::from_millis(16);
/// Frame time while everything is idle.
const IdleTick: Duration = Duration::from_millis(100);
/// Most events handled before the next frame, so a paste storm cannot starve drawing.
const MaxEventsPerFrame: usize = 256;

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn io_error(err: io::Error) -> TuiError {
    TuiError::Io(err.to_string())
}

/// Clear the screen and make the next draw send every cell. `Terminal::clear` would ask the
/// terminal for the cursor position first, which some terminals never answer; a resize to the
/// current size clears without that query.
fn full_redraw<W: Write>(terminal: &mut Terminal<CrosstermBackend<W>>) -> io::Result<()> {
    let size = terminal.size()?;
    terminal.resize(Rect::new(0, 0, size.width, size.height))
}

pub fn run_loop(runtime: Arc<dyn runtime_t>, options: app_options_t) -> Result<(), TuiError> {
    let start = Instant::now();
    let mut app = app_t::new(runtime, options.clone(), 0, wall_ms())
        .map_err(|e| TuiError::Session(e.message))?;
    let guard = terminal_guard_t::enter(crossterm_control_t {
        mouse: options.mouse,
    })
    .map_err(io_error)?;
    guard.install_panic_hook();
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).map_err(io_error)?;
    full_redraw(&mut terminal).map_err(io_error)?;

    let result = (|| -> Result<(), TuiError> {
        loop {
            let clock = start.elapsed().as_millis() as u64;
            app.tick(clock, wall_ms());
            if app.redraw {
                app.redraw = false;
                full_redraw(&mut terminal).map_err(io_error)?;
            }
            terminal
                .draw(|frame| draw(&mut app, frame))
                .map_err(io_error)?;
            if !app.outputs.is_empty() {
                let backend = terminal.backend_mut();
                for bytes in app.outputs.drain(..) {
                    backend.write_all(&bytes).map_err(io_error)?;
                }
                backend.flush().map_err(io_error)?;
            }
            if app.quit {
                // Empty conversations still open are deleted, not left as empty logs.
                app.shutdown();
                return Ok(());
            }
            let animating = app.toast_animating()
                || app.busy()
                || app.sessions.iter().any(|s| s.busy())
                || app.active().is_some_and(|s| s.view.any_live(&s.transcript));
            let wait = if animating { BusyTick } else { IdleTick };
            if event::poll(wait).map_err(io_error)? {
                let mut handled = 0usize;
                loop {
                    let ev = event::read().map_err(io_error)?;
                    app.handle_event(&ev);
                    handled += 1;
                    if handled >= MaxEventsPerFrame
                        || !event::poll(Duration::ZERO).map_err(io_error)?
                    {
                        break;
                    }
                }
            }
        }
    })();
    let restored = guard.restore().map_err(io_error);
    result.and(restored)
}
