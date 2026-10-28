/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! What the settings page offers (PROJECT.md, D63, D65): its sections, every setting with its
//! label, its one-line explanation and its control, and the rules that read, change and reset a
//! value of `settings_t`. Pure: no drawing and no app state, so each rule is tested directly.
//!
//! The saved tab list (`open_tabs`, `active_tab`) is not a setting the page shows: it is kept by
//! the tabs (D62), and a reset leaves it alone.

use harness::approval::ApprovalMode;
use harness::settings::{
    ConversationOrder, Density, OpenIn, SidebarVisibility, SidebarWidth, ToolResultView, settings_t,
};

/// A section of the settings page, in the order the list shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Appearance,
    Sidebar,
    Conversations,
    Tabs,
    Themes,
    About,
}

pub const Sections: [Section; 6] = [
    Section::Appearance,
    Section::Sidebar,
    Section::Conversations,
    Section::Tabs,
    Section::Themes,
    Section::About,
];

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::Appearance => "appearance",
            Section::Sidebar => "sidebar",
            Section::Conversations => "conversations",
            Section::Tabs => "tabs",
            Section::Themes => "themes",
            Section::About => "about",
        }
    }

    /// The line under the section's heading.
    pub fn about(self) -> &'static str {
        match self {
            Section::Appearance => "how ncagent looks and moves",
            Section::Sidebar => "the column on the right: when it docks and what it shows",
            Section::Conversations => "new conversations, the list, and deleting",
            Section::Tabs => "the conversations open across the top",
            Section::Themes => "built-in palettes and the ones you import",
            Section::About => "read-only facts about this run",
        }
    }

    /// The position of the section in `Sections`.
    pub fn index(self) -> usize {
        Sections.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The section `d` steps away, clamped to the list.
    pub fn step(self, d: i32) -> Section {
        let i = (self.index() as i64 + i64::from(d)).clamp(0, Sections.len() as i64 - 1);
        Sections.get(i as usize).copied().unwrap_or(self)
    }
}

/// One setting of `settings_t` the page can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingKey {
    Theme,
    Animation,
    Density,
    ToolResults,
    Sidebar,
    SidebarWidth,
    ShowContext,
    ShowTurn,
    ShowGeneration,
    ShowModel,
    ShowRowDetails,
    DefaultApproval,
    ConfirmDelete,
    OpenIn,
    ShowArchived,
    ConversationOrder,
    RestoreTabs,
}

/// Every setting, in the order the page lists them.
pub const SettingKeys: [SettingKey; 17] = [
    SettingKey::Theme,
    SettingKey::Animation,
    SettingKey::Density,
    SettingKey::ToolResults,
    SettingKey::Sidebar,
    SettingKey::SidebarWidth,
    SettingKey::ShowContext,
    SettingKey::ShowTurn,
    SettingKey::ShowGeneration,
    SettingKey::ShowModel,
    SettingKey::ShowRowDetails,
    SettingKey::DefaultApproval,
    SettingKey::ConfirmDelete,
    SettingKey::OpenIn,
    SettingKey::ShowArchived,
    SettingKey::ConversationOrder,
    SettingKey::RestoreTabs,
];

/// How a setting is changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// On or off.
    Toggle,
    /// One of a few options, shown as segmented chips.
    Choice,
    /// One of the themes, which the page lists from the theme set.
    Theme,
}

/// The option of `default_approval` that only the command line can select (D15).
pub const FullOption: usize = 2;

impl SettingKey {
    pub fn section(self) -> Section {
        match self {
            SettingKey::Theme
            | SettingKey::Animation
            | SettingKey::Density
            | SettingKey::ToolResults => Section::Appearance,
            SettingKey::Sidebar
            | SettingKey::SidebarWidth
            | SettingKey::ShowContext
            | SettingKey::ShowTurn
            | SettingKey::ShowGeneration
            | SettingKey::ShowModel
            | SettingKey::ShowRowDetails => Section::Sidebar,
            SettingKey::DefaultApproval
            | SettingKey::ConfirmDelete
            | SettingKey::OpenIn
            | SettingKey::ShowArchived
            | SettingKey::ConversationOrder => Section::Conversations,
            SettingKey::RestoreTabs => Section::Tabs,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingKey::Theme => "theme",
            SettingKey::Animation => "animation",
            SettingKey::Density => "density",
            SettingKey::ToolResults => "tool results",
            SettingKey::Sidebar => "sidebar",
            SettingKey::SidebarWidth => "sidebar width",
            SettingKey::ShowContext => "context",
            SettingKey::ShowTurn => "turn",
            SettingKey::ShowGeneration => "generation",
            SettingKey::ShowModel => "model",
            SettingKey::ShowRowDetails => "row details",
            SettingKey::DefaultApproval => "approval for new chats",
            SettingKey::ConfirmDelete => "confirm delete",
            SettingKey::OpenIn => "open conversations in",
            SettingKey::ShowArchived => "show archived",
            SettingKey::ConversationOrder => "order",
            SettingKey::RestoreTabs => "restore tabs",
        }
    }

