/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The built-in themes (PROJECT.md, D64): their ids, names and palettes. `ncagent-dark` is the
//! look of D47 and the default. Every built-in sets every colour, so none depends on the
//! derivation rules that custom themes use (see `theme::derive`).

use ratatui::style::Color;

use crate::theme::theme_t;

/// A theme that ships with ncagent.
#[derive(Clone, Copy, Debug)]
pub struct builtin_theme_t {
    pub id: &'static str,
    pub name: &'static str,
    pub palette: fn() -> theme_t,
}

/// The built-in themes, in the order the settings page lists them. The first is the default.
/// The ids come from this registry; they are not checked against the harness id rule.
pub const BuiltinThemes: [builtin_theme_t; 7] = [
    builtin_theme_t {
        id: "ncagent-dark",
        name: "Dark",
        palette: ncagent_dark,
    },
    builtin_theme_t {
        id: "light",
        name: "Light",
        palette: light,
    },
    builtin_theme_t {
        id: "navy",
        name: "Navy",
        palette: navy,
    },
    builtin_theme_t {
        id: "astral",
        name: "Astral",
        palette: astral,
    },
    builtin_theme_t {
        id: "ember",
        name: "Ember",
        palette: ember,
    },
    builtin_theme_t {
        id: "mono",
        name: "Mono",
        palette: mono,
    },
    builtin_theme_t {
        id: "high_contrast",
        name: "High contrast",
        palette: high_contrast,
    },
];

/// The built-in theme with this id.
pub fn builtin(id: &str) -> Option<&'static builtin_theme_t> {
    BuiltinThemes.iter().find(|t| t.id == id)
}

/// `ncagent-dark`, "Dark": white on black with ice-blue generation, orchid tools and amber
/// approvals (D47). The default.
pub fn ncagent_dark() -> theme_t {
    theme_t {
        bg: Color::Rgb(0, 0, 0),
        side: Color::Rgb(12, 12, 15),
        panel: Color::Rgb(22, 22, 27),
        dock: Color::Rgb(19, 19, 24),
        inset: Color::Rgb(30, 30, 37),
        hover: Color::Rgb(34, 34, 42),
        select: Color::Rgb(40, 40, 52),
        marked: Color::Rgb(20, 36, 58),
        fg: Color::Rgb(236, 236, 240),
        fg_dim: Color::Rgb(146, 146, 158),
        fg_faint: Color::Rgb(84, 84, 96),
        user: Color::Rgb(250, 250, 252),
        agent: Color::Rgb(142, 202, 255),
        ice: Color::Rgb(196, 228, 255),
        ice_deep: Color::Rgb(58, 92, 128),
        tool: Color::Rgb(226, 118, 236),
        tool_dim: Color::Rgb(122, 70, 132),
        approval: Color::Rgb(255, 182, 72),
        question: Color::Rgb(252, 212, 112),
        error: Color::Rgb(255, 94, 98),
        success: Color::Rgb(112, 228, 152),
        composer_rail: Color::Rgb(176, 148, 255),
        code: Color::Rgb(236, 206, 164),
        link: Color::Rgb(142, 202, 255),
        logo_hi: Color::Rgb(255, 255, 255),
        logo_lo: Color::Rgb(116, 176, 238),
        glint: Color::Rgb(246, 249, 255),
    }
}

/// `light`, "Light": dark ink on warm paper. The tone steps go darker instead of lighter, and
/// every accent is deep enough to read on a light background.
pub fn light() -> theme_t {
    theme_t {
        bg: Color::Rgb(250, 248, 243),
        side: Color::Rgb(241, 238, 231),
        panel: Color::Rgb(234, 231, 222),
        dock: Color::Rgb(237, 234, 226),
        inset: Color::Rgb(226, 222, 212),
        hover: Color::Rgb(221, 217, 206),
        select: Color::Rgb(210, 214, 228),
        marked: Color::Rgb(212, 224, 242),
        fg: Color::Rgb(30, 29, 36),
        fg_dim: Color::Rgb(88, 86, 98),
        fg_faint: Color::Rgb(136, 133, 142),
        user: Color::Rgb(18, 18, 24),
        agent: Color::Rgb(28, 94, 168),
        ice: Color::Rgb(26, 104, 186),
        ice_deep: Color::Rgb(150, 184, 222),
        tool: Color::Rgb(146, 38, 160),
        tool_dim: Color::Rgb(150, 96, 160),
        approval: Color::Rgb(160, 92, 0),
        question: Color::Rgb(138, 100, 0),
        error: Color::Rgb(192, 36, 48),
        success: Color::Rgb(22, 122, 64),
        composer_rail: Color::Rgb(104, 72, 196),
        code: Color::Rgb(140, 72, 16),
        link: Color::Rgb(28, 94, 168),
        logo_hi: Color::Rgb(30, 29, 36),
        logo_lo: Color::Rgb(40, 110, 190),
        glint: Color::Rgb(20, 60, 120),
    }
}

