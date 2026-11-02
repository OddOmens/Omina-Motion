//! Studio desktop app: window setup and startup. The editor itself lives in
//! `workspace`, its state in `state`, and each panel in `panels`.

mod app_settings;
mod assets;
mod download;
mod library_store;
mod openconnect;
mod panels;
mod project_files;
mod sound;
mod state;
mod theme;
#[cfg(test)]
mod ui_tests;
mod workspace;

use gpui_kit::{App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};

fn main() {
    gpui_kit::application().with_assets(assets::StudioAssets).run(|cx: &mut App| {
        // Must run before any component is used.
        gpui_kit::init(cx);
        let settings = app_settings::load();
        theme::apply(settings.theme, settings.accent, cx);
        workspace::bind_keys(cx);
        cx.set_menus(workspace::menus());

        // Where the window was last time, or centered at a comfortable size.
        let window_bounds = match settings.window {
            Some(placement) => {
                let bounds = Bounds::new(point(px(placement.x), px(placement.y)), size(px(placement.width.max(1100.0)), px(placement.height.max(680.0))));
                if placement.maximized { WindowBounds::Maximized(bounds) } else { WindowBounds::Windowed(bounds) }
            }
            None => WindowBounds::Windowed(Bounds::centered(None, size(px(1600.0), px(960.0)), cx)),
        };
        let options = WindowOptions {
            window_bounds: Some(window_bounds),
            window_min_size: Some(size(px(1100.0), px(680.0))),
            titlebar: Some(TitlebarOptions { title: Some("Untitled — Studio".into()), ..Default::default() }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| workspace::Workspace::new(window, cx)))
            .expect("failed to open the Studio window");
        cx.activate(true);
    });
}
