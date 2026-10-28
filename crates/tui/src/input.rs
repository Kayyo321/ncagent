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
use crate::settings_page::{PageAction, PageKeys, page_key};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Focus {
    Input,
    Transcript,
    Sidebar,
    /// The settings page in the main panel (D65).
    Settings,
}

/// Tabs across the top of the main panel (D62).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabAction {
    /// A new chat in a new tab (`ctrl+t`, the `+` chip).
    New,
    /// Close tab `n` (its `×` chip, a middle click).
    Close(usize),
    /// Close the active tab (`alt+w`).
    CloseActive,
    /// Show tab `n` (a click, `alt+1` to `alt+9`).
    Select(usize),
    /// The next or previous tab, wrapping (`alt+.`, `alt+,`, `ctrl+pgdn`, `ctrl+pgup`).
    Step(i32),
    /// The last tab (`alt+0`).
    Last,
    /// Mouse only: the `‹` `›` chips and the wheel over the tab bar scroll the bar. Every tab
    /// is reached by keys with `alt+1` to `alt+9`, `alt+0` and the steps.
    Scroll(i32),
}

/// The selection of sidebar rows (D60).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectAction {
    /// `shift+↑` `shift+↓`, `K` `J`: move the cursor and select from the anchor to it.
    Extend(i32),
    /// `ctrl+space`: toggle the cursor row.
    ToggleCursor,
    /// Mouse only: the selection chip of row `n`. Keys: move to the row, `ctrl+space`.
    ToggleAt(usize),
    /// `ctrl+a` in the sidebar: every visible row.
    All,
    /// The clear chip of the selection line. Keys: `esc` in the sidebar.
    Clear,
}

/// The context menu (D61).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// `shift+f10`, the menu key, or `.` in the sidebar: the menu for the cursor row or the
    /// selection in the sidebar, else for the active tab.
    OpenKeys,
    /// The menu chip of the selection line: the menu for the selection.
    OpenSelection,
    Move(i32),
    /// Run the highlighted item, or open its submenu.
    Enter,
    /// Open the submenu of the highlighted item.
    Right,
    /// Close the submenu, or the menu.
    Back,
    /// A click on item `n` of the menu.
    Pick(usize),
    /// A click on item `n` of the open submenu.
    PickSub(usize),
    /// The digit `n + 1`: item `n` of the open submenu, else of the menu.
    Digit(usize),
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
    /// Fold (`true`) or unfold the selected folder.
    SidebarFold(bool),
    /// A new folder at the top level (`ctrl+f`, the folder button).
    NewFolder,
    /// A new folder inside the selected folder, or at the top level (`f` in the sidebar).
    NewSubfolder,
    /// Mouse only: the `+` chip of a folder row. Keys: select the row, then `f`.
    NewSubfolderAt(usize),
    /// Rename the cursor row, a folder or a conversation (`r`).
    RenameRow,
    /// Mouse only: the rename chip. Keys: select the row, then `r`.
    RenameFolderAt(usize),
    /// Delete the selection, or the cursor row (`d`, `delete`).
    DeleteRows,
    /// Mouse only: the delete chip. Keys: select the row, then `d`.
    DeleteFolderAt(usize),
    /// Pin or unpin the selection, or the cursor row (`p`).
    TogglePin,
    /// Pick the selected row up to move it (`m`).
    PickUp,
    /// Mouse only: the move chip. Keys: select the row, then `m`.
    PickUpAt(usize),
    /// Pick-up mode: step to the next allowed destination.
    MoveStep(i32),
    /// Pick-up mode: drop on the selected destination.
    MoveDrop,
    /// The folder-name prompt.
    PromptEdit(EditOp),
    PromptSubmit,
    /// Confirm the delete that waits: a folder in the sidebar, or the delete dialog.
    ConfirmYes,
    /// Close the prompt, a delete confirmation or pick-up mode.
    SidebarCancel,
    Tab(TabAction),
    Select(SelectAction),
    Menu(MenuAction),
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
    /// The settings page (D65).
    Page(PageAction),
    /// Mouse only: a click on the toast starts its dissolve. Keys: none needed, a toast goes on
    /// its own after its time.
    DismissToast,
}

