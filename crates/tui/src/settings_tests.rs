/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Settings page tests (D63 to D65): the page drawn on ratatui's `TestBackend` and driven by
//! synthetic crossterm events against the scripted fixture. Each setting is changed by key and
//! by mouse, and the test checks that the UI behaves differently afterwards, not only that the
//! value changed.

use std::collections::BTreeMap;

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseEventKind};
use harness::api::SessionError;
use harness::approval::ApprovalMode;
use harness::settings::{
    ConversationOrder, Density, OpenIn, SettingsError, SidebarVisibility, SidebarWidth,
    ToolResultView, custom_theme_t, settings_t,
};
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::app::EscapeDelayMs;
use crate::app_tests::{keyboard_path, rig_t, start_turn};
use crate::builtin_themes::BuiltinThemes;
use crate::folder_tests::{ok, saved, session_index, toast};
use crate::input::{Action, Focus, TabAction};
use crate::layout::SidebarMode;
use crate::settings_page::{PageAction, PageConfirm, PageRow, Pane};
use crate::settings_spec::{Section, Sections, SettingKey};
use crate::sidebar::{SettingsButton, SidebarItemKind};
use crate::view_model::tests::full_session;

fn page(a: PageAction) -> Action {
    Action::Page(a)
}

fn ctrl(rig: &mut rig_t, c: char) {
    rig.key(KeyCode::Char(c), KeyModifiers::CONTROL);
}

fn esc(rig: &mut rig_t) {
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs + 1);
}

fn shown(rig: &rig_t) -> bool {
    rig.app.page_shown && rig.app.page.is_some()
}

fn section(rig: &rig_t) -> Option<Section> {
    rig.app.page.as_ref().map(|p| p.section)
}

fn cursor(rig: &rig_t) -> usize {
    rig.app
        .page
        .as_ref()
        .map(|p| p.cursor)
        .unwrap_or(usize::MAX)
}

fn rows(rig: &rig_t) -> Vec<PageRow> {
    rig.app.page_rows()
}

fn row_index(rig: &rig_t, row: &PageRow) -> usize {
    rows(rig)
        .iter()
        .position(|r| r == row)
        .unwrap_or_else(|| panic!("no row {row:?} in {:?}", rows(rig)))
}

fn setting_row(rig: &rig_t, key: SettingKey) -> usize {
    row_index(rig, &PageRow::Setting(key))
}

/// Open the page with `ctrl+o`, show `s` with its digit and move the keys to its rows.
fn open_at(rig: &mut rig_t, s: Section) {
    ctrl(rig, 'o');
    let digit = char::from(b'1' + s.index() as u8);
    rig.ch(digit);
    rig.key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(section(rig), Some(s));
    assert_eq!(rig.app.page.as_ref().map(|p| p.pane), Some(Pane::Rows));
}

/// Walk the cursor to `row` with `j` and `k`.
fn cursor_to(rig: &mut rig_t, row: usize) {
    for _ in 0..60 {
        let at = cursor(rig);
        if at == row {
            return;
        }
        rig.ch(if at > row { 'k' } else { 'j' });
    }
    panic!("the cursor did not reach row {row}\n{}", rig.screen());
}

/// Open `s`, walk to the row of `key`.
fn at_setting(rig: &mut rig_t, key: SettingKey) {
    open_at(rig, key.section());
    let row = setting_row(rig, key);
    cursor_to(rig, row);
}

/// The text of the sidebar column, line by line, trimmed.
fn side_lines(rig: &rig_t) -> Vec<String> {
    let side = rig.app.layout.sidebar;
    rig.screen()
        .lines()
        .map(|l| {
            l.chars()
                .skip(usize::from(side.x))
                .take(usize::from(side.width))
                .collect::<String>()
                .trim()
                .to_string()
        })
        .collect()
}

fn side_has_label(rig: &rig_t, label: &str) -> bool {
    side_lines(rig).iter().any(|l| l.starts_with(label))
}

/// The first (topmost) rectangle with this action: the first line of a row.
fn first_rect(rig: &rig_t, action: &Action) -> Rect {
    rig.app
        .hits
        .hits
        .iter()
        .filter(|h| h.target == crate::input::Target::Action(action.clone()))
        .map(|h| h.rect)
        .min_by_key(|r| r.y)
        .unwrap_or_else(|| panic!("no {action:?}\n{}", rig.screen()))
}

fn saved_settings(rig: &rig_t) -> settings_t {
    ok(rig.rt.saved_settings())
}

fn rig(width: u16, height: u16) -> rig_t {
    rig_t::with(width, height, true, true, |rt| {
        rt.store("saved-1", saved("saved-1", "b older chat", 9_000_000));
        rt.store("saved-2", saved("saved-2", "a oldest chat", 8_000_000));
    })
}

fn custom(id: &str, extends: &str, colors: &[(&str, &str)]) -> custom_theme_t {
    custom_theme_t {
        id: id.to_string(),
        name: format!("Custom {id}"),
        extends: extends.to_string(),
        colors: colors
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
    }
}

// Opening and closing.

