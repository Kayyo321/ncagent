/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Keys and mouse events to app actions.
//!
//! Keys map through [`map_key`], a pure function of the key and a small context. Mouse events
//! map through the hit registry that the last frame drew, so every clickable thing is a
//! rectangle with an [`Action`], and a test can click it with a synthetic event. Every action a
//! click produces also has a key; the parity test in `app_tests` checks the table.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use harness::approval::ApprovalDecision;
use ratatui::layout::{Position, Rect};

use crate::composer::EditOp;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Focus {
    Input,
    Transcript,
    Sidebar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Edit(EditOp),
    Submit,
    /// A lone Escape. The app applies it only after a short delay with no other input.
    Escape,
    /// Ctrl+C: cancel when busy, otherwise clear the input or arm quit.
    Interrupt,
    Cancel,
    Quit,
    /// Ctrl+D: quit when the input is empty.
    QuitIfEmpty,
    ToggleSidebar,
    NewChat,
    CopyReply,
    Redraw,
    ToggleHelp,
    ToggleAnimation,
    ClearQueue,
    FocusNext,
    FocusPrev,
    SetFocus(Focus),
    /// Focus the input and type this character (from navigate mode).
    TypeInto(char),
    ScrollBy(i32),
    PageBy(i32),
    ScrollTop,
    JumpLatest,
    /// Mouse only: a press or drag on the scrollbar track. Keys reach the same positions with
    /// PageUp, PageDown, Home and End.
    ScrollbarTo {
        offset: u16,
        track: u16,
    },
    SelectBlock(i32),
    SelectBlockId(u64),
    ToggleSelected,
    ToggleBlock(u64),
    SidebarMove(i32),
    SidebarActivate,
    SidebarActivateAt(usize),
    SidebarScroll(i32),
    Approve(ApprovalDecision),
    ApprovalMove(i32),
    ApprovalConfirm,
    PickOption(usize),
    QuestionMove(i32),
    QuestionSubmit,
    QuestionEdit(EditOp),
    PopupMove(i32),
    PopupComplete,
    RunCommand(&'static str),
}

/// Which widget owns the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modal {
    None,
    Help,
    Approval,
    Question,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct key_ctx_t {
    pub focus: Focus,
    pub modal: Modal,
    pub input_empty: bool,
    pub popup_open: bool,
    pub answer_empty: bool,
    pub option_count: usize,
}

fn ctrl(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

fn alt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT)
}

/// Keys that work in every state.
fn global(key: &KeyEvent) -> Option<Action> {
    let c = ctrl(key);
    match key.code {
        KeyCode::Char('c') if c => Some(Action::Interrupt),
        KeyCode::Char('b') if c => Some(Action::ToggleSidebar),
        KeyCode::Char('n') if c => Some(Action::NewChat),
        KeyCode::Char('y') if c => Some(Action::CopyReply),
        KeyCode::Char('l') if c => Some(Action::Redraw),
        KeyCode::Char('t') if c => Some(Action::ToggleAnimation),
        KeyCode::Char('x') if c => Some(Action::ClearQueue),
        KeyCode::Esc => Some(Action::Escape),
        KeyCode::F(1) => Some(Action::ToggleHelp),
        KeyCode::PageUp => Some(Action::PageBy(-1)),
        KeyCode::PageDown => Some(Action::PageBy(1)),
        KeyCode::Home if c => Some(Action::ScrollTop),
        KeyCode::End if c => Some(Action::JumpLatest),
        _ => None,
    }
}

