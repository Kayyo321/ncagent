/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! How a conversation's own settings look (D57): the colour of each tag and the glyphs for a
//! tag, a pin and a selection check.
//!
//! The tag colours do not follow the theme: a tag is the user's mark, and it should read the
//! same whatever theme is in use. They are soft and of similar lightness so that no tag shouts
//! over the others on the black background, and they map to the 256-colour cube at the end of
//! each frame like every other colour (D47).

use harness::organize::TagColor;
use ratatui::style::Color;

/// The dot that shows a conversation's colour tag.
pub const TagDot: &str = "●";

/// The mark before the title of a pinned conversation.
pub const PinGlyph: &str = "✦";

/// The selection chip of a row that is not selected, and of one that is.
pub const CheckOff: &str = "☐";
pub const CheckOn: &str = "☑";

/// The colour of a tag.
pub fn tag_rgb(color: TagColor) -> Color {
    match color {
        TagColor::Red => Color::Rgb(242, 112, 112),
        TagColor::Orange => Color::Rgb(244, 158, 92),
        TagColor::Yellow => Color::Rgb(238, 210, 110),
        TagColor::Green => Color::Rgb(122, 214, 148),
        TagColor::Teal => Color::Rgb(92, 206, 198),
        TagColor::Blue => Color::Rgb(118, 168, 255),
        TagColor::Purple => Color::Rgb(178, 142, 255),
        TagColor::Pink => Color::Rgb(242, 132, 202),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::str_width;

    #[test]
    fn every_tag_has_its_own_colour_and_the_glyphs_are_one_column() {
        let mut seen = Vec::new();
        for color in TagColor::all() {
            let rgb = tag_rgb(color);
            assert!(!seen.contains(&rgb), "{color:?} repeats a colour");
            seen.push(rgb);
        }
        for glyph in [TagDot, PinGlyph, CheckOff, CheckOn] {
            assert_eq!(str_width(glyph), 1, "{glyph}");
        }
    }
}