#[test]
fn the_page_opens_by_button_key_command_and_sidebar_key_and_closes() {
    let mut r = rig(130, 40);
    let tabs_before = r.app.sessions.len();
    // The footer button.
    r.click_action(&page(PageAction::Open));
    assert!(shown(&r));
    assert_eq!(r.app.focus, Focus::Settings);
    assert!(
        r.screen()
            .lines()
            .next()
            .is_some_and(|l| l.contains("⚙ settings"))
    );
    assert_eq!(
        r.app.sessions.len(),
        tabs_before,
        "the page is not a conversation"
    );
    // Opening again keeps the one page.
    ctrl(&mut r, 'o');
    r.key(KeyCode::F(2), KeyModifiers::NONE);
    assert_eq!(r.screen().matches("⚙ settings ×").count(), 1);
    esc(&mut r);
    assert!(r.app.page.is_none() && !r.app.page_shown);
    assert_eq!(r.app.focus, Focus::Input);
    assert!(
        r.screen().contains("Message ncagent"),
        "the conversation is back"
    );
    // F2, /settings and the sidebar's s.
    r.key(KeyCode::F(2), KeyModifiers::NONE);
    assert!(shown(&r));
    r.click_action(&page(PageAction::Close));
    assert!(r.app.page.is_none());
    r.typed("/settings");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(shown(&r));
    r.click_action(&page(PageAction::Close));
    r.app.apply(Action::SetFocus(Focus::Sidebar));
    r.ch('s');
    assert!(shown(&r));
    // The page is never a saved tab.
    let saved = saved_settings(&r);
    assert!(saved.open_tabs.iter().all(|t| r.app.tab_of(t).is_some()));
    assert_eq!(saved.open_tabs.len(), r.app.sessions.len());
}

#[test]
fn the_composer_is_hidden_and_inert_while_the_page_shows() {
    let mut r = rig(130, 40);
    r.typed("my words");
    ctrl(&mut r, 'o');
    let screen = r.screen();
    assert!(
        !screen.contains("my words") && !screen.contains("Message ncagent"),
        "{screen}"
    );
    r.typed("xyz");
    assert_eq!(
        r.app.composer.text(),
        "my words",
        "typing went to the page, not the input"
    );
    // A paste outside the import prompt does nothing.
    r.app.handle_event(&Event::Paste("pasted".into()));
    assert_eq!(r.app.composer.text(), "my words");
    r.click_action(&page(PageAction::Close));
    assert!(r.screen().contains("my words"));
}

#[test]
fn conversation_tabs_and_the_page_tab_trade_the_view() {
    let mut r = rig(130, 40);
    ctrl(&mut r, 't');
    assert_eq!(r.app.active, 1);
    ctrl(&mut r, 'o');
    // A conversation tab hides the page; the page tab brings it back.
    r.app.apply(Action::Tab(TabAction::Select(0)));
    r.frame();
    assert!(!r.app.page_shown && r.app.page.is_some());
    assert!(r.screen().contains("Message ncagent"));
    r.click_action(&page(PageAction::Show));
    assert!(shown(&r));
    // alt+. walks past the last tab onto the page, and alt+w closes it.
    r.app.apply(Action::Tab(TabAction::Select(1)));
    r.key(KeyCode::Char('.'), KeyModifiers::ALT);
    assert!(shown(&r));
    r.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert!(r.app.page.is_none());
    assert_eq!(r.app.sessions.len(), 2, "alt+w closed the page, not a tab");
    assert_eq!(
        r.app.active, 1,
        "the conversation under the page is shown again"
    );
}

#[test]
fn the_footer_button_is_a_target_docked_and_in_the_drawer() {
    for width in [130u16, 80] {
        let mut r = rig(width, 40);
        if width < 100 {
            ctrl(&mut r, 'b');
            assert_eq!(r.app.layout.sidebar_mode(), SidebarMode::Drawer);
        }
        let side = r.app.layout.sidebar;
        let button = r.rect_of(&page(PageAction::Open));
        assert!(
            button.x >= side.x && button.right() <= side.right(),
            "{button:?} {side:?}"
        );
        assert_eq!(button.y, side.bottom() - 1, "the last row of the column");
        assert!(button.right() >= side.right() - 2, "at the right");
        let lines = r.screen();
        let row = lines.lines().nth(usize::from(button.y)).unwrap_or("");
        assert!(row.contains(SettingsButton.trim()), "{row}");
        assert!(
            row.contains("ncagent"),
            "the version stays at the left: {row}"
        );
        r.click(button.x + 1, button.y);
        assert!(shown(&r), "{width}");
        assert_ne!(
            r.app.layout.sidebar_mode(),
            SidebarMode::Drawer,
            "the drawer closes"
        );
    }
}

// Rendering.

#[test]
fn every_section_renders_at_every_width() {
    for width in [130u16, 80, 50, 40] {
        let mut r = rig(width, 60);
        ctrl(&mut r, 'o');
        for s in Sections {
            r.app.apply(page(PageAction::Section(s)));
            r.frame();
            let screen = r.screen();
            let about: Vec<&str> = s.about().split_whitespace().take(3).collect();
            assert!(screen.contains(&about.join(" ")), "{s:?} {width}\n{screen}");
            assert!(screen.contains(s.label()), "{s:?} {width}");
            let first = rows(&r).into_iter().next();
            match first {
                Some(PageRow::Setting(key)) => {
                    assert!(screen.contains(key.label()), "{key:?} {width}\n{screen}")
                }
                Some(PageRow::Theme(_)) => assert!(screen.contains("Dark")),
                Some(PageRow::Fact(_)) => assert!(screen.contains("ui-preview")),
                other => panic!("{other:?}"),
            }
            // Below 80 columns the sections are a strip above the rows.
            let split = r.rect_of(&page(PageAction::Section(Section::About)));
            if width >= 80 {
                assert!(split.width >= 20, "a column");
            } else {
                assert!(split.width < 12, "a chip");
            }
        }
    }
}

