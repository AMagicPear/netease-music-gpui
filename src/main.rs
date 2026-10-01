mod assets;
mod components;
mod pages;
mod state;
mod theme;

use std::path::PathBuf;

use assets::Assets;
use components::PlayerBar;
use gpui::*;
use pages::MainContent;
use state::{
    playback::{PlaybackState, Song},
    user::UserProfile,
};
use std::time::Duration;

struct MainWindow {
    main_content: Entity<MainContent>,
    player_bar: Entity<PlayerBar>,
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = gpui_kit::base::Theme::global(cx).tokens.colors;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .child(self.main_content.clone())
            .child(self.player_bar.clone())
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(Assets {
            base: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
        })
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            theme::init(cx);
            let bounds = Bounds::centered(None, size(px(1060.), px(720.)), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    app_owns_titlebar_drag: true,
                    titlebar: Some(TitlebarOptions {
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let playback = cx.new(|_| PlaybackState {
                        current_song: Some(Song {
                            title: "Run Away With Me".into(),
                            artist: "Carly Rae Jepsen".into(),
                            duration: Duration::from_secs(210),
                        }),
                        position: Duration::from_secs(74),
                        is_playing: false,
                    });
                    let user_profile = cx.new(|_| UserProfile {
                        name: "一只会魔法的梨".into(),
                        avatar_path: "/Users/amagicpear/Pictures/Perry Origin Character/IMG_20240601_133150.jpeg".into(),
                    });
                    let main_content = cx.new(|cx| MainContent::new(window, user_profile, cx));
                    let player_bar = cx.new(|cx| PlayerBar::new(playback, cx));
                    cx.new(|_| MainWindow {
                        main_content,
                        player_bar,
                    })
                },
            )
            .unwrap();
        });
}
