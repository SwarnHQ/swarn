mod compose;
mod diff;
mod markdown;
mod model;
mod review;
mod service;
mod setup;
mod shell;
mod theme;
mod transcript;

use std::borrow::Cow;

use gpui::{
    App, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions, actions,
    point, prelude::*, px, size,
};
use shell::Shell;

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
        gpui_component::init(cx);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx.active_window() {
                window
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
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
        |window, cx| {
            let shell = cx.new(|cx| Shell::new(window, cx));
            // gpui-component's Root hosts the overlays its widgets need (menus, popovers).
            cx.new(|cx| gpui_component::Root::new(shell, window, cx))
        },
    )
    .expect("main window opens");
}