#[test]
fn toggle_rows_show_on_off_hover_cursor_and_read_only() {
    let mut r = rig(130, 50);
    open_at(&mut r, Section::Sidebar);
    let row = setting_row(&r, SettingKey::ShowContext);
    let chip = r.rect_of(&page(PageAction::Toggle(row)));
    let theme = r.app.theme.clone();
    let cell = |r: &rig_t, x: u16, y: u16| {
        r.term
            .backend()
            .buffer()
            .cell((x, y))
            .map(|c| (c.symbol().to_string(), c.fg, c.bg))
            .unwrap_or_default()
    };
    let screen_row = |r: &rig_t, y: u16| {
        r.screen()
            .lines()
            .nth(usize::from(y))
            .unwrap_or("")
            .to_string()
    };
    assert!(screen_row(&r, chip.y).contains("◉ on"));
    assert_eq!(cell(&r, chip.x + 1, chip.y).1, theme.success);
    // The cursor row is on the select tone with an ice rail; another row is not.
    cursor_to(&mut r, row);
    let band_x = r.app.layout.main.x + 23;
    assert_eq!(cell(&r, band_x, chip.y).2, theme.select);
    assert_eq!(cell(&r, band_x, chip.y).0, "▌");
    assert_eq!(cell(&r, band_x, chip.y).1, theme.ice);
    // Hover lifts the chip.
    r.mouse(MouseEventKind::Moved, chip.x + 1, chip.y);
    assert_eq!(cell(&r, chip.x + 1, chip.y).2, theme.select);
    // Off.
    r.click(chip.x + 1, chip.y);
    assert!(!r.app.settings.show_context);
    let chip = r.rect_of(&page(PageAction::Toggle(row)));
    assert!(screen_row(&r, chip.y).contains("○ off"));
    assert!(
        screen_row(&r, chip.y).contains("↺ reset"),
        "a changed value offers reset"
    );

    // Read-only: the command line turned animation off.
    let mut r = rig_t::new(130, 50, false);
    open_at(&mut r, Section::Appearance);
    let row = setting_row(&r, SettingKey::Animation);
    assert!(r.screen().contains("○ off · by command line"));
    assert!(r.app.hits.find(&page(PageAction::Toggle(row))).is_none());
    cursor_to(&mut r, row);
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(toast(&r).contains("command line"), "{}", toast(&r));
    assert!(r.app.settings.animation, "the setting is untouched");
    assert!(!r.app.animate);
}

#[test]
fn choice_rows_show_the_chosen_option_and_the_disabled_one() {
    let mut r = rig(130, 50);
    open_at(&mut r, Section::Conversations);
    let row = setting_row(&r, SettingKey::DefaultApproval);
    let theme = r.app.theme.clone();
    let ask = r.rect_of(&page(PageAction::Pick(row, 0)));
    let edit = r.rect_of(&page(PageAction::Pick(row, 1)));
    let buf = r.term.backend().buffer();
    assert_eq!(
        buf.cell((ask.x + 1, ask.y)).map(|c| c.bg),
        Some(theme.agent)
    );
    assert_eq!(
        buf.cell((edit.x + 1, edit.y)).map(|c| c.bg),
        Some(theme.inset)
    );
    assert!(r.screen().contains("full · command line only"));
    assert!(
        r.app.hits.find(&page(PageAction::Pick(row, 2))).is_none(),
        "full is not a target"
    );
    // Keys skip full: → twice from ask lands on ask again.
    cursor_to(&mut r, row);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(r.app.settings.default_approval, ApprovalMode::Edit);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(r.app.settings.default_approval, ApprovalMode::Ask);
}

#[test]
fn the_theme_list_shows_a_swatch_per_theme() {
    let mut r = rig_t::with(130, 70, true, true, |rt| {
        ok(rt.seed_theme(&custom("deep", "navy", &[("agent", "#00ffaa")])));
    });
    open_at(&mut r, Section::Themes);
    let screen = r.screen();
    for b in BuiltinThemes {
        assert!(screen.contains(b.name), "{}\n{screen}", b.name);
    }
    assert!(screen.contains("Custom deep") && screen.contains("imported"));
    assert!(screen.contains("deep · extends navy"));
    assert!(screen.contains("● in use"));
    assert!(
        r.app
            .hits
            .find(&page(PageAction::RemoveTheme("deep".into())))
            .is_some()
    );
    assert!(
        r.app
            .hits
            .find(&page(PageAction::RemoveTheme("light".into())))
            .is_none()
    );
    // The light row's swatch: two cells per key colour, in order.
    let apply = first_rect(&r, &page(PageAction::ApplyTheme("light".into())));
    let light = crate::builtin_themes::light();
    let buf = r.term.backend().buffer();
    let pane_x = r.app.layout.main.x + 25;
    for (i, name) in crate::settings_draw::SwatchColors.iter().enumerate() {
        let x = pane_x + 2 * i as u16;
        assert_eq!(
            buf.cell((x, apply.y)).map(|c| c.bg),
            light.get(name),
            "{name}"
        );
    }
}

#[test]
fn moving_or_hovering_over_a_theme_previews_it_and_leaving_reverts() {
    let mut r = rig(130, 60);
    open_at(&mut r, Section::Themes);
    let light = row_index(&r, &PageRow::Theme("light".into()));
    cursor_to(&mut r, light);
    assert_eq!(
        r.app.theme,
        crate::builtin_themes::light(),
        "the whole UI previews"
    );
    assert_eq!(r.app.settings.theme, "ncagent-dark", "nothing is saved");
    assert!(r.screen().contains("◌ preview"));
    let buf = r.term.backend().buffer();
    let right = r.app.layout.main.right() - 1;
    assert_eq!(
        buf.cell((right, 3)).map(|c| c.bg),
        Some(crate::builtin_themes::light().bg)
    );
    // The sidebar and the tab bar follow too.
    let side = r.app.layout.sidebar;
    assert_eq!(
        buf.cell((side.x + 1, side.y + 5)).map(|c| c.bg),
        Some(crate::builtin_themes::light().side)
    );
    // Moving away reverts.
    r.ch('k');
    assert_ne!(r.app.theme, crate::builtin_themes::light());
    // The pointer over a theme previews it, wherever the cursor is.
    let astral = r.rect_of(&page(PageAction::ApplyTheme("astral".into())));
    r.mouse(MouseEventKind::Moved, astral.x + 30, astral.y);
    assert_eq!(r.app.theme, crate::builtin_themes::astral());
    r.mouse(MouseEventKind::Moved, 0, 0);
    assert_eq!(
        r.app.theme,
        crate::theme::theme_t::dark(),
        "the cursor row again"
    );
    // esc closes the page and the preview with it.
    esc(&mut r);
    assert!(r.app.page.is_none());
    assert_eq!(r.app.theme, crate::theme::theme_t::dark());
    assert_eq!(saved_settings(&r).theme, "ncagent-dark");
}