/// `navy`, "Navy": deep navy panels with cool blue and ice accents.
pub fn navy() -> theme_t {
    theme_t {
        bg: Color::Rgb(8, 14, 32),
        side: Color::Rgb(12, 20, 42),
        panel: Color::Rgb(20, 30, 58),
        dock: Color::Rgb(16, 25, 50),
        inset: Color::Rgb(27, 39, 72),
        hover: Color::Rgb(32, 46, 82),
        select: Color::Rgb(40, 56, 98),
        marked: Color::Rgb(22, 52, 96),
        fg: Color::Rgb(224, 234, 248),
        fg_dim: Color::Rgb(142, 160, 196),
        fg_faint: Color::Rgb(84, 102, 140),
        user: Color::Rgb(242, 247, 255),
        agent: Color::Rgb(110, 180, 255),
        ice: Color::Rgb(184, 224, 255),
        ice_deep: Color::Rgb(44, 86, 150),
        tool: Color::Rgb(206, 140, 250),
        tool_dim: Color::Rgb(108, 76, 150),
        approval: Color::Rgb(255, 196, 100),
        question: Color::Rgb(246, 216, 126),
        error: Color::Rgb(255, 110, 126),
        success: Color::Rgb(96, 226, 180),
        composer_rail: Color::Rgb(130, 164, 255),
        code: Color::Rgb(226, 206, 178),
        link: Color::Rgb(120, 186, 255),
        logo_hi: Color::Rgb(240, 246, 255),
        logo_lo: Color::Rgb(90, 160, 245),
        glint: Color::Rgb(226, 240, 255),
    }
}

/// `astral`, "Astral": a violet-indigo void with pale starlight text, a violet agent, nebula
/// magenta tools, solar gold approvals, aurora teal for finished work and a cold comet blue for
/// the generation effect.
pub fn astral() -> theme_t {
    theme_t {
        bg: Color::Rgb(9, 6, 20),
        side: Color::Rgb(16, 11, 32),
        panel: Color::Rgb(27, 19, 50),
        dock: Color::Rgb(22, 15, 42),
        inset: Color::Rgb(36, 26, 64),
        hover: Color::Rgb(43, 31, 75),
        select: Color::Rgb(56, 40, 96),
        marked: Color::Rgb(40, 30, 88),
        fg: Color::Rgb(236, 230, 255),
        fg_dim: Color::Rgb(160, 148, 196),
        fg_faint: Color::Rgb(100, 90, 136),
        user: Color::Rgb(250, 246, 255),
        agent: Color::Rgb(196, 126, 255),
        ice: Color::Rgb(176, 214, 255),
        ice_deep: Color::Rgb(52, 74, 150),
        tool: Color::Rgb(255, 110, 200),
        tool_dim: Color::Rgb(132, 58, 112),
        approval: Color::Rgb(255, 200, 90),
        question: Color::Rgb(255, 222, 140),
        error: Color::Rgb(255, 96, 120),
        success: Color::Rgb(80, 236, 190),
        composer_rail: Color::Rgb(130, 150, 255),
        code: Color::Rgb(255, 206, 170),
        link: Color::Rgb(150, 190, 255),
        logo_hi: Color::Rgb(255, 240, 255),
        logo_lo: Color::Rgb(170, 120, 255),
        glint: Color::Rgb(230, 240, 255),
    }
}

