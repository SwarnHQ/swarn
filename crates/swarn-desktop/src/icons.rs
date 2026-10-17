//! The few Lucide icons the window uses (ISC, see assets/icons/LICENSE), embedded so the app needs
//! no asset source and doesn't carry the 7 MB catalog.

use gpui::{Hsla, prelude::*, px};
use gpui_component::Icon;

pub const FOLDER: &[u8] = include_bytes!("../assets/icons/folder.svg");
pub const LAPTOP: &[u8] = include_bytes!("../assets/icons/laptop.svg");
pub const SETTINGS: &[u8] = include_bytes!("../assets/icons/settings.svg");
pub const SHIELD: &[u8] = include_bytes!("../assets/icons/shield-check.svg");
pub const CHEVRON_DOWN: &[u8] = include_bytes!("../assets/icons/chevron-down.svg");
pub const CHEVRON_RIGHT: &[u8] = include_bytes!("../assets/icons/chevron-right.svg");
pub const ROTATE: &[u8] = include_bytes!("../assets/icons/rotate-ccw.svg");
pub const ARROW_UP: &[u8] = include_bytes!("../assets/icons/arrow-up.svg");
pub const SQUARE: &[u8] = include_bytes!("../assets/icons/square.svg");
pub const CHECK: &[u8] = include_bytes!("../assets/icons/check.svg");
pub const SEARCH: &[u8] = include_bytes!("../assets/icons/search.svg");
pub const SQUARE_PEN: &[u8] = include_bytes!("../assets/icons/square-pen.svg");
pub const SMARTPHONE: &[u8] = include_bytes!("../assets/icons/smartphone.svg");
pub const PANEL_RIGHT: &[u8] = include_bytes!("../assets/icons/panel-right.svg");
pub const PLUS: &[u8] = include_bytes!("../assets/icons/plus.svg");
pub const GIT_BRANCH: &[u8] = include_bytes!("../assets/icons/git-branch.svg");

pub fn icon(svg: &[u8], size: f32, color: impl Into<Hsla>) -> Icon {
    Icon::empty().data(svg).size(px(size)).text_color(color)
}
