mod api;
mod desktop;
mod models;
mod persistence;
mod playback;
mod state;
mod ui;

use gpui::*;
use persistence::Persistence;
use playback::PlaybackController;
use state::{account::AccountState, library::MusicLibrary};
use ui::{
    assets::Assets,
    components::PlayerBar,
    shell::{MainContent, MainWindow},
    theme,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assets = Assets::new()?;
    let persistence = Persistence::new()?;
    let api = api::MusicApi::from_env()?;
    let application = gpui_kit::application()
        .with_http_client(api.http_client())
        .with_assets(assets)
        .with_quit_mode(QuitMode::Explicit);
    application.on_reopen(desktop::show_window);
    application.run(move |cx: &mut App| {
        cx.set_global(api);
        theme::load_fonts(cx).expect("failed to load external dolphin fonts");
        gpui_kit::init(cx);
        theme::init(cx);
        let bounds = Bounds::centered(None, size(px(1060.), px(720.)), cx);
        // Dock 菜单在窗口构建后单独安装，需要拿到播放与音乐库 Entity。
        let mut desktop_handles = None;
        let (window, view) = gpui_kit::open_window(
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
                desktop::configure_window(window, cx);
                let playback =
                    cx.new(|cx| PlaybackController::new(window, persistence.clone(), cx));
                let user_profile = cx.new(AccountState::new);
                let library = cx.new(|cx| MusicLibrary::new(user_profile.clone(), cx));
                desktop_handles = Some((playback.clone(), library.clone()));
                let main_content = cx.new(|cx| {
                    MainContent::new(window, user_profile, library.clone(), playback.clone(), cx)
                });
                let player_bar = cx.new(|cx| PlayerBar::new(playback, library, window, cx));
                cx.new(|cx| MainWindow::new(main_content, player_bar, cx))
            },
        )
        .unwrap();
        let (playback, library) = desktop_handles.expect("open_window 会运行构建闭包");
        desktop::init(window, view, playback, library, cx);
    });
    Ok(())
}