/// Which widget owns the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modal {
    None,
    Help,
    Approval,
    Question,
    /// The folder-name prompt in the sidebar.
    Prompt,
    /// The inline delete confirmation in the sidebar.
    Confirm,
    /// Pick-up mode: choosing where a conversation or folder goes (D55).
    Move,
    /// The context menu (D61).
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct key_ctx_t {
    pub focus: Focus,
    pub modal: Modal,
    pub input_empty: bool,
    pub popup_open: bool,
    pub answer_empty: bool,
    pub option_count: usize,
    /// What the settings page's keys do, when it has the focus.
    pub page: PageKeys,
}

fn ctrl(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

fn alt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT)
}

fn shift(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::SHIFT)
}

/// The tab keys (D62). They avoid the composer's keys: `ctrl+w` deletes a word and `alt+←`
/// `alt+→` move by word, and `ctrl+tab` does not reach most terminals.
fn tab_key(key: &KeyEvent) -> Option<TabAction> {
    let (c, a) = (ctrl(key), alt(key));
    match key.code {
        KeyCode::Char('t') if c => Some(TabAction::New),
        KeyCode::Char('w') if a && !c => Some(TabAction::CloseActive),
        KeyCode::Char('.') if a && !c => Some(TabAction::Step(1)),
        KeyCode::Char(',') if a && !c => Some(TabAction::Step(-1)),
        KeyCode::PageDown if c => Some(TabAction::Step(1)),
        KeyCode::PageUp if c => Some(TabAction::Step(-1)),
        KeyCode::Char('0') if a && !c => Some(TabAction::Last),
        KeyCode::Char(d @ '1'..='9') if a && !c => {
            Some(TabAction::Select((d as usize).saturating_sub('1' as usize)))
        }
        _ => None,
    }
}

/// Keys that work in every state.
fn global(key: &KeyEvent) -> Option<Action> {
    let c = ctrl(key);
    if let Some(tab) = tab_key(key) {
        return Some(Action::Tab(tab));
    }
    match key.code {
        KeyCode::Char('c') if c => Some(Action::Interrupt),
        KeyCode::Char('b') if c => Some(Action::ToggleSidebar),
        KeyCode::Char('n') if c => Some(Action::NewChat),
        KeyCode::Char('y') if c => Some(Action::CopyReply),
        KeyCode::Char('l') if c => Some(Action::Redraw),
        KeyCode::Char('x') if c => Some(Action::ClearQueue),
        KeyCode::Char('f') if c => Some(Action::NewFolder),
        KeyCode::Char('o') if c => Some(Action::Page(PageAction::Open)),
        KeyCode::Esc => Some(Action::Escape),
        KeyCode::F(1) => Some(Action::ToggleHelp),
        KeyCode::F(2) => Some(Action::Page(PageAction::Open)),
        KeyCode::F(10) if shift(key) => Some(Action::Menu(MenuAction::OpenKeys)),
        KeyCode::Menu => Some(Action::Menu(MenuAction::OpenKeys)),
        KeyCode::PageUp => Some(Action::PageBy(-1)),
        KeyCode::PageDown => Some(Action::PageBy(1)),
        KeyCode::Home if c => Some(Action::ScrollTop),
        KeyCode::End if c => Some(Action::JumpLatest),
        _ => None,
    }
}

