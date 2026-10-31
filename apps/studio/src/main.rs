//! Studio desktop app: window setup and startup. The editor itself lives in
//! `workspace`, its state in `state`, and each panel in `panels`.

mod assets;
mod panels;
mod project_files;
mod sound;
mod state;
mod theme;
mod timeline_rows;
#[cfg(test)]
mod ui_tests;
mod workspace;

use gpui_kit::{App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

fn main() {
    gpui_kit::application().with_assets(assets::StudioAssets).run(|cx: &mut App| {
        // Must run before any component is used.
        gpui_kit::init(cx);
        theme::apply(cx);
        workspace::bind_keys(cx);
        cx.set_menus(workspace::menus());

        let bounds = Bounds::centered(None, size(px(1600.0), px(960.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(1100.0), px(680.0))),
            titlebar: Some(TitlebarOptions { title: Some("Untitled — Studio".into()), ..Default::default() }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| workspace::Workspace::new(window, cx)))
            .expect("failed to open the Studio window");
        cx.activate(true);
    });
}