fn editing(key: &KeyEvent) -> Option<EditOp> {
    let c = ctrl(key);
    let a = alt(key);
    let op = match key.code {
        KeyCode::Char('j') if c => EditOp::Newline,
        KeyCode::Enter if a || key.modifiers.contains(KeyModifiers::SHIFT) => EditOp::Newline,
        KeyCode::Char('a') if c => EditOp::LineStart,
        KeyCode::Char('e') if c => EditOp::LineEnd,
        KeyCode::Char('u') if c => EditOp::KillToStart,
        KeyCode::Char('k') if c => EditOp::KillToEnd,
        KeyCode::Char('w') if c => EditOp::KillWordBack,
        KeyCode::Char('b') if a => EditOp::WordLeft,
        KeyCode::Char('f') if a => EditOp::WordRight,
        KeyCode::Backspace if a || c => EditOp::KillWordBack,
        KeyCode::Backspace => EditOp::Backspace,
        KeyCode::Delete => EditOp::Delete,
        KeyCode::Left if a || c => EditOp::WordLeft,
        KeyCode::Right if a || c => EditOp::WordRight,
        KeyCode::Left => EditOp::Left,
        KeyCode::Right => EditOp::Right,
        KeyCode::Home => EditOp::LineStart,
        KeyCode::End => EditOp::LineEnd,
        KeyCode::Up => EditOp::Up,
        KeyCode::Down => EditOp::Down,
        KeyCode::Char(ch) if !c && !a => EditOp::Insert(ch),
        _ => return None,
    };
    Some(op)
}

/// Map a key event to an action in the given context. Release events map to nothing.
pub fn map_key(key: &KeyEvent, ctx: &key_ctx_t) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if let Some(action) = global(key) {
        return Some(action);
    }
    let c = ctrl(key);
    match ctx.modal {
        Modal::Help => {
            return match key.code {
                KeyCode::Up | KeyCode::Char('k') => Some(Action::ScrollBy(-1)),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::ScrollBy(1)),
                _ => Some(Action::ToggleHelp),
            };
        }
        Modal::Approval => {
            return match key.code {
                KeyCode::Char('y') if !c => Some(Action::Approve(ApprovalDecision::Once)),
                KeyCode::Char('a') if !c => Some(Action::Approve(ApprovalDecision::Session)),
                KeyCode::Char('n') if !c => Some(Action::Approve(ApprovalDecision::Deny)),
                KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                    Some(Action::ApprovalMove(-1))
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => Some(Action::ApprovalMove(1)),
                KeyCode::Enter | KeyCode::Char(' ') => Some(Action::ApprovalConfirm),
                KeyCode::Up => Some(Action::ScrollBy(-1)),
                KeyCode::Down => Some(Action::ScrollBy(1)),
                KeyCode::Home => Some(Action::ScrollTop),
                KeyCode::End => Some(Action::JumpLatest),
                KeyCode::Char('?') => Some(Action::ToggleHelp),
                _ => None,
            };
        }
        Modal::Question => {
            return match key.code {
                KeyCode::Char(d @ '1'..='9') if !c && ctx.answer_empty => {
                    let index = (d as usize).saturating_sub('1' as usize);
                    if index < ctx.option_count {
                        Some(Action::PickOption(index))
                    } else {
                        Some(Action::QuestionEdit(EditOp::Insert(d)))
                    }
                }
                KeyCode::Up | KeyCode::BackTab => Some(Action::QuestionMove(-1)),
                KeyCode::Down | KeyCode::Tab => Some(Action::QuestionMove(1)),
                KeyCode::Enter if !alt(key) => Some(Action::QuestionSubmit),
                _ => editing(key).map(Action::QuestionEdit),
            };
        }
        Modal::None => {}
    }
    match ctx.focus {
        Focus::Input => {
            match key.code {
                KeyCode::Tab if ctx.popup_open => return Some(Action::PopupComplete),
                KeyCode::Up if ctx.popup_open => return Some(Action::PopupMove(-1)),
                KeyCode::Down if ctx.popup_open => return Some(Action::PopupMove(1)),
                KeyCode::Tab => return Some(Action::FocusNext),
                KeyCode::BackTab => return Some(Action::FocusPrev),
                KeyCode::Enter if !alt(key) && !key.modifiers.contains(KeyModifiers::SHIFT) => {
                    return Some(Action::Submit);
                }
                KeyCode::Char('d') if c => return Some(Action::QuitIfEmpty),
                KeyCode::Char('?') if ctx.input_empty => return Some(Action::ToggleHelp),
                _ => {}
            }
            editing(key).map(Action::Edit)
        }
        Focus::Transcript => match key.code {
            KeyCode::Tab => Some(Action::FocusNext),
            KeyCode::BackTab => Some(Action::FocusPrev),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectBlock(1)),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectBlock(-1)),
            KeyCode::Enter | KeyCode::Char(' ') => Some(Action::ToggleSelected),
            KeyCode::Home | KeyCode::Char('g') => Some(Action::ScrollTop),
            KeyCode::End | KeyCode::Char('G') => Some(Action::JumpLatest),
            KeyCode::Char('i') => Some(Action::SetFocus(Focus::Input)),
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Char(ch) if !c && !alt(key) => Some(Action::TypeInto(ch)),
            _ => None,
        },
        Focus::Sidebar => match key.code {
            KeyCode::Tab => Some(Action::FocusNext),
            KeyCode::BackTab => Some(Action::FocusPrev),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::SidebarMove(1)),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::SidebarMove(-1)),
            KeyCode::Enter | KeyCode::Char(' ') => Some(Action::SidebarActivate),
            KeyCode::Char('n') => Some(Action::NewChat),
            KeyCode::Char('i') => Some(Action::SetFocus(Focus::Input)),
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            _ => None,
        },
    }
}