#[test]
fn hovering_a_theme_chip_in_appearance_previews_it() {
    let mut r = rig(130, 50);
    open_at(&mut r, Section::Appearance);
    let row = setting_row(&r, SettingKey::Theme);
    let navy = BuiltinThemes
        .iter()
        .position(|b| b.id == "navy")
        .unwrap_or(0);
    let chip = r.rect_of(&page(PageAction::Pick(row, navy)));
    r.mouse(MouseEventKind::Moved, chip.x + 1, chip.y);
    assert_eq!(r.app.theme, crate::builtin_themes::navy());
    r.click(chip.x + 1, chip.y);
    assert_eq!(r.app.settings.theme, "navy");
    assert_eq!(saved_settings(&r).theme, "navy");
    r.mouse(MouseEventKind::Moved, 0, 0);
    assert_eq!(
        r.app.theme,
        crate::builtin_themes::navy(),
        "chosen, so it stays"
    );
}

#[test]
fn the_import_prompt_renders_and_takes_typing_and_paste() {
    let mut r = rig(80, 50);
    open_at(&mut r, Section::Themes);
    r.click_action(&page(PageAction::Press(row_index(&r, &PageRow::Import))));
    assert!(r.app.page.as_ref().is_some_and(|p| p.prompt.is_some()));
    let screen = r.screen();
    assert!(screen.contains("path to a theme file"), "{screen}");
    assert!(screen.contains("▸"), "{screen}");
    assert!(screen.contains("enter import") && screen.contains("esc cancel"));
    r.typed("/tmp/a b");
    r.app.handle_event(&Event::Paste(".json".into()));
    r.frame();
    let text = r
        .app
        .page
        .as_ref()
        .and_then(|p| p.prompt.as_ref())
        .map(|p| p.text().to_string());
    assert_eq!(text.as_deref(), Some("/tmp/a b.json"));
    assert!(r.screen().contains("/tmp/a b.json"));
    // Keys do not reach the rows while the prompt is open; esc closes only the prompt.
    r.ch('j');
    assert!(r.screen().contains("/tmp/a b.jsonj"));
    esc(&mut r);
    assert!(shown(&r));
    assert!(r.app.page.as_ref().is_some_and(|p| p.prompt.is_none()));
}

// Each setting changes behaviour.

#[test]
fn the_sidebar_setting_changes_the_docking_rule() {
    let mut r = rig(96, 40);
    assert_eq!(
        r.app.layout.sidebar_mode(),
        SidebarMode::Hidden,
        "auto: no dock under 100"
    );
    at_setting(&mut r, SettingKey::Sidebar);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(r.app.settings.sidebar, SidebarVisibility::Pinned);
    assert_eq!(
        r.app.layout.sidebar_mode(),
        SidebarMode::Docked,
        "pinned docks at 96"
    );
    let row = setting_row(&r, SettingKey::Sidebar);
    r.click_action(&page(PageAction::Pick(row, 2)));
    assert_eq!(r.app.settings.sidebar, SidebarVisibility::Hidden);
    r.resize(160, 40);
    assert_eq!(
        r.app.layout.sidebar_mode(),
        SidebarMode::Hidden,
        "hidden never docks"
    );
    // ctrl+b still opens it, as a drawer.
    ctrl(&mut r, 'b');
    assert_eq!(r.app.layout.sidebar_mode(), SidebarMode::Drawer);
    assert_eq!(saved_settings(&r).sidebar, SidebarVisibility::Hidden);
}

