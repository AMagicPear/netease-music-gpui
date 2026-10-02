mod api;
mod models;
mod playback;
mod state;
mod ui;

use std::path::PathBuf;

use gpui::*;
use playback::PlaybackController;
use state::{account::AccountState, library::MusicLibrary};
use ui::{MainContent, MainWindow, PlayerBar, assets::Assets, theme};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api = api::MusicApi::from_env()?;
    gpui_kit::application()
        .with_http_client(api.http_client())
        .with_assets(Assets {
            base: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
        })
        .run(move |cx: &mut App| {
            cx.set_global(api);
            theme::load_fonts(cx);
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
                    let playback = cx.new(PlaybackController::new);
                    let user_profile = cx.new(AccountState::new);
                    let library = cx.new(|cx| MusicLibrary::new(user_profile.clone(), cx));
                    let main_content = cx.new(|cx| {
                        MainContent::new(window, user_profile, library, playback.clone(), cx)
                    });
                    let player_bar = cx.new(|cx| PlayerBar::new(playback, window, cx));
                    cx.new(|_| MainWindow {
                        main_content,
                        player_bar,
                    })
                },
            )
            .unwrap();
        });
    Ok(())
}
