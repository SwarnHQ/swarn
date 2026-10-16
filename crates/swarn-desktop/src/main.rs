mod theme;

use std::borrow::Cow;

use gpui::{
    App, Bounds, Context, FontWeight, KeyBinding, Menu, MenuItem, MouseButton, TitlebarOptions, Window,
    WindowBounds, WindowOptions, actions, div, point, prelude::*, px, size,
};
use theme::{HAIRLINE, SANS, TITLE_BAR_HEIGHT, Theme};

actions!(swarn, [Quit, CloseWindow]);

const FONTS: [&[u8]; 7] = [
    include_bytes!("../assets/fonts/Manrope-Regular.ttf"),
    include_bytes!("../assets/fonts/Manrope-Medium.ttf"),
    include_bytes!("../assets/fonts/Manrope-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Manrope-Bold.ttf"),
    include_bytes!("../assets/fonts/RobotoMono-Regular.ttf"),
    include_bytes!("../assets/fonts/RobotoMono-Medium.ttf"),
    include_bytes!("../assets/fonts/RobotoMono-SemiBold.ttf"),
];

fn main() {
    let app = gpui_platform::application();
    // macOS keeps the app alive with no windows; a Dock click brings the window back.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(cx);
        }
    });
    app.run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
            .expect("bundled fonts load");

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx.active_window() {
                window.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Swarn".into(),
            items: vec![MenuItem::action("Quit Swarn", Quit)],
            disabled: false,
        }]);

        open_main_window(cx);
        cx.activate(true);
    });
}

fn open_main_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(760.), px(640.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(720.), px(620.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Swarn".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(9.))),
            }),
            app_id: Some("dev.swarn.desktop".into()),
            app_owns_titlebar_drag: true,
            ..Default::default()
        },
        |window, cx| cx.new(|cx| Shell::new(window, cx)),
    )
    .expect("main window opens");
}

struct Shell;

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Theme::sync(window, cx);
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync(window, cx);
            cx.notify();
        })
        .detach();
        Self
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .font_family(SANS)
            .child(title_bar(theme))
            .child(div().flex_1().min_h_0())
    }
}

/// Plexo's title bar: the app name centred on a strip drawn under the native traffic lights.
fn title_bar(theme: &Theme) -> impl IntoElement {
    div()
        .id("title-bar")
        .h(px(TITLE_BAR_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .px(px(94.))
        .bg(theme.bg_secondary)
        .border_b(px(HAIRLINE))
        .border_color(theme.border)
        // Plexo drags from the whole strip (`-webkit-app-region: drag`), and double-click zooms.
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_secondary)
                .child("Swarn"),
        )
}