    /// The one-line explanation under the label.
    pub fn about(self) -> &'static str {
        match self {
            SettingKey::Theme => "the palette of the whole UI; hover a theme to preview it",
            SettingKey::Animation => {
                "the level-by-level dissolve, the logo reveal and the pulses; also /animate"
            }
            SettingKey::Density => "compact drops the blank rows between blocks and sections",
            SettingKey::ToolResults => {
                "whether a tool's output starts folded; enter or a click still flips one"
            }
            SettingKey::Sidebar => {
                "auto docks from 100 columns; pinned docks whenever it fits; hidden keeps it a drawer"
            }
            SettingKey::SidebarWidth => "28, 34 or 42 columns; normal grows to 38 from 150 columns",
            SettingKey::ShowContext => "the context section: tokens packed into the window",
            SettingKey::ShowTurn => "the turn section: the phase, calls and level passes",
            SettingKey::ShowGeneration => "the generation section: the level strip and timings",
            SettingKey::ShowModel => "the model section: the model id and backend",
            SettingKey::ShowRowDetails => "the age and turn count beside each conversation",
            SettingKey::DefaultApproval => {
                "the mode a conversation without its own runs in; full is for the command line"
            }
            SettingKey::ConfirmDelete => "ask before a conversation and its saved log are deleted",
            SettingKey::OpenIn => "a click opens a new tab, or replaces the idle tab you are in",
            SettingKey::ShowArchived => "list archived conversations, dimmed, with a tag",
            SettingKey::ConversationOrder => "newest first, or by title",
            SettingKey::RestoreTabs => "reopen the tabs that were open when ncagent last quit",
        }
    }

    pub fn control(self) -> Control {
        match self {
            SettingKey::Theme => Control::Theme,
            SettingKey::Animation
            | SettingKey::ShowContext
            | SettingKey::ShowTurn
            | SettingKey::ShowGeneration
            | SettingKey::ShowModel
            | SettingKey::ShowRowDetails
            | SettingKey::ConfirmDelete
            | SettingKey::ShowArchived
            | SettingKey::RestoreTabs => Control::Toggle,
            SettingKey::Density
            | SettingKey::ToolResults
            | SettingKey::Sidebar
            | SettingKey::SidebarWidth
            | SettingKey::DefaultApproval
            | SettingKey::OpenIn
            | SettingKey::ConversationOrder => Control::Choice,
        }
    }

    /// The options of a choice, in order. Empty for a toggle or the theme.
    pub fn options(self) -> &'static [&'static str] {
        match self {
            SettingKey::Density => &["comfortable", "compact"],
            SettingKey::ToolResults => &["folded", "expanded"],
            SettingKey::Sidebar => &["auto", "pinned", "hidden"],
            SettingKey::SidebarWidth => &["compact", "normal", "wide"],
            SettingKey::DefaultApproval => &["ask", "edit", "full"],
            SettingKey::OpenIn => &["new tab", "current tab"],
            SettingKey::ConversationOrder => &["recent", "title"],
            _ => &[],
        }
    }

    /// False for an option that is shown but cannot be chosen here.
    pub fn option_enabled(self, index: usize) -> bool {
        !(self == SettingKey::DefaultApproval && index == FullOption)
    }

    /// The option chosen now. 0 for a toggle or the theme.
    pub fn choice(self, s: &settings_t) -> usize {
        match self {
            SettingKey::Density => match s.density {
                Density::Comfortable => 0,
                Density::Compact => 1,
            },
            SettingKey::ToolResults => match s.tool_results {
                ToolResultView::Folded => 0,
                ToolResultView::Expanded => 1,
            },
            SettingKey::Sidebar => match s.sidebar {
                SidebarVisibility::Auto => 0,
                SidebarVisibility::Pinned => 1,
                SidebarVisibility::Hidden => 2,
            },
            SettingKey::SidebarWidth => match s.sidebar_width {
                SidebarWidth::Compact => 0,
                SidebarWidth::Normal => 1,
                SidebarWidth::Wide => 2,
            },
            SettingKey::DefaultApproval => match s.default_approval {
                ApprovalMode::Ask => 0,
                ApprovalMode::Edit => 1,
                ApprovalMode::Full => FullOption,
            },
            SettingKey::OpenIn => match s.open_in {
                OpenIn::NewTab => 0,
                OpenIn::CurrentTab => 1,
            },
            SettingKey::ConversationOrder => match s.conversation_order {
                ConversationOrder::Recent => 0,
                ConversationOrder::Title => 1,
            },
            _ => 0,
        }
    }

    /// Choose option `index`. False, and nothing changes, for an option out of range, one that
    /// cannot be chosen here, or a setting that is not a choice.
    pub fn set_choice(self, s: &mut settings_t, index: usize) -> bool {
        if !self.option_enabled(index) {
            return false;
        }
        match (self, index) {
            (SettingKey::Density, 0) => s.density = Density::Comfortable,
            (SettingKey::Density, 1) => s.density = Density::Compact,
            (SettingKey::ToolResults, 0) => s.tool_results = ToolResultView::Folded,
            (SettingKey::ToolResults, 1) => s.tool_results = ToolResultView::Expanded,
            (SettingKey::Sidebar, 0) => s.sidebar = SidebarVisibility::Auto,
            (SettingKey::Sidebar, 1) => s.sidebar = SidebarVisibility::Pinned,
            (SettingKey::Sidebar, 2) => s.sidebar = SidebarVisibility::Hidden,
            (SettingKey::SidebarWidth, 0) => s.sidebar_width = SidebarWidth::Compact,
            (SettingKey::SidebarWidth, 1) => s.sidebar_width = SidebarWidth::Normal,
            (SettingKey::SidebarWidth, 2) => s.sidebar_width = SidebarWidth::Wide,
            (SettingKey::DefaultApproval, 0) => s.default_approval = ApprovalMode::Ask,
            (SettingKey::DefaultApproval, 1) => s.default_approval = ApprovalMode::Edit,
            (SettingKey::OpenIn, 0) => s.open_in = OpenIn::NewTab,
            (SettingKey::OpenIn, 1) => s.open_in = OpenIn::CurrentTab,
            (SettingKey::ConversationOrder, 0) => s.conversation_order = ConversationOrder::Recent,
            (SettingKey::ConversationOrder, 1) => s.conversation_order = ConversationOrder::Title,
            _ => return false,
        }
        true
    }

    /// The next option `d` steps away that can be chosen, wrapping. `None` when there is none.
    pub fn stepped(self, s: &settings_t, d: i32) -> Option<usize> {
        let n = self.options().len();
        if n == 0 {
            return None;
        }
        let mut i = self.choice(s);
        for _ in 0..n {
            i = (i as i64 + i64::from(d.signum())).rem_euclid(n as i64) as usize;
            if self.option_enabled(i) {
                return Some(i);
            }
        }
        None
    }

    /// The value of a toggle. `None` for a setting that is not a toggle.
    pub fn toggle(self, s: &settings_t) -> Option<bool> {
        Some(match self {
            SettingKey::Animation => s.animation,
            SettingKey::ShowContext => s.show_context,
            SettingKey::ShowTurn => s.show_turn,
            SettingKey::ShowGeneration => s.show_generation,
            SettingKey::ShowModel => s.show_model,
            SettingKey::ShowRowDetails => s.show_row_details,
            SettingKey::ConfirmDelete => s.confirm_delete,
            SettingKey::ShowArchived => s.show_archived,
            SettingKey::RestoreTabs => s.restore_tabs,
            _ => return None,
        })
    }

    /// Set a toggle. False for a setting that is not a toggle.
    pub fn set_toggle(self, s: &mut settings_t, on: bool) -> bool {
        let field = match self {
            SettingKey::Animation => &mut s.animation,
            SettingKey::ShowContext => &mut s.show_context,
            SettingKey::ShowTurn => &mut s.show_turn,
            SettingKey::ShowGeneration => &mut s.show_generation,
            SettingKey::ShowModel => &mut s.show_model,
            SettingKey::ShowRowDetails => &mut s.show_row_details,
            SettingKey::ConfirmDelete => &mut s.confirm_delete,
            SettingKey::ShowArchived => &mut s.show_archived,
            SettingKey::RestoreTabs => &mut s.restore_tabs,
            _ => return false,
        };
        *field = on;
        true
    }

    /// Put this setting back to its default.
    pub fn reset(self, s: &mut settings_t) {
        let d = settings_t::default();
        match self {
            SettingKey::Theme => s.theme = d.theme,
            SettingKey::Animation => s.animation = d.animation,
            SettingKey::Density => s.density = d.density,
            SettingKey::ToolResults => s.tool_results = d.tool_results,
            SettingKey::Sidebar => s.sidebar = d.sidebar,
            SettingKey::SidebarWidth => s.sidebar_width = d.sidebar_width,
            SettingKey::ShowContext => s.show_context = d.show_context,
            SettingKey::ShowTurn => s.show_turn = d.show_turn,
            SettingKey::ShowGeneration => s.show_generation = d.show_generation,
            SettingKey::ShowModel => s.show_model = d.show_model,
            SettingKey::ShowRowDetails => s.show_row_details = d.show_row_details,
            SettingKey::DefaultApproval => s.default_approval = d.default_approval,
            SettingKey::ConfirmDelete => s.confirm_delete = d.confirm_delete,
            SettingKey::OpenIn => s.open_in = d.open_in,
            SettingKey::ShowArchived => s.show_archived = d.show_archived,
            SettingKey::ConversationOrder => s.conversation_order = d.conversation_order,
            SettingKey::RestoreTabs => s.restore_tabs = d.restore_tabs,
        }
    }

    /// True when the setting holds its default value.
    pub fn is_default(self, s: &settings_t) -> bool {
        let mut reset = s.clone();
        self.reset(&mut reset);
        reset == *s
    }
}