#[test]
fn the_sidebar_width_setting_changes_the_column() {
    let mut r = rig(130, 40);
    assert_eq!(r.app.layout.sidebar.width, 34);
    at_setting(&mut r, SettingKey::SidebarWidth);
    r.key(KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(r.app.settings.sidebar_width, SidebarWidth::Compact);
    assert_eq!(r.app.layout.sidebar.width, 28);
    let row = setting_row(&r, SettingKey::SidebarWidth);
    r.click_action(&page(PageAction::Pick(row, 2)));
    assert_eq!(r.app.layout.sidebar.width, 42);
    r.click_action(&page(PageAction::Pick(row, 1)));
    r.resize(160, 40);
    assert_eq!(
        r.app.layout.sidebar.width, 38,
        "normal grows from 150 columns"
    );
}

#[test]
fn each_sidebar_section_can_be_hidden() {
    let mut r = rig(130, 50);
    let cases = [
        (SettingKey::ShowContext, "context"),
        (SettingKey::ShowTurn, "turn"),
        (SettingKey::ShowGeneration, "generation"),
        (SettingKey::ShowModel, "model"),
    ];
    for (_, label) in cases {
        assert!(side_has_label(&r, label), "{label}: {:?}", side_lines(&r));
    }
    open_at(&mut r, Section::Sidebar);
    for (i, (key, label)) in cases.iter().enumerate() {
        let row = setting_row(&r, *key);
        if i % 2 == 0 {
            r.click_action(&page(PageAction::Toggle(row)));
        } else {
            cursor_to(&mut r, row);
            r.key(KeyCode::Enter, KeyModifiers::NONE);
        }
        assert!(
            !side_has_label(&r, label),
            "{label} hidden: {:?}",
            side_lines(&r)
        );
    }
    assert!(side_has_label(&r, "conversations"), "the list stays");
    let s = saved_settings(&r);
    assert!(!s.show_context && !s.show_turn && !s.show_generation && !s.show_model);
}

#[test]
fn row_details_can_be_hidden() {
    let mut r = rig(130, 40);
    let i = session_index(&r, "saved-1");
    assert!(
        r.app
            .sidebar_items
            .get(i)
            .is_some_and(|it| it.detail.ends_with("1t"))
    );
    assert!(
        side_lines(&r)
            .iter()
            .any(|l| l.contains("b older chat") && l.contains("1t"))
    );
    at_setting(&mut r, SettingKey::ShowRowDetails);
    r.key(KeyCode::Char(' '), KeyModifiers::NONE);
    assert!(!r.app.settings.show_row_details);
    assert!(
        r.app
            .sidebar_items
            .iter()
            .all(|it| !matches!(it.kind, SidebarItemKind::Session { .. }) || it.detail.is_empty())
    );
    assert!(
        !side_lines(&r)
            .iter()
            .any(|l| l.contains("b older chat") && l.contains("1t"))
    );
}

#[test]
fn compact_density_tightens_the_transcript_the_dock_and_the_sidebar() {
    let mut r = rig(130, 50);
    r.push(full_session().into_iter().map(|s| s.event).collect());
    let total = |r: &rig_t| r.app.active().map(|s| s.view.total).unwrap_or(0);
    let comfortable = total(&r);
    let dock = r.app.layout.dock.height;
    let new_chat_row = |r: &rig_t| r.app.hits.find(&Action::NewChat).map(|x| x.y);
    let before = new_chat_row(&r);
    at_setting(&mut r, SettingKey::Density);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(r.app.settings.density, Density::Compact);
    assert!(new_chat_row(&r) < before, "the sidebar moves up");
    r.click_action(&page(PageAction::Close));
    let blocks = r
        .app
        .active()
        .map(|s| s.transcript.blocks.len())
        .unwrap_or(0);
    assert_eq!(
        total(&r) + blocks,
        comfortable,
        "one gap row less per block"
    );
    assert_eq!(r.app.layout.dock.height + 2, dock, "no padding in the dock");
}

#[test]
fn the_tool_results_setting_opens_tool_blocks_and_toggles_still_work() {
    let mut r = rig(130, 60);
    r.push(full_session().into_iter().map(|s| s.event).collect());
    let height = |r: &rig_t| {
        let s = r.app.active().unwrap_or_else(|| panic!("session"));
        let tool = s
            .transcript
            .blocks
            .iter()
            .find(|b| matches!(b.kind, crate::view_model::BlockKind::Tool(_)))
            .map(|b| b.id)
            .unwrap_or(u64::MAX);
        (
            tool,
            s.view.rows_of(tool).map(|x| x.rows.len()).unwrap_or(0),
        )
    };
    let (tool, folded) = height(&r);
    at_setting(&mut r, SettingKey::ToolResults);
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(r.app.settings.tool_results, ToolResultView::Expanded);
    r.click_action(&page(PageAction::Close));
    let (_, open) = height(&r);
    assert!(open > folded, "{open} > {folded}");
    // The block's own toggle folds it back.
    r.click_action(&Action::ToggleBlock(tool));
    assert_eq!(height(&r).1, folded);
    // A replayed conversation opens with its tools expanded too.
    let mut opened = rig_t::with(130, 60, false, true, |rt| {
        ok(rt.seed_settings(&settings_t {
            tool_results: ToolResultView::Expanded,
            ..settings_t::default()
        }));
        let events = full_session()
            .into_iter()
            .map(|mut s| {
                if let harness::event::SessionEvent::SessionStart { session_id, .. } = &mut s.event
                {
                    *session_id = "saved-tools".into();
                }
                s
            })
            .collect();
        rt.store("saved-tools", events);
    });
    let i = session_index(&opened, "saved-tools");
    opened.click_row(i);
    assert!(opened.app.tab_of("saved-tools").is_some());
    let screen = opened.screen();
    assert!(screen.contains("enter or click to fold"), "{screen}");
    assert!(!screen.contains("enter or click to expand"), "{screen}");
}

#[test]
fn the_animation_setting_and_animate_command_switch_the_effect() {
    let mut r = rig(130, 40);
    assert!(r.app.animate);
    at_setting(&mut r, SettingKey::Animation);
    r.key(KeyCode::Left, KeyModifiers::NONE);
    assert!(!r.app.settings.animation && !r.app.animate);
    assert!(!saved_settings(&r).animation);
    r.click_action(&page(PageAction::Close));
    r.typed("/animate");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        r.app.animate && saved_settings(&r).animation,
        "/animate saves"
    );
    // A new chat plays its logo only with animation on.
    r.typed("/animate");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    ctrl(&mut r, 't');
    assert!(r.app.active().is_some_and(|s| s.logo_from.is_none()));
    // At start the setting decides too.
    let off = rig_t::with(130, 40, true, true, |rt| {
        ok(rt.seed_settings(&settings_t {
            animation: false,
            ..settings_t::default()
        }));
    });
    assert!(!off.app.animate);
    assert!(off.app.active().is_some_and(|s| s.logo_from.is_none()));
    // The command line wins over the setting.
    let mut forced = rig_t::new(130, 40, false);
    forced.typed("/animate");
    forced.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(!forced.app.animate && forced.app.settings.animation);
    assert!(toast(&forced).contains("--speed instant"));
}

#[test]
fn the_default_approval_applies_to_new_conversations() {
    let mut r = rig(130, 40);
    let mode = |r: &rig_t| r.app.active().map(|s| s.session.approval_mode());
    assert_eq!(mode(&r), Some(ApprovalMode::Ask));
    at_setting(&mut r, SettingKey::DefaultApproval);
    let row = setting_row(&r, SettingKey::DefaultApproval);
    r.click_action(&page(PageAction::Pick(row, 1)));
    assert_eq!(saved_settings(&r).default_approval, ApprovalMode::Edit);
    ctrl(&mut r, 't');
    assert_eq!(mode(&r), Some(ApprovalMode::Edit));
    assert!(r.screen().contains("edit approval"));
}