/// What a clickable rectangle does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Action(Action),
    /// The scrollbar track: the action depends on the row pressed.
    Scrollbar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct hit_t {
    pub rect: Rect,
    pub target: Target,
}

/// Clickable rectangles of the last frame. Later entries sit on top.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct hit_registry_t {
    pub hits: Vec<hit_t>,
}

impl hit_registry_t {
    pub fn clear(&mut self) {
        self.hits.clear();
    }

    pub fn add(&mut self, rect: Rect, action: Action) {
        if rect.width > 0 && rect.height > 0 {
            self.hits.push(hit_t {
                rect,
                target: Target::Action(action),
            });
        }
    }

    pub fn add_target(&mut self, rect: Rect, target: Target) {
        if rect.width > 0 && rect.height > 0 {
            self.hits.push(hit_t { rect, target });
        }
    }

    pub fn at(&self, x: u16, y: u16) -> Option<&hit_t> {
        self.hits
            .iter()
            .rev()
            .find(|h| h.rect.contains(Position::new(x, y)))
    }

    /// Every rectangle whose action is `action`.
    pub fn find(&self, action: &Action) -> Option<Rect> {
        self.hits.iter().rev().find_map(|h| match &h.target {
            Target::Action(a) if a == action => Some(h.rect),
            _ => None,
        })
    }
}

/// The scrollbar action for a press at row `y` of `track`.
pub fn scrollbar_action(track: Rect, y: u16) -> Action {
    Action::ScrollbarTo {
        offset: y
            .saturating_sub(track.y)
            .min(track.height.saturating_sub(1)),
        track: track.height,
    }
}

/// What a mouse event means, given the registry and whether a scrollbar drag is active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MouseIntent {
    Act(Action),
    StartDrag(Action),
    Drag(Action),
    EndDrag,
    Hover(u16, u16),
    Nothing,
}

