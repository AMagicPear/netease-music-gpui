mod api;
mod auth;
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
    components::{PlayerBar, SidePanel},
    now_playing::NowPlaying,
    shell::{MainContent, MainWindow},
    theme,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assets = Assets::new()?;
    let persistence = Persistence::new()?;
    // 登录凭据只来自本地保存的会话；没有就进登录窗扫码。
    let cookie = persistence.load_session().unwrap_or_else(|error| {
        eprintln!("读取登录凭据失败：{error}");
        None
    });
    let has_session = cookie.is_some();
    let api = api::MusicApi::new(cookie)?;
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
                let user_profile =
                    cx.new(|cx| AccountState::new(persistence.clone(), has_session, cx));
                let library = cx.new(|cx| MusicLibrary::new(user_profile.clone(), cx));
                // 全局侧面板：播放栏（播放队列页）与页头（消息页）共用同一个开合外壳。
                let side_panel = cx.new(|cx| {
                    SidePanel::new(
                        playback.clone(),
                        library.clone(),
                        user_profile.clone(),
                        window,
                        cx,
                    )
                });
                desktop_handles = Some((playback.clone(), library.clone()));
                let main_content = cx.new(|cx| {
                    MainContent::new(
                        window,
                        user_profile,
                        library.clone(),
                        playback.clone(),
                        side_panel.clone(),
                        cx,
                    )
                });
                let now_playing = cx.new(|_| NowPlaying::new());
                let player_bar = cx.new(|cx| {
                    PlayerBar::new(
                        playback.clone(),
                        library.clone(),
                        now_playing.clone(),
                        side_panel,
                        window,
                        cx,
                    )
                });
                cx.new(|cx| {
                    MainWindow::new(main_content, player_bar, now_playing, playback, library, cx)
                })
            },
        )
        .unwrap();
        let (playback, library) = desktop_handles.expect("open_window 会运行构建闭包");
        desktop::init(window, view, playback, library, cx);
    });
    Ok(())
}
