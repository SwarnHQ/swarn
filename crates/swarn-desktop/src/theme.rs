//! Plexo's palette and type, ported from its `main.css`. Hex values are copied as-is so the two
//! apps match; change them here, not at call sites.

// Most tokens have no view yet; they land as screens are built.
#![allow(dead_code)]

use gpui::{App, Global, Rgba, Window, WindowAppearance, rgb};

pub const SANS: &str = "Manrope";
pub const MONO: &str = "Roboto Mono";

/// macOS title bar height. The traffic lights sit at (16, 9) so their 14px buttons centre in it.
pub const TITLE_BAR_HEIGHT: f32 = 32.;

/// Hairline used for every divider and control border (`border-[0.5px]` in Plexo).
pub const HAIRLINE: f32 = 0.5;

pub const RADIUS: f32 = 8.;

/// One accent hue in all the shades Plexo derives from it.
#[derive(Clone, Copy)]
pub struct Hue {
    pub solid: Rgba,
    pub bg: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    /// Glyph colour for content drawn on top of `solid`.
    pub on_solid: Rgba,
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Rgba,
    pub bg_secondary: Rgba,
    pub bg_tertiary: Rgba,
    pub input_bg: Rgba,
    pub track_bg: Rgba,

    pub text: Rgba,
    pub text_secondary: Rgba,
    pub text_tertiary: Rgba,

    pub border: Rgba,
    pub border_subtle: Rgba,
    pub border_strong: Rgba,
    pub footer_border: Rgba,

    pub icon_muted: Rgba,
    pub scrollbar_thumb: Rgba,

    pub accent: Rgba,
    pub accent_on_solid: Rgba,
    pub success: Rgba,
    pub danger: Rgba,
    pub danger_bg: Rgba,
    pub danger_border: Rgba,

    /// Plexo's per-network hues (Wi-Fi, USB, Ethernet). Swarn uses them for per-entity colour.
    pub teal: Hue,
    pub amber: Hue,
    pub steel: Hue,
}

impl Global for Theme {}

impl Theme {
    pub fn dark() -> Self {
        Self {
            bg: rgb(0x1b1e20),
            bg_secondary: rgb(0x202325),
            bg_tertiary: rgb(0x202325),
            input_bg: rgb(0x242729),
            track_bg: rgb(0x262a2d),
            text: rgb(0xeae7e2),
            text_secondary: rgb(0xa9adb2),
            text_tertiary: rgb(0x8d9196),
            border: rgb(0x2b2f33),
            border_subtle: rgb(0x23262a),
            border_strong: rgb(0x33383c),
            footer_border: rgb(0x2b2f33),
            icon_muted: rgb(0x33383c),
            scrollbar_thumb: rgb(0x3a3f44),
            accent: rgb(0xd8a44c),
            accent_on_solid: rgb(0x1b1408),
            success: rgb(0x4ea89a),
            danger: rgb(0xeda3a3),
            danger_bg: rgb(0x2c1c1c),
            danger_border: rgb(0x4a2a2a),
            teal: hue(0x4ea89a, 0x22312e, 0x3a5450, 0x7fc9bb, 0x10201d),
            amber: hue(0xd8a44c, 0x33291a, 0x54421f, 0xe8bb6d, 0x221806),
            steel: hue(0x7e93bd, 0x232a38, 0x3a4660, 0xa7b8db, 0x141a26),
        }
    }

    pub fn light() -> Self {
        Self {
            bg: rgb(0xffffff),
            bg_secondary: rgb(0xfafafa),
            bg_tertiary: rgb(0xf0f0f0),
            input_bg: rgb(0xffffff),
            track_bg: rgb(0xe4e4e6),
            text: rgb(0x1d1d1f),
            text_secondary: rgb(0x6e6e73),
            text_tertiary: rgb(0x737377),
            border: rgb(0xe0e0e2),
            border_subtle: rgb(0xececee),
            border_strong: rgb(0xb9b9bb),
            footer_border: rgb(0xd8d8da),
            icon_muted: rgb(0xc2c2c6),
            scrollbar_thumb: rgb(0xd0d0d2),
            accent: rgb(0xd97706),
            accent_on_solid: rgb(0xffffff),
            success: rgb(0x1f8a70),
            danger: rgb(0xdc2626),
            danger_bg: rgb(0xfef2f2),
            danger_border: rgb(0xfecaca),
            teal: hue(0x1f8a70, 0xe6f6f2, 0xb8e6dc, 0x0e6b54, 0xffffff),
            amber: hue(0xd97706, 0xfef3c7, 0xfde68a, 0x92400e, 0xffffff),
            steel: hue(0x4f64a5, 0xedf0f8, 0xcbd3ed, 0x374984, 0xffffff),
        }
    }

    /// Follows the system appearance, as Plexo does by default.
    pub fn sync(window: &Window, cx: &mut App) {
        cx.set_global(match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        });
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }
}

fn hue(solid: u32, bg: u32, border: u32, text: u32, on_solid: u32) -> Hue {
    Hue {
        solid: rgb(solid),
        bg: rgb(bg),
        border: rgb(border),
        text: rgb(text),
        on_solid: rgb(on_solid),
    }
}
