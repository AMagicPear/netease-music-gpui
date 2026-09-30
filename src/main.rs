mod assets;
mod components;
mod theme;

use std::path::PathBuf;

use assets::Assets;
use components::{PlayerBar, ProgressBar};
use gpui::*;

struct MainWindow {
    theme: theme::Theme,
    progress_bar: Entity<ProgressBar>,
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.app_background)
            .child(div().flex_1())
            .child(PlayerBar {
                theme,
                progress_bar: self.progress_bar.clone(),
            })
    }
}

fn main() {
    Application::new()
        .with_assets(Assets {
            base: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
        })
        .run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(1060.), px(720.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| {
                    let theme = theme::Theme::netease();
                    let progress_bar = cx.new(|_| ProgressBar::new(theme));
                    cx.new(|_| MainWindow {
                        theme,
                        progress_bar,
                    })
                },
            )
            .unwrap();
        });
}