/// The editing keys of a one-line or multi-line editor.
pub(crate) fn editing(key: &KeyEvent) -> Option<EditOp> {
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
        Modal::Prompt => {
            return match key.code {
                KeyCode::Enter if !alt(key) => Some(Action::PromptSubmit),
                KeyCode::Tab | KeyCode::BackTab => None,
                _ => editing(key)
                    .filter(|op| !matches!(op, EditOp::Newline | EditOp::Up | EditOp::Down))
                    .map(Action::PromptEdit),
            };
        }
        Modal::Confirm => {
            return match key.code {
                KeyCode::Enter | KeyCode::Char('y') if !c => Some(Action::ConfirmYes),
                KeyCode::Char('n') if !c => Some(Action::SidebarCancel),
                KeyCode::Char('?') => Some(Action::ToggleHelp),
                _ => None,
            };
        }
        Modal::Menu => {
            let menu = |m| Some(Action::Menu(m));
            return match key.code {
                KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => menu(MenuAction::Move(-1)),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => menu(MenuAction::Move(1)),
                KeyCode::Enter | KeyCode::Char(' ') => menu(MenuAction::Enter),
                KeyCode::Right | KeyCode::Char('l') => menu(MenuAction::Right),
                KeyCode::Left | KeyCode::Char('h') => menu(MenuAction::Back),
                KeyCode::Char(d @ '1'..='9') if !c => {
                    menu(MenuAction::Digit((d as usize).saturating_sub('1' as usize)))
                }
                _ => None,
            };
        }
        Modal::Move => {
            return match key.code {
                KeyCode::Down | KeyCode::Char('j') => Some(Action::MoveStep(1)),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::MoveStep(-1)),
                KeyCode::Enter | KeyCode::Char(' ') => Some(Action::MoveDrop),
                KeyCode::Left | KeyCode::Char('h') => Some(Action::SidebarFold(true)),
                KeyCode::Right | KeyCode::Char('l') => Some(Action::SidebarFold(false)),
                KeyCode::Char('?') => Some(Action::ToggleHelp),
                _ => None,
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
            KeyCode::Down if shift(key) => Some(Action::Select(SelectAction::Extend(1))),
            KeyCode::Up if shift(key) => Some(Action::Select(SelectAction::Extend(-1))),
            KeyCode::Char('J') => Some(Action::Select(SelectAction::Extend(1))),
            KeyCode::Char('K') => Some(Action::Select(SelectAction::Extend(-1))),
            KeyCode::Char(' ') if c => Some(Action::Select(SelectAction::ToggleCursor)),
            KeyCode::Char('a') if c => Some(Action::Select(SelectAction::All)),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::SidebarMove(1)),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::SidebarMove(-1)),
            KeyCode::Enter | KeyCode::Char(' ') => Some(Action::SidebarActivate),
            KeyCode::Left | KeyCode::Char('h') => Some(Action::SidebarFold(true)),
            KeyCode::Right | KeyCode::Char('l') => Some(Action::SidebarFold(false)),
            KeyCode::Char('n') => Some(Action::NewChat),
            KeyCode::Char('f') => Some(Action::NewSubfolder),
            KeyCode::Char('r') => Some(Action::RenameRow),
            KeyCode::Char('d') | KeyCode::Delete => Some(Action::DeleteRows),
            KeyCode::Char('p') => Some(Action::TogglePin),
            KeyCode::Char('.') => Some(Action::Menu(MenuAction::OpenKeys)),
            KeyCode::Char('m') => Some(Action::PickUp),
            KeyCode::Char('s') => Some(Action::Page(PageAction::Open)),
            KeyCode::Char('i') => Some(Action::SetFocus(Focus::Input)),
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            _ => None,
        },
        Focus::Settings => page_key(key, ctx.page),
    }
}

/// What a clickable rectangle does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Action(Action),
    /// The scrollbar track: the action depends on the row pressed.
    Scrollbar,
    /// A row of the sidebar's conversation tree, by item index. A press grabs it: a release on
    /// the same row is a click (open or fold), and a drag to another row moves it (D55). The
    /// "conversations" label is the row of the top level.
    Row(usize),
    /// Tab `n` of the tab bar: a press shows it, a middle click closes it, and a right click
    /// opens its menu (D62).
    Tab(usize),
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

    /// The topmost rectangle whose action is `action`.
    pub fn find(&self, action: &Action) -> Option<Rect> {
        self.find_target(&Target::Action(action.clone()))
    }

    /// The topmost rectangle with this target.
    pub fn find_target(&self, target: &Target) -> Option<Rect> {
        self.hits
            .iter()
            .rev()
            .find_map(|h| (h.target == *target).then_some(h.rect))
    }

    /// The sidebar row under a point, if the topmost target there is a row.
    pub fn row_at(&self, x: u16, y: u16) -> Option<usize> {
        match self.at(x, y) {
            Some(hit_t {
                target: Target::Row(index),
                ..
            }) => Some(*index),
            _ => None,
        }
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
    /// A press on a sidebar row: the row is grabbed until the release. The modifiers decide
    /// what a click (a release without a drag) does to the selection (D60).
    Grab(usize, KeyModifiers),
    /// A right press: the app opens the menu of whatever is under the pointer (D61).
    Context(u16, u16),
    /// A press on the selection chip of sidebar row `n`: a drag from it paints a range.
    Paint(usize),
    /// The pointer moved with the button held, outside a scrollbar drag.
    DragAt(u16, u16),
    /// The button went up, outside a scrollbar drag.
    Release(u16, u16),
    Nothing,
}

/// The areas where the wheel scrolls something other than the transcript.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct wheel_areas_t {
    pub sidebar: Rect,
    pub tabs: Rect,
}

