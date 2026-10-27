/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Screen geometry as a pure function of the terminal size and the UI state. No drawing here;
//! tests check the boundaries directly.

use ratatui::layout::Rect;

pub const MinWidth: u16 = 40;
pub const MinHeight: u16 = 12;
/// Below this width the sidebar is not docked; it opens as a drawer over the transcript.
pub const DockWidth: u16 = 100;
pub const SidebarWidth: u16 = 34;
pub const WideSidebarWidth: u16 = 38;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    Hidden,
    Docked,
    Drawer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct layout_input_t {
    pub width: u16,
    pub height: u16,
    /// The user's choice for a wide terminal.
    pub sidebar_pinned: bool,
    /// The drawer on a narrow terminal.
    pub drawer_open: bool,
    /// Rows the bottom dock wants: the composer, or an approval or question widget.
    pub dock_rows: u16,
    pub queue_rows: u16,
    /// Rows of the slash-command popup, 0 when closed.
    pub popup_rows: u16,
    pub help_open: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct layout_t {
    pub too_small: bool,
    pub area: Rect,
    pub main: Rect,
    pub transcript: Rect,
    pub scrollbar: Rect,
    pub queue: Rect,
    pub popup: Rect,
    pub dock: Rect,
    pub status: Rect,
    pub sidebar: Rect,
    pub sidebar_mode: Option<SidebarMode>,
    pub help: Rect,
}

impl layout_t {
    pub fn sidebar_mode(&self) -> SidebarMode {
        self.sidebar_mode.unwrap_or(SidebarMode::Hidden)
    }
}

pub fn compute(input: layout_input_t) -> layout_t {
    let area = Rect::new(0, 0, input.width, input.height);
    if input.width < MinWidth || input.height < MinHeight {
        return layout_t {
            too_small: true,
            area,
            ..layout_t::default()
        };
    }
    let wide = input.width >= DockWidth;
    let (mode, side_w) = if wide && input.sidebar_pinned {
        let w = if input.width >= 150 {
            WideSidebarWidth
        } else {
            SidebarWidth
        };
        (SidebarMode::Docked, w)
    } else if !wide && input.drawer_open {
        (
            SidebarMode::Drawer,
            SidebarWidth.min(input.width.saturating_sub(6)),
        )
    } else {
        (SidebarMode::Hidden, 0)
    };
    let main_w = match mode {
        SidebarMode::Docked => input.width.saturating_sub(side_w),
        _ => input.width,
    };
    let main = Rect::new(0, 0, main_w, input.height);
    let sidebar = match mode {
        SidebarMode::Hidden => Rect::default(),
        _ => Rect::new(input.width.saturating_sub(side_w), 0, side_w, input.height),
    };

    let status_h = 1u16;
    // The dock never takes more than half the screen; the transcript keeps at least 3 rows.
    let max_dock = (input.height / 2).max(3);
    let dock_h = input.dock_rows.clamp(2, max_dock);
    let queue_h = input.queue_rows.min(2);
    let below = status_h + dock_h + queue_h;
    let transcript_h = input.height.saturating_sub(below).max(3);
    let transcript = Rect::new(main.x, 0, main.width, transcript_h);
    let queue = Rect::new(main.x, transcript_h, main.width, queue_h);
    let dock_y = transcript_h.saturating_add(queue_h);
    let dock = Rect::new(main.x, dock_y, main.width, dock_h);
    let status = Rect::new(
        main.x,
        input.height.saturating_sub(status_h),
        main.width,
        status_h,
    );
    let scrollbar = Rect::new(
        transcript.right().saturating_sub(1),
        transcript.y,
        1,
        transcript.height,
    );
    let popup_h = input.popup_rows.min(transcript_h.saturating_sub(1));
    let popup = Rect::new(
        main.x.saturating_add(2),
        dock_y.saturating_sub(queue_h).saturating_sub(popup_h),
        main.width.saturating_sub(4).min(64),
        popup_h,
    );
    let help = if input.help_open {
        let w = input.width.saturating_sub(4).min(76);
        let h = input.height.saturating_sub(2).min(48);
        Rect::new(
            (input.width.saturating_sub(w)) / 2,
            (input.height.saturating_sub(h)) / 2,
            w,
            h,
        )
    } else {
        Rect::default()
    };
    layout_t {
        too_small: false,
        area,
        main,
        transcript,
        scrollbar,
        queue,
        popup,
        dock,
        status,
        sidebar,
        sidebar_mode: Some(mode),
        help,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(width: u16, height: u16) -> layout_input_t {
        layout_input_t {
            width,
            height,
            sidebar_pinned: true,
            drawer_open: false,
            dock_rows: 4,
            queue_rows: 0,
            popup_rows: 0,
            help_open: false,
        }
    }

    fn inside(inner: Rect, outer: Rect) -> bool {
        inner.width == 0
            || inner.height == 0
            || (inner.x >= outer.x
                && inner.y >= outer.y
                && inner.right() <= outer.right()
                && inner.bottom() <= outer.bottom())
    }

    #[test]
    fn too_small_boundaries() {
        assert!(compute(input(39, 11)).too_small);
        assert!(compute(input(39, 40)).too_small);
        assert!(compute(input(120, 11)).too_small);
        assert!(!compute(input(40, 12)).too_small);
        assert!(compute(input(0, 0)).too_small);
    }

    #[test]
    fn sidebar_docks_only_when_wide() {
        for (w, mode) in [
            (40, SidebarMode::Hidden),
            (59, SidebarMode::Hidden),
            (60, SidebarMode::Hidden),
            (99, SidebarMode::Hidden),
            (100, SidebarMode::Docked),
            (140, SidebarMode::Docked),
        ] {
            let l = compute(input(w, 30));
            assert_eq!(l.sidebar_mode(), mode, "width {w}");
        }
        let mut i = input(140, 30);
        i.sidebar_pinned = false;
        assert_eq!(compute(i).sidebar_mode(), SidebarMode::Hidden);
    }

    #[test]
    fn drawer_on_narrow() {
        let mut i = input(60, 20);
        i.drawer_open = true;
        let l = compute(i);
        assert_eq!(l.sidebar_mode(), SidebarMode::Drawer);
        assert_eq!(l.main.width, 60);
        assert!(l.sidebar.width <= 54);
        // The drawer has no effect on a wide terminal; the docked sidebar follows the pin.
        let mut i = input(120, 20);
        i.drawer_open = true;
        i.sidebar_pinned = false;
        assert_eq!(compute(i).sidebar_mode(), SidebarMode::Hidden);
    }

    #[test]
    fn rects_stay_inside_the_screen() {
        for w in [40u16, 59, 60, 80, 99, 100, 140, 300, u16::MAX] {
            for h in [12u16, 13, 20, 50, u16::MAX] {
                for dock in [0u16, 2, 8, 200, u16::MAX] {
                    let mut i = input(w, h);
                    i.dock_rows = dock;
                    i.queue_rows = 1;
                    i.popup_rows = 9;
                    i.help_open = true;
                    let l = compute(i);
                    for r in [
                        l.main,
                        l.transcript,
                        l.queue,
                        l.dock,
                        l.status,
                        l.sidebar,
                        l.popup,
                        l.help,
                        l.scrollbar,
                    ] {
                        assert!(inside(r, l.area), "{r:?} in {:?}", l.area);
                    }
                    assert!(l.transcript.height >= 3);
                    assert_eq!(l.status.bottom(), h);
                    assert!(l.dock.bottom() <= l.status.y);
                }
            }
        }
    }

    #[test]
    fn docked_sidebar_and_main_do_not_overlap() {
        let l = compute(input(140, 40));
        assert_eq!(l.main.right(), l.sidebar.x);
        assert_eq!(l.sidebar.right(), 140);
    }
}
