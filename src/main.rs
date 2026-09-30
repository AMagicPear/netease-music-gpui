mod assets;
mod components;
mod state;
mod theme;

use std::path::PathBuf;

use assets::Assets;
use components::PlayerBar;
use gpui::*;
use state::playback::{PlaybackState, Song};
use std::time::Duration;

struct MainWindow {
    theme: theme::Theme,
    player_bar: Entity<PlayerBar>,
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
            .child(self.player_bar.clone())
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
                    let playback = cx.new(|_| PlaybackState {
                        current_song: Some(Song {
                            title: "Run Away With Me".into(),
                            artist: "Carly Rae Jepsen".into(),
                            duration: Duration::from_secs(210),
                        }),
                        position: Duration::from_secs(74),
                        is_playing: false,
                    });
                    let player_bar = cx.new(|cx| PlayerBar::new(theme, playback, cx));
                    cx.new(|_| MainWindow { theme, player_bar })
                },
            )
            .unwrap();
        });
}
