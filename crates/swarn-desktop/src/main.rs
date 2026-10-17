mod bench;
mod compose;
mod diff;
mod icons;
mod markdown;
mod model;
mod notify;
mod pairing;
mod palette;
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
    bench::launched();
    shell::prefetch();
    let app = gpui_platform::application();
    // macOS keeps the app alive with no windows; a Dock click brings the window back.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(cx);
        }
    });
    bench::mark("application created");
    app.run(|cx: &mut App| {
        bench::mark("run entered");
        cx.text_system()
            .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
            .expect("bundled fonts load");
        bench::mark("fonts added");
        gpui_component::init(cx);
        bench::mark("gpui_component::init");
        palette::bind_keys(cx);

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
        bench::mark("window opened");
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
                traffic_light_position: Some(point(px(18.), px(12.))),
            }),
            app_id: Some("dev.swarn.desktop".into()),
            app_owns_titlebar_drag: true,
            ..Default::default()
        },
        |window, cx| {
            let shell = cx.new(|cx| Shell::new(window, cx));
            // Keyboard actions reach the shell from the start, before anything is clicked.
            palette::register_handlers(cx, shell.downgrade());
            let focus = shell.read(cx).focus.clone();
            focus.focus(window, cx);
            // gpui-component's Root hosts the overlays its widgets need (menus, popovers).
            cx.new(|cx| gpui_component::Root::new(shell, window, cx))
        },
    )
    .expect("main window opens");
}