#[test]
fn confirm_delete_off_deletes_without_the_dialog() {
    let mut r = rig(130, 40);
    let done = || harness::event::SessionEvent::TurnEnd {
        turn: 1,
        outcome: harness::event::TurnOutcome::Completed,
        calls: 0,
        passes: 0,
    };
    let mut events = start_turn("to delete");
    events.push(done());
    r.push(events);
    r.typed("/delete");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(r.app.delete_plan.is_some(), "asks by default");
    esc(&mut r);
    at_setting(&mut r, SettingKey::ConfirmDelete);
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(!r.app.settings.confirm_delete);
    r.click_action(&page(PageAction::Close));
    let id = r
        .app
        .active()
        .map(|s| s.session.id().to_string())
        .unwrap_or_default();
    r.typed("/delete");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(r.app.delete_plan.is_none());
    assert!(r.rt.deleted().contains(&id), "{:?}", r.rt.deleted());
}

#[test]
fn open_in_current_tab_replaces_the_idle_tab() {
    let mut r = rig(130, 40);
    r.push(start_turn("in use"));
    r.push(vec![harness::event::SessionEvent::TurnEnd {
        turn: 1,
        outcome: harness::event::TurnOutcome::Completed,
        calls: 0,
        passes: 0,
    }]);
    let i = session_index(&r, "saved-1");
    r.click_row(i);
    assert_eq!(r.app.sessions.len(), 2, "a new tab by default");
    at_setting(&mut r, SettingKey::OpenIn);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(r.app.settings.open_in, OpenIn::CurrentTab);
    let i = session_index(&r, "saved-2");
    r.click_row(i);
    assert!(!r.app.page_shown, "opening a conversation shows it");
    assert_eq!(r.app.sessions.len(), 2, "the shown tab was replaced");
    assert!(r.app.tab_of("saved-2").is_some() && r.app.tab_of("saved-1").is_none());
}

#[test]
fn show_archived_lists_archived_conversations() {
    let mut r = rig(130, 40);
    let i = session_index(&r, "saved-1");
    r.click_row(i);
    r.app.apply(Action::SetFocus(Focus::Input));
    r.typed("/archive");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    let listed = |r: &rig_t| {
        r.app
            .sidebar_items
            .iter()
            .any(|it| matches!(&it.kind, SidebarItemKind::Session { id } if id == "saved-1"))
    };
    // The open tab stays listed while it is shown; another tab hides it.
    r.app.apply(Action::Tab(TabAction::New));
    r.frame();
    r.app.apply(Action::Tab(TabAction::Close(
        r.app.tab_of("saved-1").unwrap_or(0),
    )));
    r.frame();
    assert!(!listed(&r), "archived is hidden");
    at_setting(&mut r, SettingKey::ShowArchived);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    assert!(listed(&r));
    r.mouse(MouseEventKind::Moved, 0, 0);
    assert!(
        side_lines(&r).iter().any(|l| l.contains("archived")),
        "{:?}",
        side_lines(&r)
    );
}

#[test]
fn the_order_setting_reorders_the_list() {
    let mut r = rig(130, 40);
    let order = |r: &rig_t| (session_index(r, "saved-1"), session_index(r, "saved-2"));
    let (b, a) = order(&r);
    assert!(b < a, "recent first: b older chat before a oldest chat");
    at_setting(&mut r, SettingKey::ConversationOrder);
    let row = setting_row(&r, SettingKey::ConversationOrder);
    r.click_action(&page(PageAction::Pick(row, 1)));
    assert_eq!(r.app.settings.conversation_order, ConversationOrder::Title);
    let (b, a) = order(&r);
    assert!(a < b, "by title: a before b");
}

#[test]
fn restore_tabs_off_stops_saving_tabs_and_on_saves_them_at_once() {
    let mut r = rig(130, 40);
    ctrl(&mut r, 't');
    assert_eq!(saved_settings(&r).open_tabs.len(), 2);
    at_setting(&mut r, SettingKey::RestoreTabs);
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(!saved_settings(&r).restore_tabs);
    r.app.apply(Action::Tab(TabAction::New));
    r.frame();
    assert_eq!(saved_settings(&r).open_tabs.len(), 2, "not saved while off");
    r.app.apply(page(PageAction::Show));
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        saved_settings(&r).open_tabs.len(),
        3,
        "saved when turned on"
    );
}

#[test]
fn a_refused_save_is_a_toast_and_the_value_stays() {
    let mut r = rig(130, 40);
    at_setting(&mut r, SettingKey::Density);
    r.rt.fail_settings(Some(SessionError::Settings(SettingsError::Io {
        path: "/x/settings.json".into(),
        message: "disk full".into(),
    })));
    r.key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(r.app.settings.density, Density::Comfortable);
    assert!(toast(&r).contains("disk full"), "{}", toast(&r));
    r.advance(crate::toast::MaxToastFxMs);
    assert!(r.screen().contains("disk full"));
    r.rt.fail_settings(None);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(r.app.settings.density, Density::Compact);
}

