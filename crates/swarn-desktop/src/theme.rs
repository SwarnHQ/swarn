//! Plexo's palette and type, ported from its `main.css`. Hex values are copied as-is so the two
//! apps match; change them here, not at call sites.

// Most tokens have no view yet; they land as screens are built.
#![allow(dead_code)]

use gpui::{
    App, Div, ElementId, FontWeight, Global, Rgba, SharedString, Stateful, Window,
    WindowAppearance, div, prelude::*, px, rgb,
};
use swarn_core::schema::Status;

/// SF Pro, through the name macOS gives its system UI font. Manrope stays bundled while we compare.
pub const SANS: &str = ".SystemUIFont";
pub const MONO: &str = "Roboto Mono";

/// Codex's title bar height. The traffic lights sit at (18, 12) so their 14px buttons centre in it.
pub const TITLE_BAR_HEIGHT: f32 = 38.;

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
    /// The window frame: title bar and the margin around the content panel.
    pub chrome: Rgba,
    /// The selected row in a list.
    pub selected: Rgba,
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
    /// Sampled from the Codex desktop app (PRD §5.6): neutral greys, the window frame lighter than
    /// the sidebar, the sidebar lighter than the canvas.
    pub fn dark() -> Self {
        Self {
            chrome: rgb(0x303030),
            selected: rgb(0x313131),
            bg: rgb(0x171717),
            bg_secondary: rgb(0x202020),
            bg_tertiary: rgb(0x262626),
            input_bg: rgb(0x242424),
            track_bg: rgb(0x2f2f2f),
            text: rgb(0xececec),
            text_secondary: rgb(0xc4c4c4),
            text_tertiary: rgb(0x7c7c7c),
            border: rgb(0x2e2e2e),
            border_subtle: rgb(0x2a2a2a),
            border_strong: rgb(0x3a3a3a),
            footer_border: rgb(0x2e2e2e),
            icon_muted: rgb(0x5a5a5a),
            scrollbar_thumb: rgb(0x3a3a3a),
            accent: rgb(0x3b82f6),
            accent_on_solid: rgb(0xffffff),
            success: rgb(0x3fb950),
            danger: rgb(0xf47174),
            danger_bg: rgb(0x2a1818),
            danger_border: rgb(0x4a2424),
            teal: hue(0x3fb950, 0x1c2a1f, 0x2d4733, 0x7ad48c, 0x0f1a12),
            amber: hue(0xf5b833, 0x2f2814, 0x4d3f1a, 0xf7c95c, 0x221806),
            steel: hue(0x9a9a9a, 0x262626, 0x3a3a3a, 0xc4c4c4, 0x171717),
        }
    }

    pub fn light() -> Self {
        Self {
            chrome: rgb(0xe9e9eb),
            selected: rgb(0xe6e6e8),
            bg: rgb(0xffffff),
            bg_secondary: rgb(0xf7f7f8),
            bg_tertiary: rgb(0xf0f0f1),
            input_bg: rgb(0xf4f4f5),
            track_bg: rgb(0xededef),
            text: rgb(0x1d1d1f),
            text_secondary: rgb(0x3f3f46),
            text_tertiary: rgb(0x8a8a90),
            border: rgb(0xe4e4e7),
            border_subtle: rgb(0xeeeef0),
            border_strong: rgb(0xd4d4d8),
            footer_border: rgb(0xe4e4e7),
            icon_muted: rgb(0xb4b4ba),
            scrollbar_thumb: rgb(0xd0d0d2),
            accent: rgb(0x2563eb),
            accent_on_solid: rgb(0xffffff),
            success: rgb(0x1a7f37),
            danger: rgb(0xdc2626),
            danger_bg: rgb(0xfef2f2),
            danger_border: rgb(0xfecaca),
            teal: hue(0x1a7f37, 0xe8f5ec, 0xbfe3c9, 0x116329, 0xffffff),
            amber: hue(0xd99a06, 0xfef6e0, 0xf6df9e, 0x8a5a00, 0xffffff),
            steel: hue(0x71717a, 0xf4f4f5, 0xe4e4e7, 0x3f3f46, 0xffffff),
        }
    }

    /// Follows the system appearance, as Plexo does by default, and points gpui-component's
    /// widgets (text inputs so far) at the same palette and fonts.
    pub fn sync(window: &mut Window, cx: &mut App) {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let theme = if dark { Self::dark() } else { Self::light() };
        cx.set_global(theme);

        use gpui_component::{Theme as Ui, ThemeMode};
        Ui::change(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            Some(window),
            cx,
        );
        let ui = Ui::global_mut(cx);
        ui.font_family = SANS.into();
        ui.mono_font_family = MONO.into();
        ui.radius = px(RADIUS);
        let hsla = |c: Rgba| -> gpui::Hsla { c.into() };
        let c = &mut ui.colors;
        c.background = hsla(theme.input_bg);
        c.foreground = hsla(theme.text);
        c.muted_foreground = hsla(theme.text_tertiary);
        c.border = hsla(theme.border);
        c.input = hsla(theme.border_strong);
        c.ring = hsla(theme.accent);
        c.caret = hsla(theme.accent);
        c.selection = hsla(theme.accent).opacity(0.3);
        c.popover = hsla(theme.bg_secondary);
        c.popover_foreground = hsla(theme.text);
        c.accent = hsla(theme.track_bg);
        c.accent_foreground = hsla(theme.text);
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Status colours, borrowed from Plexo's network hues: teal while working, amber when the
    /// user is needed (Plexo's accent), steel when done, danger red on failure, grey otherwise.
    pub fn status_hue(&self, status: Option<Status>) -> Hue {
        match status {
            Some(Status::Working) => self.teal,
            Some(Status::NeedsInput) => self.amber,
            Some(Status::Done) => self.steel,
            Some(Status::Failed) => Hue {
                solid: self.danger,
                bg: self.danger_bg,
                border: self.danger_border,
                text: self.danger,
                on_solid: self.bg,
            },
            None | Some(Status::Starting) | Some(Status::Exited) => Hue {
                solid: self.text_tertiary,
                bg: self.bg_secondary,
                border: self.border,
                text: self.text_secondary,
                on_solid: self.bg,
            },
        }
    }
}

/// A section label as Codex draws them ("Projects"): sentence case, quiet, the interface font.
pub fn label(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.5))
        .line_height(px(14.))
        .text_color(theme.text_tertiary)
        .child(text.into())
}

#[derive(Clone, Copy)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Destructive,
}

/// A button at Codex's size: 28px tall, 8px radius, 13px medium text.
pub fn button(
    theme: &Theme,
    id: impl Into<ElementId>,
    text: &'static str,
    kind: ButtonKind,
) -> Stateful<Div> {
    let (bg, fg, border) = match kind {
        // Codex's primary action is the foreground colour as a fill: white in dark, black in light.
        ButtonKind::Primary => (theme.text, theme.bg, theme.text),
        ButtonKind::Secondary => (theme.track_bg, theme.text, theme.track_bg),
        ButtonKind::Destructive => (theme.danger_bg, theme.danger, theme.danger_border),
    };
    div()
        .id(id)
        .h(px(28.))
        .px(px(11.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS))
        .border(px(1.))
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .hover(|s| s.opacity(0.85))
        .active(|s| s.opacity(0.7))
        .child(text)
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