/// `ember`, "Ember": charcoal and brown-black panels with fire accents (an orange agent, a
/// red-orange tool colour, amber approvals) and warm off-white text.
pub fn ember() -> theme_t {
    theme_t {
        bg: Color::Rgb(14, 10, 8),
        side: Color::Rgb(23, 17, 13),
        panel: Color::Rgb(36, 26, 20),
        dock: Color::Rgb(30, 22, 17),
        inset: Color::Rgb(45, 33, 26),
        hover: Color::Rgb(52, 38, 30),
        select: Color::Rgb(66, 47, 36),
        marked: Color::Rgb(60, 36, 28),
        fg: Color::Rgb(246, 236, 222),
        fg_dim: Color::Rgb(174, 154, 136),
        fg_faint: Color::Rgb(108, 92, 80),
        user: Color::Rgb(255, 248, 238),
        agent: Color::Rgb(255, 160, 90),
        ice: Color::Rgb(255, 214, 160),
        ice_deep: Color::Rgb(126, 70, 36),
        tool: Color::Rgb(244, 118, 64),
        tool_dim: Color::Rgb(130, 64, 36),
        approval: Color::Rgb(255, 200, 70),
        question: Color::Rgb(250, 222, 124),
        error: Color::Rgb(255, 84, 84),
        success: Color::Rgb(176, 214, 110),
        composer_rail: Color::Rgb(255, 128, 72),
        code: Color::Rgb(242, 202, 144),
        link: Color::Rgb(255, 180, 110),
        logo_hi: Color::Rgb(255, 244, 228),
        logo_lo: Color::Rgb(240, 120, 56),
        glint: Color::Rgb(255, 240, 220),
    }
}

/// `mono`, "Mono": greyscale tone steps only. The colours that carry a meaning keep a muted
/// hue so they stay apart: a dusty red for errors, a sand amber for approvals and questions, a
/// sage green for finished work. Everything else is grey.
pub fn mono() -> theme_t {
    theme_t {
        bg: Color::Rgb(0, 0, 0),
        side: Color::Rgb(12, 12, 12),
        panel: Color::Rgb(24, 24, 24),
        dock: Color::Rgb(19, 19, 19),
        inset: Color::Rgb(32, 32, 32),
        hover: Color::Rgb(38, 38, 38),
        select: Color::Rgb(50, 50, 50),
        marked: Color::Rgb(42, 42, 42),
        fg: Color::Rgb(236, 236, 236),
        fg_dim: Color::Rgb(150, 150, 150),
        fg_faint: Color::Rgb(92, 92, 92),
        user: Color::Rgb(255, 255, 255),
        agent: Color::Rgb(206, 206, 206),
        ice: Color::Rgb(236, 236, 236),
        ice_deep: Color::Rgb(88, 88, 88),
        tool: Color::Rgb(182, 182, 182),
        tool_dim: Color::Rgb(108, 108, 108),
        approval: Color::Rgb(212, 186, 132),
        question: Color::Rgb(204, 194, 156),
        error: Color::Rgb(220, 130, 130),
        success: Color::Rgb(142, 198, 152),
        composer_rail: Color::Rgb(164, 164, 164),
        code: Color::Rgb(214, 214, 214),
        link: Color::Rgb(224, 224, 224),
        logo_hi: Color::Rgb(255, 255, 255),
        logo_lo: Color::Rgb(150, 150, 150),
        glint: Color::Rgb(250, 250, 250),
    }
}

/// `high_contrast`, "High contrast": pure black and pure white, saturated accents, and dim
/// text that still reads at 7:1.
pub fn high_contrast() -> theme_t {
    theme_t {
        bg: Color::Rgb(0, 0, 0),
        side: Color::Rgb(10, 10, 10),
        panel: Color::Rgb(28, 28, 28),
        dock: Color::Rgb(20, 20, 20),
        inset: Color::Rgb(38, 38, 38),
        hover: Color::Rgb(52, 52, 52),
        select: Color::Rgb(66, 66, 92),
        marked: Color::Rgb(18, 48, 100),
        fg: Color::Rgb(255, 255, 255),
        fg_dim: Color::Rgb(214, 214, 214),
        fg_faint: Color::Rgb(170, 170, 170),
        user: Color::Rgb(255, 255, 255),
        agent: Color::Rgb(80, 200, 255),
        ice: Color::Rgb(176, 236, 255),
        ice_deep: Color::Rgb(40, 120, 190),
        tool: Color::Rgb(255, 100, 255),
        tool_dim: Color::Rgb(214, 140, 224),
        approval: Color::Rgb(255, 196, 0),
        question: Color::Rgb(255, 236, 60),
        error: Color::Rgb(255, 80, 80),
        success: Color::Rgb(40, 255, 120),
        composer_rail: Color::Rgb(200, 170, 255),
        code: Color::Rgb(255, 224, 150),
        link: Color::Rgb(110, 212, 255),
        logo_hi: Color::Rgb(255, 255, 255),
        logo_lo: Color::Rgb(80, 200, 255),
        glint: Color::Rgb(255, 255, 255),
    }
}