pub fn map_mouse(
    event: &MouseEvent,
    hits: &hit_registry_t,
    dragging: Option<Rect>,
    areas: wheel_areas_t,
) -> MouseIntent {
    let (x, y) = (event.column, event.row);
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => match hits.at(x, y) {
            Some(hit_t {
                rect,
                target: Target::Scrollbar,
            }) => MouseIntent::StartDrag(scrollbar_action(*rect, y)),
            Some(hit_t {
                target: Target::Action(Action::Select(SelectAction::ToggleAt(index))),
                ..
            }) => MouseIntent::Paint(*index),
            Some(hit_t {
                target: Target::Action(action),
                ..
            }) => MouseIntent::Act(action.clone()),
            Some(hit_t {
                target: Target::Row(index),
                ..
            }) => MouseIntent::Grab(*index, event.modifiers),
            Some(hit_t {
                target: Target::Tab(index),
                ..
            }) => MouseIntent::Act(Action::Tab(TabAction::Select(*index))),
            None => MouseIntent::Nothing,
        },
        MouseEventKind::Down(MouseButton::Right) => MouseIntent::Context(x, y),
        MouseEventKind::Down(MouseButton::Middle) => match hits.at(x, y) {
            Some(hit_t {
                target: Target::Tab(index),
                ..
            }) => MouseIntent::Act(Action::Tab(TabAction::Close(*index))),
            _ => MouseIntent::Nothing,
        },
        MouseEventKind::Drag(MouseButton::Left) => match dragging {
            Some(track) => MouseIntent::Drag(scrollbar_action(track, y)),
            None => MouseIntent::DragAt(x, y),
        },
        MouseEventKind::Up(_) => match dragging {
            Some(_) => MouseIntent::EndDrag,
            None => MouseIntent::Release(x, y),
        },
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let delta: i32 = if event.kind == MouseEventKind::ScrollUp {
                -3
            } else {
                3
            };
            if areas.tabs.contains(Position::new(x, y)) {
                MouseIntent::Act(Action::Tab(TabAction::Scroll(delta.signum())))
            } else if areas.sidebar.contains(Position::new(x, y)) {
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
            page: PageKeys::Browse,
        }
    }

    #[test]
    fn globals_win_everywhere() {
        for modal in [
            Modal::None,
            Modal::Help,
            Modal::Approval,
            Modal::Question,
            Modal::Prompt,
            Modal::Confirm,
            Modal::Move,
            Modal::Menu,
        ] {
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
    fn folder_keys() {
        let s = ctx(Focus::Sidebar, Modal::None);
        let k = |code| map_key(&key(code, KeyModifiers::NONE), &s);
        assert_eq!(k(KeyCode::Char('f')), Some(Action::NewSubfolder));
        assert_eq!(k(KeyCode::Char('r')), Some(Action::RenameRow));
        assert_eq!(k(KeyCode::Char('d')), Some(Action::DeleteRows));
        assert_eq!(k(KeyCode::Delete), Some(Action::DeleteRows));
        assert_eq!(k(KeyCode::Char('p')), Some(Action::TogglePin));
        assert_eq!(k(KeyCode::Char('m')), Some(Action::PickUp));
        assert_eq!(k(KeyCode::Left), Some(Action::SidebarFold(true)));
        assert_eq!(k(KeyCode::Right), Some(Action::SidebarFold(false)));
        for focus in [Focus::Input, Focus::Transcript, Focus::Sidebar] {
            assert_eq!(
                map_key(
                    &key(KeyCode::Char('f'), KeyModifiers::CONTROL),
                    &ctx(focus, Modal::None)
                ),
                Some(Action::NewFolder)
            );
        }
        // The input keeps plain letters.
        assert_eq!(
            map_key(
                &key(KeyCode::Char('f'), KeyModifiers::NONE),
                &ctx(Focus::Input, Modal::None)
            ),
            Some(Action::Edit(EditOp::Insert('f')))
        );

        let p = ctx(Focus::Sidebar, Modal::Prompt);
        let k = |code, m| map_key(&key(code, m), &p);
        assert_eq!(
            k(KeyCode::Char('j'), KeyModifiers::NONE),
            Some(Action::PromptEdit(EditOp::Insert('j')))
        );
        assert_eq!(
            k(KeyCode::Enter, KeyModifiers::NONE),
            Some(Action::PromptSubmit)
        );
        assert_eq!(k(KeyCode::Char('j'), KeyModifiers::CONTROL), None);
        assert_eq!(k(KeyCode::Up, KeyModifiers::NONE), None);
        assert_eq!(
            k(KeyCode::Backspace, KeyModifiers::NONE),
            Some(Action::PromptEdit(EditOp::Backspace))
        );

        let c = ctx(Focus::Sidebar, Modal::Confirm);
        let k = |code| map_key(&key(code, KeyModifiers::NONE), &c);
        assert_eq!(k(KeyCode::Enter), Some(Action::ConfirmYes));
        assert_eq!(k(KeyCode::Char('y')), Some(Action::ConfirmYes));
        assert_eq!(k(KeyCode::Char('n')), Some(Action::SidebarCancel));
        assert_eq!(k(KeyCode::Char('d')), None);

        let m = ctx(Focus::Sidebar, Modal::Move);
        let k = |code| map_key(&key(code, KeyModifiers::NONE), &m);
        assert_eq!(k(KeyCode::Char('j')), Some(Action::MoveStep(1)));
        assert_eq!(k(KeyCode::Up), Some(Action::MoveStep(-1)));
        assert_eq!(k(KeyCode::Enter), Some(Action::MoveDrop));
        assert_eq!(k(KeyCode::Char('x')), None);
    }

    #[test]
    fn tab_keys_work_everywhere_and_leave_the_composer_keys_alone() {
        let k = |code, m, c: &key_ctx_t| map_key(&key(code, m), c);
        for modal in [Modal::None, Modal::Approval, Modal::Question, Modal::Prompt] {
            for focus in [Focus::Input, Focus::Transcript, Focus::Sidebar] {
                let c = ctx(focus, modal);
                assert_eq!(
                    k(KeyCode::Char('t'), KeyModifiers::CONTROL, &c),
                    Some(Action::Tab(TabAction::New))
                );
                assert_eq!(
                    k(KeyCode::Char('w'), KeyModifiers::ALT, &c),
                    Some(Action::Tab(TabAction::CloseActive))
                );
                assert_eq!(
                    k(KeyCode::Char('.'), KeyModifiers::ALT, &c),
                    Some(Action::Tab(TabAction::Step(1)))
                );
                assert_eq!(
                    k(KeyCode::Char(','), KeyModifiers::ALT, &c),
                    Some(Action::Tab(TabAction::Step(-1)))
                );
                assert_eq!(
                    k(KeyCode::PageDown, KeyModifiers::CONTROL, &c),
                    Some(Action::Tab(TabAction::Step(1)))
                );
                assert_eq!(
                    k(KeyCode::Char('3'), KeyModifiers::ALT, &c),
                    Some(Action::Tab(TabAction::Select(2)))
                );
                assert_eq!(
                    k(KeyCode::Char('0'), KeyModifiers::ALT, &c),
                    Some(Action::Tab(TabAction::Last))
                );
                assert_eq!(
                    k(KeyCode::F(10), KeyModifiers::SHIFT, &c),
                    Some(Action::Menu(MenuAction::OpenKeys))
                );
            }
        }
        let input = ctx(Focus::Input, Modal::None);
        assert_eq!(
            k(KeyCode::Char('w'), KeyModifiers::CONTROL, &input),
            Some(Action::Edit(EditOp::KillWordBack))
        );
        assert_eq!(
            k(KeyCode::Left, KeyModifiers::ALT, &input),
            Some(Action::Edit(EditOp::WordLeft))
        );
        assert_eq!(
            k(KeyCode::PageDown, KeyModifiers::NONE, &input),
            Some(Action::PageBy(1))
        );
        assert_eq!(
            k(KeyCode::Char('.'), KeyModifiers::NONE, &input),
            Some(Action::Edit(EditOp::Insert('.')))
        );
    }

    #[test]
    fn selection_and_menu_keys() {
        let s = ctx(Focus::Sidebar, Modal::None);
        let k = |code, m| map_key(&key(code, m), &s);
        assert_eq!(
            k(KeyCode::Down, KeyModifiers::SHIFT),
            Some(Action::Select(SelectAction::Extend(1)))
        );
        assert_eq!(
            k(KeyCode::Up, KeyModifiers::SHIFT),
            Some(Action::Select(SelectAction::Extend(-1)))
        );
        assert_eq!(
            k(KeyCode::Char('J'), KeyModifiers::SHIFT),
            Some(Action::Select(SelectAction::Extend(1)))
        );
        assert_eq!(
            k(KeyCode::Char(' '), KeyModifiers::CONTROL),
            Some(Action::Select(SelectAction::ToggleCursor))
        );
        assert_eq!(
            k(KeyCode::Char(' '), KeyModifiers::NONE),
            Some(Action::SidebarActivate)
        );
        assert_eq!(
            k(KeyCode::Char('a'), KeyModifiers::CONTROL),
            Some(Action::Select(SelectAction::All))
        );
        assert_eq!(
            k(KeyCode::Char('.'), KeyModifiers::NONE),
            Some(Action::Menu(MenuAction::OpenKeys))
        );
        let m = ctx(Focus::Sidebar, Modal::Menu);
        let k = |code| map_key(&key(code, KeyModifiers::NONE), &m);
        assert_eq!(
            k(KeyCode::Char('j')),
            Some(Action::Menu(MenuAction::Move(1)))
        );
        assert_eq!(k(KeyCode::Up), Some(Action::Menu(MenuAction::Move(-1))));
        assert_eq!(k(KeyCode::Enter), Some(Action::Menu(MenuAction::Enter)));
        assert_eq!(k(KeyCode::Right), Some(Action::Menu(MenuAction::Right)));
        assert_eq!(k(KeyCode::Char('h')), Some(Action::Menu(MenuAction::Back)));
        assert_eq!(
            k(KeyCode::Char('2')),
            Some(Action::Menu(MenuAction::Digit(1)))
        );
        assert_eq!(k(KeyCode::Char('x')), None);
        assert_eq!(k(KeyCode::Esc), Some(Action::Escape));
    }

    #[test]
    fn right_and_middle_presses_and_modifiers() {
        let mut hits = hit_registry_t::default();
        hits.add_target(Rect::new(0, 0, 20, 1), Target::Tab(2));
        hits.add_target(Rect::new(60, 5, 20, 1), Target::Row(4));
        let ev = |kind, column, row, modifiers| MouseEvent {
            kind,
            column,
            row,
            modifiers,
        };
        let none = KeyModifiers::NONE;
        let areas = wheel_areas_t::default();
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Down(MouseButton::Left), 3, 0, none),
                &hits,
                None,
                areas
            ),
            MouseIntent::Act(Action::Tab(TabAction::Select(2)))
        );
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Down(MouseButton::Middle), 3, 0, none),
                &hits,
                None,
                areas
            ),
            MouseIntent::Act(Action::Tab(TabAction::Close(2)))
        );
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Down(MouseButton::Middle), 3, 9, none),
                &hits,
                None,
                areas
            ),
            MouseIntent::Nothing
        );
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Down(MouseButton::Right), 61, 5, none),
                &hits,
                None,
                areas
            ),
            MouseIntent::Context(61, 5)
        );
        let both = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Down(MouseButton::Left), 61, 5, both),
                &hits,
                None,
                areas
            ),
            MouseIntent::Grab(4, both)
        );
    }

    #[test]
    fn row_targets_grab_and_release() {
        let mut hits = hit_registry_t::default();
        hits.add_target(Rect::new(60, 5, 20, 1), Target::Row(3));
        hits.add(Rect::new(76, 5, 3, 1), Action::PickUpAt(3));
        let ev = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let side = wheel_areas_t {
            sidebar: Rect::new(60, 0, 20, 20),
            tabs: Rect::default(),
        };
        let down = MouseEventKind::Down(MouseButton::Left);
        assert_eq!(
            map_mouse(&ev(down, 62, 5), &hits, None, side),
            MouseIntent::Grab(3, KeyModifiers::NONE)
        );
        // A chip on the row sits on top of it.
        assert_eq!(
            map_mouse(&ev(down, 77, 5), &hits, None, side),
            MouseIntent::Act(Action::PickUpAt(3))
        );
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Drag(MouseButton::Left), 61, 9),
                &hits,
                None,
                side
            ),
            MouseIntent::DragAt(61, 9)
        );
        assert_eq!(
            map_mouse(
                &ev(MouseEventKind::Up(MouseButton::Left), 61, 9),
                &hits,
                None,
                side
            ),
            MouseIntent::Release(61, 9)
        );
        assert_eq!(hits.row_at(62, 5), Some(3));
        assert_eq!(hits.row_at(77, 5), None);
        assert_eq!(
            hits.find_target(&Target::Row(3)),
            Some(Rect::new(60, 5, 20, 1))
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
        let side = wheel_areas_t {
            sidebar: Rect::new(60, 0, 20, 20),
            tabs: Rect::new(0, 0, 60, 1),
        };
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
            map_mouse(&ev(MouseEventKind::ScrollDown, 5, 0), &hits, None, side),
            MouseIntent::Act(Action::Tab(TabAction::Scroll(1)))
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