/// How many settings differ from their defaults.
pub fn changed_count(s: &settings_t) -> usize {
    SettingKeys.iter().filter(|k| !k.is_default(s)).count()
}

/// Every setting back to its default. The saved tab list stays (D62).
pub fn reset_all(s: &mut settings_t) {
    for key in SettingKeys {
        key.reset(s);
    }
}

/// The settings of a section, in order.
pub fn keys_of(section: Section) -> Vec<SettingKey> {
    SettingKeys
        .iter()
        .copied()
        .filter(|k| k.section() == section)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_has_a_section_a_label_and_an_explanation() {
        for key in SettingKeys {
            assert!(
                !key.label().is_empty() && !key.about().is_empty(),
                "{key:?}"
            );
            match key.control() {
                Control::Choice => assert!(key.options().len() >= 2, "{key:?}"),
                Control::Toggle => assert!(key.toggle(&settings_t::default()).is_some()),
                Control::Theme => assert_eq!(key, SettingKey::Theme),
            }
        }
        for section in Sections {
            let has_rows = !keys_of(section).is_empty();
            let listed_elsewhere = matches!(section, Section::Themes | Section::About);
            assert!(has_rows || listed_elsewhere, "{section:?}");
        }
    }

    #[test]
    fn each_choice_round_trips_and_full_is_refused() {
        for key in SettingKeys {
            for i in 0..key.options().len() {
                let mut s = settings_t::default();
                let set = key.set_choice(&mut s, i);
                assert_eq!(set, key.option_enabled(i), "{key:?} {i}");
                if set {
                    assert_eq!(key.choice(&s), i, "{key:?} {i}");
                    assert!(s.validate().is_ok());
                }
            }
        }
        let mut s = settings_t::default();
        assert!(!SettingKey::DefaultApproval.set_choice(&mut s, FullOption));
        assert_eq!(s.default_approval, ApprovalMode::Ask);
        // Stepping skips full and wraps.
        assert_eq!(SettingKey::DefaultApproval.stepped(&s, 1), Some(1));
        assert!(SettingKey::DefaultApproval.set_choice(&mut s, 1));
        assert_eq!(SettingKey::DefaultApproval.stepped(&s, 1), Some(0));
        assert_eq!(SettingKey::DefaultApproval.stepped(&s, -1), Some(0));
        assert_eq!(SettingKey::Animation.stepped(&s, 1), None);
    }

    #[test]
    fn toggles_resets_and_the_changed_count() {
        let mut s = settings_t::default();
        assert_eq!(changed_count(&s), 0);
        for key in SettingKeys {
            if let Some(on) = key.toggle(&s) {
                assert!(key.set_toggle(&mut s, !on));
                assert_eq!(key.toggle(&s), Some(!on));
                assert!(!key.is_default(&s));
            }
        }
        assert!(!SettingKey::Theme.set_toggle(&mut s, true));
        s.theme = "light".to_string();
        s.open_tabs = vec!["s_1".to_string()];
        let toggles = SettingKeys
            .iter()
            .filter(|k| k.toggle(&s).is_some())
            .count();
        assert_eq!(changed_count(&s), toggles + 1);
        SettingKey::Theme.reset(&mut s);
        assert_eq!(s.theme, "ncagent-dark");
        reset_all(&mut s);
        assert_eq!(changed_count(&s), 0);
        assert_eq!(s.open_tabs, vec!["s_1".to_string()], "the tabs stay");
    }

    #[test]
    fn sections_step_and_clamp() {
        assert_eq!(Section::Appearance.step(-1), Section::Appearance);
        assert_eq!(Section::Appearance.step(1), Section::Sidebar);
        assert_eq!(Section::About.step(3), Section::About);
        assert_eq!(Section::Themes.index(), 4);
    }
}