pub fn map_mouse(
    event: &MouseEvent,
    hits: &hit_registry_t,
    dragging: Option<Rect>,
    sidebar: Rect,
) -> MouseIntent {
    let (x, y) = (event.column, event.row);
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => match hits.at(x, y) {
            Some(hit_t {
                rect,
                target: Target::Scrollbar,
            }) => MouseIntent::StartDrag(scrollbar_action(*rect, y)),
            Some(hit_t {
                target: Target::Action(action),
                ..
            }) => MouseIntent::Act(action.clone()),
            None => MouseIntent::Nothing,
        },
        MouseEventKind::Drag(MouseButton::Left) => match dragging {
            Some(track) => MouseIntent::Drag(scrollbar_action(track, y)),
            None => MouseIntent::Nothing,
        },
        MouseEventKind::Up(_) => match dragging {
            Some(_) => MouseIntent::EndDrag,
            None => MouseIntent::Nothing,
        },
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let delta = if event.kind == MouseEventKind::ScrollUp {
                -3
            } else {
                3
            };
            if sidebar.contains(Position::new(x, y)) {
                MouseIntent::Act(Action::SidebarScroll(delta))
            } else {
                MouseIntent::Act(Action::ScrollBy(delta))
            }
        }
        MouseEventKind::Moved => MouseIntent::Hover(x, y),
        _ => MouseIntent::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn ctx(focus: Focus, modal: Modal) -> key_ctx_t {
        key_ctx_t {
            focus,
            modal,
            input_empty: true,
            popup_open: false,
            answer_empty: true,
            option_count: 3,
        }
    }

    #[test]
    fn globals_win_everywhere() {
        for modal in [Modal::None, Modal::Help, Modal::Approval, Modal::Question] {
            for focus in [Focus::Input, Focus::Transcript, Focus::Sidebar] {
                let c = ctx(focus, modal);
                assert_eq!(
                    map_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), &c),
                    Some(Action::Interrupt)
                );
                assert_eq!(
                    map_key(&key(KeyCode::Esc, KeyModifiers::NONE), &c),
                    Some(Action::Escape)
                );
                assert_eq!(
                    map_key(&key(KeyCode::Char('b'), KeyModifiers::CONTROL), &c),
                    Some(Action::ToggleSidebar)
                );
            }
        }
    }

    #[test]
    fn release_events_are_ignored() {
        let k = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
            KeyEventState::NONE,
        );
        assert_eq!(map_key(&k, &ctx(Focus::Input, Modal::None)), None);
    }

    #[test]
    fn input_keys() {
        let c = ctx(Focus::Input, Modal::None);
        assert_eq!(
            map_key(&key(KeyCode::Enter, KeyModifiers::NONE), &c),
            Some(Action::Submit)
        );
        assert_eq!(
            map_key(&key(KeyCode::Enter, KeyModifiers::ALT), &c),
            Some(Action::Edit(EditOp::Newline))
        );
        assert_eq!(
            map_key(&key(KeyCode::Char('j'), KeyModifiers::CONTROL), &c),
            Some(Action::Edit(EditOp::Newline))
        );
        assert_eq!(
            map_key(&key(KeyCode::Char('?'), KeyModifiers::NONE), &c),
            Some(Action::ToggleHelp)
        );
        let typing = key_ctx_t {
            input_empty: false,
            ..c
        };
        assert_eq!(
            map_key(&key(KeyCode::Char('?'), KeyModifiers::NONE), &typing),
            Some(Action::Edit(EditOp::Insert('?')))
        );
        let popup = key_ctx_t {
            popup_open: true,
            ..c
        };
        assert_eq!(
            map_key(&key(KeyCode::Tab, KeyModifiers::NONE), &popup),
            Some(Action::PopupComplete)
        );
        assert_eq!(
            map_key(&key(KeyCode::Tab, KeyModifiers::NONE), &c),
            Some(Action::FocusNext)
        );
    }

    #[test]
    fn approval_keys() {
        let c = ctx(Focus::Input, Modal::Approval);
        assert_eq!(
            map_key(&key(KeyCode::Char('y'), KeyModifiers::NONE), &c),
            Some(Action::Approve(ApprovalDecision::Once))
        );
        assert_eq!(
            map_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), &c),
            Some(Action::Approve(ApprovalDecision::Session))
        );
        assert_eq!(
            map_key(&key(KeyCode::Char('n'), KeyModifiers::NONE), &c),
            Some(Action::Approve(ApprovalDecision::Deny))
        );
        assert_eq!(
            map_key(&key(KeyCode::Tab, KeyModifiers::NONE), &c),
            Some(Action::ApprovalMove(1))
        );
        assert_eq!(
            map_key(&key(KeyCode::Enter, KeyModifiers::NONE), &c),
            Some(Action::ApprovalConfirm)
        );
        // Typing letters does nothing: the composer is disabled.
        assert_eq!(
            map_key(&key(KeyCode::Char('z'), KeyModifiers::NONE), &c),
            None
        );
    }

    #[test]
    fn question_keys() {
        let c = ctx(Focus::Input, Modal::Question);
        assert_eq!(
            map_key(&key(KeyCode::Char('2'), KeyModifiers::NONE), &c),
            Some(Action::PickOption(1))
        );
        assert_eq!(
            map_key(&key(KeyCode::Char('7'), KeyModifiers::NONE), &c),
            Some(Action::QuestionEdit(EditOp::Insert('7')))
        );
        let typing = key_ctx_t {
            answer_empty: false,
            ..c
        };
        assert_eq!(
            map_key(&key(KeyCode::Char('2'), KeyModifiers::NONE), &typing),
            Some(Action::QuestionEdit(EditOp::Insert('2')))
        );
        assert_eq!(
            map_key(&key(KeyCode::Enter, KeyModifiers::NONE), &c),
            Some(Action::QuestionSubmit)
        );
    }

    #[test]
    fn navigate_and_sidebar_keys() {
        let t = ctx(Focus::Transcript, Modal::None);
        assert_eq!(
            map_key(&key(KeyCode::Char('j'), KeyModifiers::NONE), &t),
            Some(Action::SelectBlock(1))
        );
        assert_eq!(
            map_key(&key(KeyCode::Char('G'), KeyModifiers::SHIFT), &t),
            Some(Action::JumpLatest)
        );
        assert_eq!(
            map_key(&key(KeyCode::Char('x'), KeyModifiers::NONE), &t),
            Some(Action::TypeInto('x'))
        );
        let s = ctx(Focus::Sidebar, Modal::None);
        assert_eq!(
            map_key(&key(KeyCode::Enter, KeyModifiers::NONE), &s),
            Some(Action::SidebarActivate)
        );
        assert_eq!(
            map_key(&key(KeyCode::Char('n'), KeyModifiers::NONE), &s),
            Some(Action::NewChat)
        );
    }

    #[test]
    fn mouse_mapping() {
        let mut hits = hit_registry_t::default();
        hits.add(Rect::new(0, 0, 10, 1), Action::NewChat);
        hits.add_target(Rect::new(79, 0, 1, 20), Target::Scrollbar);
        let ev = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let side = Rect::new(60, 0, 20, 20);
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Down(MouseButton::Left), 3, 0),
                &hits,
                None,
                side
            ),
            MouseIntent::Act(Action::NewChat)
        );
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Down(MouseButton::Left), 79, 19),
                &hits,
                None,
                side
            ),
            MouseIntent::StartDrag(Action::ScrollbarTo {
                offset: 19,
                track: 20
            })
        );
        let track = Rect::new(79, 0, 1, 20);
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Drag(MouseButton::Left), 50, 200),
                &hits,
                Some(track),
                side
            ),
            MouseIntent::Drag(Action::ScrollbarTo {
                offset: 19,
                track: 20
            })
        );
        assert_eq!(
            map_mouse(&ev(MouseEventKind::ScrollDown, 5, 5), &hits, None, side),
            MouseIntent::Act(Action::ScrollBy(3))
        );
        assert_eq!(
            map_mouse(&ev(MouseEventKind::ScrollUp, 65, 5), &hits, None, side),
            MouseIntent::Act(Action::SidebarScroll(-3))
        );
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Down(MouseButton::Left), 40, 10),
                &hits,
                None,
                side
            ),
            MouseIntent::Nothing
        );
    }
}
