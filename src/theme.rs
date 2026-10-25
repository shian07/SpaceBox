use crate::scan::Category;
use fltk::{app, enums::Color};

pub const BG: Color = Color::from_rgb(19, 21, 25);
pub const PANEL: Color = Color::from_rgb(27, 30, 36);
pub const PANEL_ALT: Color = Color::from_rgb(35, 39, 47);
/// Hairlines between panels. Decorative only: at 1.46:1 against `PANEL` these
/// separate regions by texture, not luminance, so nothing relies on them to
/// carry meaning. Elements that must be perceivable use `LINE` (>= 3:1, WCAG 1.4.11).
pub const BORDER: Color = Color::from_rgb(52, 58, 69);
/// Perceivable boundary for elements that carry meaning: progress tracks,
/// drop targets and focus outlines.
pub const LINE: Color = Color::from_rgb(98, 108, 126);
pub const TEXT: Color = Color::from_rgb(230, 234, 240);
pub const MUTED: Color = Color::from_rgb(143, 151, 164);
pub const ACCENT: Color = Color::from_rgb(72, 160, 255);
pub const ACCENT_DIM: Color = Color::from_rgb(38, 92, 148);
pub const DANGER: Color = Color::from_rgb(235, 94, 94);
pub const WARNING: Color = Color::from_rgb(242, 183, 82);
pub const SUCCESS: Color = Color::from_rgb(92, 202, 143);

pub const OTHER_RGB: (u8, u8, u8) = (72, 77, 88);
pub const SMALL_FILES_RGB: (u8, u8, u8) = (91, 96, 107);
pub const ACCENT_RGB: (u8, u8, u8) = (72, 160, 255);

pub const BG_RGB: (u8, u8, u8) = (19, 21, 25);
pub const PANEL_RGB: (u8, u8, u8) = (27, 30, 36);
pub const TEXT_RGB: (u8, u8, u8) = (230, 234, 240);
pub const MUTED_RGB: (u8, u8, u8) = (143, 151, 164);

pub fn apply() {
    app::background(BG_RGB.0, BG_RGB.1, BG_RGB.2);
    app::background2(PANEL_RGB.0, PANEL_RGB.1, PANEL_RGB.2);
    app::foreground(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2);
    app::set_color(Color::BackGround, BG_RGB.0, BG_RGB.1, BG_RGB.2);
    app::set_color(Color::BackGround2, PANEL_RGB.0, PANEL_RGB.1, PANEL_RGB.2);
    app::set_color(Color::ForeGround, TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2);
    app::set_color(Color::Selection, 44, 88, 142);
    app::set_color(Color::Inactive, MUTED_RGB.0, MUTED_RGB.1, MUTED_RGB.2);
    app::set_font_size(12);
}

pub fn dir_rgb(depth: u8) -> (u8, u8, u8) {
    match depth {
        0 => (57, 66, 81),
        1 => (67, 78, 95),
        2 => (78, 91, 110),
        _ => (90, 105, 126),
    }
}

pub fn category_rgb(category: Category) -> (u8, u8, u8) {
    match category {
        Category::Video => (224, 96, 112),
        Category::Image => (240, 160, 72),
        Category::Audio => (196, 128, 240),
        Category::Archive => (214, 188, 76),
        Category::Code => (84, 190, 236),
        Category::Document => (108, 208, 156),
        Category::Data => (120, 160, 246),
        Category::System => (160, 172, 190),
        Category::Other => (150, 156, 168),
    }
}

pub fn rgb_color(rgb: (u8, u8, u8)) -> Color {
    Color::from_rgb(rgb.0, rgb.1, rgb.2)
}

pub fn blend(left: (u8, u8, u8), right: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let amount = amount.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount) as u8;
    (
        mix(left.0, right.0),
        mix(left.1, right.1),
        mix(left.2, right.2),
    )
}

pub fn text_on(rgb: (u8, u8, u8)) -> Color {
    let luminance = 0.2126 * rgb.0 as f32 + 0.7152 * rgb.1 as f32 + 0.0722 * rgb.2 as f32;
    if luminance > 145.0 {
        Color::from_rgb(18, 20, 24)
    } else {
        Color::from_rgb(244, 246, 250)
    }
}
