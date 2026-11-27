use rune::TextStyle;
use rune::color::{Color, hex};
use rune::tok::Family;
use rune_kit::{Kit, PaletteFocus, PaletteSeed};

pub fn h(code: &str) -> Color {
    hex(code).unwrap_or(Color::WHITE)
}

#[derive(Clone, Copy)]
pub struct UiTheme {
    pub bg_shell: Color,
    pub bg_canvas: Color,
    pub surface_1: Color,
    pub surface_2: Color,
    pub surface_3: Color,
    pub surface_code: Color,
    pub border_subtle: Color,
    pub border_strong: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub hover: Color,
    pub press: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub primary_bg: Color,
    pub primary_fg: Color,
    pub success: Color,
    pub success_bg: Color,
    pub warning: Color,
    pub warning_bg: Color,
    pub danger: Color,
    pub danger_bg: Color,
    pub info: Color,
    pub info_bg: Color,
    pub shadow: Color,
    pub scrim: Color,
    pub hair: f32,
}

impl UiTheme {
    pub fn new(dark: bool, scale: f32) -> Self {
        let hair = 1.0 / scale.max(1.0);
        if dark {
            Self {
                bg_shell: h("#121212"),
                bg_canvas: h("#161616"),
                surface_1: h("#1e1e1e"),
                surface_2: h("#262626"),
                surface_3: h("#303030"),
                surface_code: h("#0e0e0e"),
                border_subtle: Color::WHITE.alpha(0.08),
                border_strong: Color::WHITE.alpha(0.16),
                text_primary: h("#fafafa"),
                text_secondary: h("#d4d4d4"),
                text_muted: h("#9e9e9e"),
                hover: Color::WHITE.alpha(0.055),
                press: Color::WHITE.alpha(0.10),
                accent: h("#7698fd"),
                accent_soft: h("#7698fd").alpha(0.14),
                primary_bg: h("#fafafa"),
                primary_fg: h("#161616"),
                success: h("#6bd586"),
                success_bg: h("#6bd586").alpha(0.14),
                warning: h("#f2cf76"),
                warning_bg: h("#f2cf76").alpha(0.14),
                danger: h("#f17471"),
                danger_bg: h("#f17471").alpha(0.14),
                info: h("#7698fd"),
                info_bg: h("#7698fd").alpha(0.14),
                shadow: Color::BLACK.alpha(0.42),
                scrim: Color::BLACK.alpha(0.56),
                hair,
            }
        } else {
            Self {
                bg_shell: h("#f3f3f3"),
                bg_canvas: h("#fafafa"),
                surface_1: h("#ffffff"),
                surface_2: h("#f2f2f2"),
                surface_3: h("#e6e6e6"),
                surface_code: h("#18181b"),
                border_subtle: Color::BLACK.alpha(0.08),
                border_strong: Color::BLACK.alpha(0.18),
                text_primary: h("#161616"),
                text_secondary: h("#404040"),
                text_muted: h("#5c5c5c"),
                hover: Color::BLACK.alpha(0.045),
                press: Color::BLACK.alpha(0.08),
                accent: h("#3b5cf6"),
                accent_soft: h("#3b5cf6").alpha(0.12),
                primary_bg: h("#161616"),
                primary_fg: h("#fafafa"),
                success: h("#198b43"),
                success_bg: h("#198b43").alpha(0.12),
                warning: h("#ad5700"),
                warning_bg: h("#cb9f34").alpha(0.15),
                danger: h("#b82d35"),
                danger_bg: h("#b82d35").alpha(0.12),
                info: h("#3250df"),
                info_bg: h("#3b5cf6").alpha(0.12),
                shadow: Color::BLACK.alpha(0.14),
                scrim: Color::BLACK.alpha(0.42),
                hair,
            }
        }
    }

    pub fn build_kit(&self) -> Kit {
        let mut kit = Kit::from_palette_focus(
            PaletteSeed {
                bg: self.bg_canvas,
                fg: self.text_primary,
                accent: self.accent,
            },
            PaletteFocus::Blue,
        );
        kit.c.page = self.bg_shell;
        kit.c.bg_base = self.bg_canvas;
        kit.c.bg_deep = self.bg_shell;
        kit.c.layer_01 = self.surface_1;
        kit.c.layer_02 = self.surface_2;
        kit.c.layer_03 = self.surface_3;
        kit.c.border_muted = self.border_subtle;
        kit.c.border_base = self.border_subtle;
        kit.c.border_strong = self.border_strong;
        kit.c.border_focus = self.accent;
        kit.c.text_base = self.text_primary;
        kit.c.text_muted = self.text_secondary;
        kit.c.text_faint = self.text_muted;
        kit.c.scrim = self.scrim;
        kit.c.bg_contrast = self.primary_bg;
        kit.c.text_contrast = self.primary_fg;
        kit.c.bg_icon_button_contrast = self.primary_bg;
        kit.c.submit_fg = self.primary_fg;
        kit.c.submit_icon = self.primary_fg;
        kit.c.success.fg = self.success;
        kit.c.success.bg = self.success_bg;
        kit.c.warning.fg = self.warning;
        kit.c.warning.bg = self.warning_bg;
        kit.c.danger.fg = self.danger;
        kit.c.danger.bg = self.danger_bg;
        kit.c.info.fg = self.info;
        kit.c.info.bg = self.info_bg;
        kit.radius.xs = 2.0;
        kit.radius.sm = 4.0;
        kit.radius.md = 6.0;
        kit.radius.lg = 8.0;
        kit.radius.xl = 10.0;
        kit
    }
}

pub mod ty {
    use super::*;
    pub const H1: TextStyle = TextStyle::new(20.0, 600.0).lh(26.0);
    pub const H2: TextStyle = TextStyle::new(15.0, 600.0).lh(22.0);
    pub const H3: TextStyle = TextStyle::new(13.5, 600.0).lh(19.0);
    pub const BODY: TextStyle = TextStyle::new(13.0, 400.0).lh(19.0);
    pub const BODY_MED: TextStyle = TextStyle::new(13.0, 500.0).lh(19.0);
    pub const SMALL: TextStyle = TextStyle::new(12.0, 400.0).lh(17.0);
    pub const SMALL_MED: TextStyle = TextStyle::new(12.0, 500.0).lh(17.0);
    pub const CAPTION: TextStyle = TextStyle::new(11.0, 500.0).lh(15.0);
    pub const EYEBROW: TextStyle = TextStyle::new(10.5, 600.0).lh(14.0).tracking(0.5);
    pub const MONO: TextStyle = TextStyle::new(12.0, 400.0)
        .family(Family::Mono)
        .lh(18.0)
        .features("'tnum' 1");
    pub const MONO_SM: TextStyle = TextStyle::new(11.5, 500.0)
        .family(Family::Mono)
        .lh(16.0)
        .features("'tnum' 1");
}