#[test]
fn reset_a_row_and_reset_everything() {
    let mut r = rig(130, 50);
    at_setting(&mut r, SettingKey::Density);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    r.ch('r');
    assert_eq!(r.app.settings.density, Density::Comfortable);
    r.key(KeyCode::Right, KeyModifiers::NONE);
    let row = setting_row(&r, SettingKey::Density);
    r.click_action(&page(PageAction::Reset(row)));
    assert_eq!(r.app.settings.density, Density::Comfortable);
    assert!(
        r.app.hits.find(&page(PageAction::Reset(row))).is_none(),
        "no chip at default"
    );
    // Change three, then reset all from About after a yes.
    r.key(KeyCode::Right, KeyModifiers::NONE);
    r.app.apply_theme("astral");
    r.app.change_setting(|s| s.show_archived = true);
    ctrl(&mut r, 't');
    let tabs = saved_settings(&r).open_tabs.clone();
    open_at(&mut r, Section::About);
    assert!(r.screen().contains("3 setting(s) differ"), "{}", r.screen());
    let reset = row_index(&r, &PageRow::ResetAll);
    r.click_action(&page(PageAction::Press(reset)));
    assert_eq!(
        r.app.page.as_ref().and_then(|p| p.confirm.clone()),
        Some(PageConfirm::ResetAll)
    );
    assert!(r.screen().contains("reset every setting to its default?"));
    r.ch('n');
    assert!(r.app.settings.show_archived, "n keeps them");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    let s = saved_settings(&r);
    assert_eq!(crate::settings_spec::changed_count(&s), 0);
    assert_eq!(s.open_tabs, tabs, "the tabs stay");
    assert_eq!(r.app.theme, crate::theme::theme_t::dark());
    assert!(r.screen().contains("every setting is at its default"));
}

#[test]
fn every_click_target_on_the_page_has_a_keyboard_path() {
    for width in [130u16, 60] {
        let mut r = rig_t::with(width, 60, true, true, |rt| {
            ok(rt.seed_theme(&custom("deep", "navy", &[])));
        });
        ctrl(&mut r, 'o');
        for s in Sections {
            r.app.apply(page(PageAction::Section(s)));
            r.frame();
            for hit in &r.app.hits.hits {
                assert!(keyboard_path(&hit.target), "{s:?}: {:?}", hit.target);
            }
        }
        r.app.apply(page(PageAction::Section(Section::Themes)));
        r.app
            .apply(page(PageAction::Press(row_index(&r, &PageRow::Import))));
        r.frame();
        for hit in &r.app.hits.hits {
            assert!(keyboard_path(&hit.target), "prompt: {:?}", hit.target);
        }
    }
}

// Themes.

#[test]
fn a_theme_is_applied_by_click_and_by_enter_to_the_whole_ui() {
    let mut r = rig(130, 60);
    open_at(&mut r, Section::Themes);
    r.click_action(&page(PageAction::ApplyTheme("astral".into())));
    assert_eq!(r.app.settings.theme, "astral");
    assert_eq!(saved_settings(&r).theme, "astral");
    let row = row_index(&r, &PageRow::Theme("light".into()));
    cursor_to(&mut r, row);
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(saved_settings(&r).theme, "light");
    r.click_action(&page(PageAction::Close));
    assert_eq!(r.app.theme, crate::builtin_themes::light());
    let buf = r.term.backend().buffer();
    assert_eq!(
        buf.cell((0, 3)).map(|c| c.bg),
        Some(crate::builtin_themes::light().bg)
    );
    // /theme with a name, and /theme alone opens the list.
    r.typed("/theme astral");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(r.app.settings.theme, "astral");
    r.typed("/theme nope");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(toast(&r).contains("no theme"), "{}", toast(&r));
    r.typed("/theme");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(section(&r), Some(Section::Themes));
}

/// A rig whose fixture finds theme files at `/t/<name>.json`.
fn import_rig() -> rig_t {
    rig_t::with(130, 60, false, true, |rt| {
        rt.offer_theme_file(
            "/t/good.json",
            Ok(custom(
                "good",
                "navy",
                &[("bg", "#101820"), ("agent", "#ff8800")],
            )),
        );
        rt.offer_theme_file(
            "/t/my theme.json",
            Ok(custom("spaced", "ncagent-dark", &[("tool", "#00ff00")])),
        );
        rt.offer_theme_file(
            "/t/unknown.json",
            Ok(custom(
                "unknown",
                "ncagent-dark",
                &[("backgroud", "#000000")],
            )),
        );
        rt.offer_theme_file("/t/orphan.json", Ok(custom("orphan", "gone", &[])));
        rt.offer_theme_file("/t/builtin.json", Ok(custom("light", "ncagent-dark", &[])));
        rt.offer_theme_file("/t/loop.json", Ok(custom("loop", "loop", &[])));
        rt.offer_theme_file(
            "/t/missing.json",
            Err(SessionError::Settings(SettingsError::Io {
                path: "/t/missing.json".into(),
                message: "No such file or directory".into(),
            })),
        );
    })
}

#[test]
fn importing_through_the_page_saves_and_applies_the_theme() {
    let mut r = import_rig();
    open_at(&mut r, Section::Themes);
    let import = row_index(&r, &PageRow::Import);
    cursor_to(&mut r, import);
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    // Dropped from a file manager: quoted, with a newline.
    r.app.handle_event(&Event::Paste("'/t/good.json'\n".into()));
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        r.app.page.as_ref().is_some_and(|p| p.prompt.is_none()),
        "{}",
        toast(&r)
    );
    assert_eq!(r.app.settings.theme, "good");
    assert_eq!(saved_settings(&r).theme, "good");
    let themes = ok(harness::api::runtime_t::themes(&*r.rt));
    assert_eq!(
        themes
            .themes
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["good"]
    );
    assert_eq!(r.app.theme.bg, Color::Rgb(0x10, 0x18, 0x20));
    assert!(toast(&r).contains("imported") && toast(&r).contains("applied"));
    assert!(r.screen().contains("Custom good"));
    // /theme import with a path that holds a space, typed as is.
    r.click_action(&page(PageAction::Close));
    r.typed("/theme import /t/my theme.json");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(r.app.settings.theme, "spaced", "{}", toast(&r));
}

#[test]
fn each_import_failure_is_a_toast_that_names_it_and_keeps_the_prompt() {
    let cases = [
        ("/t/missing.json", "No such file or directory"),
        ("/t/unknown.json", "\"backgroud\" is not a colour name"),
        ("/t/orphan.json", "extends \"gone\""),
        ("/t/builtin.json", "belongs to a built-in theme"),
        ("/t/loop.json", "extends loops"),
        ("/t/not-offered.json", "does not read files"),
        ("   ", "type the path"),
    ];
    for (path, needle) in cases {
        let mut r = import_rig();
        open_at(&mut r, Section::Themes);
        r.click_action(&page(PageAction::Press(row_index(&r, &PageRow::Import))));
        r.typed(path);
        r.key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(toast(&r).contains(needle), "{path}: {}", toast(&r));
        assert!(
            r.app.page.as_ref().is_some_and(|p| p.prompt.is_some()),
            "{path}: the prompt stays"
        );
        assert_eq!(r.app.settings.theme, "ncagent-dark");
        assert!(
            ok(harness::api::runtime_t::themes(&*r.rt))
                .themes
                .is_empty()
        );
    }
}

#[test]
fn an_imported_theme_is_removed_after_a_yes() {
    let mut r = rig_t::with(130, 60, false, true, |rt| {
        ok(rt.seed_theme(&custom("deep", "navy", &[])));
        ok(rt.seed_settings(&settings_t {
            theme: "deep".to_string(),
            ..settings_t::default()
        }));
    });
    assert_eq!(r.app.theme, crate::builtin_themes::navy());
    open_at(&mut r, Section::Themes);
    r.click_action(&page(PageAction::RemoveTheme("deep".into())));
    assert!(
        r.screen().contains("remove “Custom deep”?"),
        "{}",
        r.screen()
    );
    r.ch('n');
    assert_eq!(ok(harness::api::runtime_t::themes(&*r.rt)).themes.len(), 1);
    let row = row_index(&r, &PageRow::Theme("deep".into()));
    cursor_to(&mut r, row);
    r.ch('d');
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        ok(harness::api::runtime_t::themes(&*r.rt))
            .themes
            .is_empty()
    );
    assert_eq!(saved_settings(&r).theme, "ncagent-dark", "it was in use");
    assert_eq!(
        ok(r.app.themes.resolve(&r.app.settings.theme)),
        crate::theme::theme_t::dark()
    );
    assert!(toast(&r).contains("removed"));
    // A built-in cannot be removed.
    let light = row_index(&r, &PageRow::Theme("light".into()));
    cursor_to(&mut r, light);
    r.ch('d');
    assert!(toast(&r).contains("only an imported theme"));
}

#[test]
fn a_bad_theme_at_start_falls_back_once_and_keeps_the_settings() {
    let r = rig_t::with(130, 40, false, true, |rt| {
        ok(rt.seed_settings(&settings_t {
            theme: "gone-theme".to_string(),
            density: Density::Compact,
            ..settings_t::default()
        }));
        rt.skip_theme_files(vec![
            "bad.json: the file is not valid: expected value".into(),
        ]);
    });
    assert_eq!(r.app.theme, crate::theme::theme_t::dark());
    let t = toast(&r);
    assert!(
        t.contains("gone-theme") && t.contains("ncagent-dark"),
        "{t}"
    );
    assert!(t.contains("1 theme file(s) ignored: bad.json"), "{t}");
    assert_eq!(saved_settings(&r).theme, "gone-theme", "not rewritten");
    assert_eq!(
        r.app.settings.density,
        Density::Compact,
        "the rest still applies"
    );
    // A broken imported theme in use falls back the same way.
    let r = rig_t::with(130, 40, false, true, |rt| {
        ok(rt.seed_theme(&custom("broken", "gone", &[])));
        ok(rt.seed_settings(&settings_t {
            theme: "broken".to_string(),
            ..settings_t::default()
        }));
    });
    assert_eq!(r.app.theme, crate::theme::theme_t::dark());
    assert!(toast(&r).contains("broken"));
}

#[test]
fn page_keys_walk_sections_rows_and_the_sidebar() {
    let mut r = rig(130, 40);
    ctrl(&mut r, 'o');
    assert_eq!(r.app.page.as_ref().map(|p| p.pane), Some(Pane::Rows));
    r.key(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(r.app.page.as_ref().map(|p| p.pane), Some(Pane::List));
    r.ch('j');
    r.ch('j');
    assert_eq!(section(&r), Some(Section::Conversations));
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(r.app.page.as_ref().map(|p| p.pane), Some(Pane::Rows));
    r.key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(r.app.focus, Focus::Sidebar, "tab goes on to the sidebar");
    r.key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(r.app.focus, Focus::Settings);
    assert_eq!(r.app.page.as_ref().map(|p| p.pane), Some(Pane::List));
    // About: the cursor skips the facts to the reset row.
    r.ch('6');
    r.key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(rows(&r).get(cursor(&r)), Some(&PageRow::ResetAll));
    r.ch('k');
    assert_eq!(rows(&r).get(cursor(&r)), Some(&PageRow::ResetAll));
    // The wheel scrolls the rows.
    let before = r.app.page.as_ref().map(|p| p.scroll);
    r.resize(130, 14);
    r.mouse(MouseEventKind::ScrollDown, 60, 6);
    assert_ne!(r.app.page.as_ref().map(|p| p.scroll), before);
    // esc with the sidebar focused closes the page, not the turn.
    r.push(start_turn("busy"));
    r.app.apply(Action::SetFocus(Focus::Sidebar));
    esc(&mut r);
    assert!(r.app.page.is_none());
    assert!(r.app.busy(), "the turn under the page still runs");
}

#[test]
fn the_about_section_shows_the_facts_and_the_demo_notice() {
    let mut r = rig(130, 60);
    open_at(&mut r, Section::About);
    let screen = r.screen();
    for needle in [
        "ui-preview",
        "simulated, not NCM output",
        "(always shown)",
        "preview state is kept in memory only",
        "P 4096",
        "24-bit",
        "read-only facts",
    ] {
        assert!(screen.contains(needle), "{needle}\n{screen}");
    }
    let notice = r.app.info.notice.clone().unwrap_or_default();
    assert!(screen.contains(&notice[..notice.len().min(20)]));
}
