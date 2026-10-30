use anyhow::anyhow;
use gpui::*;
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

use crate::api::MusicApi;
// 只在 macOS 的 Dock 菜单动作里用到（见下面的 dock_actions），Windows 上不需要。
#[cfg(target_os = "macos")]
use crate::models::PlayMode;
use crate::playback::PlaybackController;
#[cfg(target_os = "macos")]
use crate::playback::PlaybackEvent;
use crate::state::library::MusicLibrary;
use crate::ui::shell::MainWindow;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

/// Dock 右键菜单里可触发的动作。它们只转发到播放控制器 / 音乐库。
#[cfg(target_os = "macos")]
mod dock_actions {
    use gpui::actions;

    actions!(
        netease_music,
        [
            TogglePlayback,
            NextTrack,
            PreviousTrack,
            ToggleLike,
            RepeatOff,
            RepeatOneTrack,
            RepeatAllTracks,
            ShufflePlayback,
            HeartMode,
        ]
    );
}

/// 应用持有视图和托盘；窗口不可见时，播放 Entity 和页面状态仍然存活。
struct Desktop {
    window: AnyWindowHandle,
    view: Entity<MainWindow>,
    /// 网页唤起后台取回数据后要提交播放队列，需要控制器句柄。
    playback: Entity<PlaybackController>,
    /// 音乐库；收藏弹窗要显示歌单列表，需要读它。
    library: Entity<MusicLibrary>,
    bounds: WindowBounds,
    /// 「收藏到歌单」弹窗的句柄；为空表示当前没有打开。同一时刻只允许存在一个。
    collect_window: Option<AnyWindowHandle>,
    /// 「发布评论」小窗的句柄；为空表示当前没有打开。同一时刻只允许存在一个。
    comment_window: Option<AnyWindowHandle>,
    _tray: Option<TrayIcon>,
    #[cfg(target_os = "macos")]
    _dock_subscriptions: Vec<Subscription>,
}

impl Global for Desktop {}

pub fn init(
    window: AnyWindowHandle,
    view: Entity<MainWindow>,
    playback: Entity<PlaybackController>,
    library: Entity<MusicLibrary>,
    cx: &mut App,
) {
    let bounds = window
        .update(cx, |_, window, _| window.window_bounds())
        .unwrap();
    cx.set_global(Desktop {
        window,
        view,
        playback: playback.clone(),
        library: library.clone(),
        bounds,
        collect_window: None,
        comment_window: None,
        _tray: None,
        #[cfg(target_os = "macos")]
        _dock_subscriptions: Vec::new(),
    });
    #[cfg(target_os = "macos")]
    {
        cx.global_mut::<Desktop>()._dock_subscriptions = install_dock_menu(playback, library, cx);
    }
    // 托盘需要在原生事件循环启动后、GPUI 主线程上创建。
    cx.spawn(async |cx| {
        cx.update(|cx| match create_tray(cx) {
            Ok(tray) => cx.global_mut::<Desktop>()._tray = Some(tray),
            Err(error) => log::warn!("系统托盘初始化失败：{error}"),
        });
    })
    .detach();
}

/// 音乐库 Entity；窗口创建后一直存在，收藏弹窗用它读歌单列表。
pub(crate) fn library(cx: &App) -> Option<Entity<MusicLibrary>> {
    Some(cx.try_global::<Desktop>()?.library.clone())
}

/// 播放控制器 Entity；网页唤起用它提交播放队列。
pub(crate) fn playback(cx: &App) -> Option<Entity<PlaybackController>> {
    Some(cx.try_global::<Desktop>()?.playback.clone())
}

/// 当前打开的收藏弹窗句柄；`None` 表示没有打开。
pub(crate) fn collect_window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Desktop>()?.collect_window
}

/// 记录 / 清除收藏弹窗句柄。窗口关闭后必须清掉，否则会留下一个失效句柄。
pub(crate) fn set_collect_window(window: Option<AnyWindowHandle>, cx: &mut App) {
    cx.global_mut::<Desktop>().collect_window = window;
}

/// 当前打开发布评论小窗的句柄；`None` 表示没有打开。
pub(crate) fn comment_window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Desktop>()?.comment_window
}

/// 记录 / 清除发布评论小窗句柄。窗口关闭后必须清掉，否则会留下一个失效句柄。
pub(crate) fn set_comment_window(window: Option<AnyWindowHandle>, cx: &mut App) {
    cx.global_mut::<Desktop>().comment_window = window;
}

/// macOS Dock 右键菜单：播放控制、喜欢开关、播放方式。
///
/// Dock 菜单是原生 NSMenu，内容在创建时固定，无法像 GPUI 元素那样自动重绘。
/// 所以这里在播放状态变化时重建整份菜单，让“播放/暂停”“喜欢/取消喜欢”
/// 和循环方式的勾选跟着当前状态走。
#[cfg(target_os = "macos")]
fn install_dock_menu(
    playback: Entity<PlaybackController>,
    library: Entity<MusicLibrary>,
    cx: &mut App,
) -> Vec<Subscription> {
    use dock_actions::*;

    // 所有处理器只把动作转发给 Entity；窗口 / 菜单状态由控制器自己维护。
    cx.on_action({
        let playback = playback.clone();
        move |_: &TogglePlayback, cx: &mut App| playback.update(cx, |p, cx| p.toggle(cx))
    });
    cx.on_action({
        let playback = playback.clone();
        move |_: &NextTrack, cx: &mut App| playback.update(cx, |p, cx| p.next(cx))
    });
    cx.on_action({
        let playback = playback.clone();
        move |_: &PreviousTrack, cx: &mut App| playback.update(cx, |p, cx| p.previous(cx))
    });
    cx.on_action({
        let playback = playback.clone();
        let library = library.clone();
        move |_: &ToggleLike, cx: &mut App| {
            let Some(song) = playback.read(cx).snapshot().current_song.clone() else {
                return;
            };
            library.update(cx, |library, cx| library.toggle_like(song, cx));
        }
    });
    cx.on_action({
        let playback = playback.clone();
        move |_: &RepeatOff, cx: &mut App| {
            playback.update(cx, |p, cx| p.set_mode(PlayMode::Sequential, cx))
        }
    });
    cx.on_action({
        let playback = playback.clone();
        move |_: &RepeatOneTrack, cx: &mut App| {
            playback.update(cx, |p, cx| p.set_mode(PlayMode::RepeatOne, cx))
        }
    });
    cx.on_action({
        let playback = playback.clone();
        move |_: &RepeatAllTracks, cx: &mut App| {
            playback.update(cx, |p, cx| p.set_mode(PlayMode::RepeatAll, cx))
        }
    });
    cx.on_action({
        let playback = playback.clone();
        move |_: &ShufflePlayback, cx: &mut App| {
            playback.update(cx, |p, cx| p.set_mode(PlayMode::Shuffle, cx))
        }
    });
    cx.on_action({
        let playback = playback.clone();
        move |_: &HeartMode, cx: &mut App| {
            playback.update(cx, |p, cx| {
                if p.snapshot().mode == PlayMode::Heart {
                    p.exit_heart_mode(cx);
                } else {
                    p.set_mode(PlayMode::Heart, cx);
                }
            });
        }
    });

    // 播放控制器每 100 ms 就 notify 一次，菜单内容只有在这三个字段变化时才需要重建。
    let last_state = std::rc::Rc::new(std::cell::RefCell::new(None));
    let apply = {
        let playback = playback.clone();
        let library = library.clone();
        let last_state = last_state.clone();
        move |cx: &mut App| {
            let state = dock_menu_state(&playback, &library, cx);
            if last_state.replace(Some(state)) != Some(state) {
                cx.set_dock_menu(build_dock_menu(state));
            }
        }
    };
    apply(cx);
    vec![
        // Dock 菜单跟着播放态与模式走：都是 SnapshotChanged；`apply` 内部还有
        // 一层三字段 diff，只有内容真变了才重建原生菜单。
        cx.subscribe(&playback, {
            let apply = apply.clone();
            move |_, event: &PlaybackEvent, cx| {
                if matches!(event, PlaybackEvent::SnapshotChanged) {
                    apply(cx);
                }
            }
        }),
        cx.observe(&library, move |_, cx| apply(cx)),
    ]
}

/// Dock 菜单上会随状态变化的字段；这几个字段都不变时菜单不必重建。
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq)]
struct DockMenuState {
    playing: bool,
    mode: PlayMode,
    liked: bool,
    /// 心动模式建立在「我喜欢的音乐」上；播放别的歌单时这一档不存在。
    heart_reachable: bool,
}

#[cfg(target_os = "macos")]
fn dock_menu_state(
    playback: &Entity<PlaybackController>,
    library: &Entity<MusicLibrary>,
    cx: &App,
) -> DockMenuState {
    let player = playback.read(cx);
    DockMenuState {
        playing: player.is_play_requested(),
        mode: player.snapshot().mode,
        liked: player
            .snapshot()
            .current_song
            .as_ref()
            .is_some_and(|song| library.read(cx).is_liked(song.id)),
        heart_reachable: player.heart_reachable(),
    }
}

#[cfg(target_os = "macos")]
fn build_dock_menu(state: DockMenuState) -> Vec<gpui::MenuItem> {
    let DockMenuState {
        playing,
        mode,
        liked,
        heart_reachable,
    } = state;
    let mut items = vec![
        gpui::MenuItem::action(
            if playing { "暂停" } else { "播放" },
            dock_actions::TogglePlayback,
        ),
        gpui::MenuItem::action("下一个", dock_actions::NextTrack),
        gpui::MenuItem::action("上一个", dock_actions::PreviousTrack),
        gpui::MenuItem::action(
            if liked { "取消喜欢" } else { "喜欢" },
            dock_actions::ToggleLike,
        ),
        gpui::MenuItem::submenu(
            gpui::Menu::new("循环播放").items([
                gpui::MenuItem::action("关", dock_actions::RepeatOff)
                    .checked(mode == PlayMode::Sequential),
                gpui::MenuItem::action("单曲", dock_actions::RepeatOneTrack)
                    .checked(mode == PlayMode::RepeatOne),
                gpui::MenuItem::action("全部", dock_actions::RepeatAllTracks)
                    .checked(mode == PlayMode::RepeatAll),
            ]),
        ),
        gpui::MenuItem::action("随机播放", dock_actions::ShufflePlayback)
            .checked(mode == PlayMode::Shuffle),
    ];
    // 心动模式建立在「我喜欢的音乐」上；播放别的歌单时这一档不存在。
    if heart_reachable {
        items.push(
            gpui::MenuItem::action("心动模式", dock_actions::HeartMode)
                .checked(mode == PlayMode::Heart),
        );
    }
    items
}

/// 网页「点击打开客户端」按钮的唤起目标。官方网页把参数 JSON 编成 base64
/// 放在 orpheus:// 后面，如 {"type":"song","id":"21948077","cmd":"play"}。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OrpheusTarget {
    Song(u64),
    Playlist(u64),
}

/// 解析 orpheus:// 唤起链接。官方网页用 `location.href = "orpheus://" + btoa(JSON)`
/// 触发，cmd 只区分 play/download，这里统一按播放处理；其余类型（专辑、节目等）不响应。
fn parse_orpheus_url(url: &str) -> Option<OrpheusTarget> {
    let payload = url.strip_prefix("orpheus://")?.trim();
    // URL 传递时可能丢掉 base64 尾部的 `=` 填充，补齐再解码。
    let mut payload = payload.to_string();
    while payload.len() % 4 != 0 {
        payload.push('=');
    }
    let json = BASE64.decode(payload.as_bytes()).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&json).ok()?;
    let id = match &value["id"] {
        serde_json::Value::String(id) => id.parse().ok()?,
        serde_json::Value::Number(id) => id.as_u64()?,
        _ => return None,
    };
    match value["type"].as_str()? {
        "song" => Some(OrpheusTarget::Song(id)),
        "playlist" => Some(OrpheusTarget::Playlist(id)),
        _ => None,
    }
}

/// 处理系统转来的 orpheus:// 唤起：解析出目标，后台取回数据后立即播放，并把窗口带回前台。
pub fn handle_open_urls(urls: Vec<String>, cx: &mut App) {
    let mut targets = Vec::new();
    for url in urls {
        match parse_orpheus_url(&url) {
            Some(target) => targets.push(target),
            None => log::warn!("忽略无法识别的唤起链接"),
        }
    }
    if targets.is_empty() {
        return;
    }
    show_window(cx);
    let Some(playback) = playback(cx) else {
        return;
    };
    for target in targets {
        play_orpheus_target(target, playback.clone(), cx);
    }
}

/// 先发请求再提交播放：网络在 Tokio 运行时上，结果回到 GPUI 主线程交给播放控制器。
fn play_orpheus_target(target: OrpheusTarget, playback: Entity<PlaybackController>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let fetched = match target {
            OrpheusTarget::Song(id) => {
                let request = cx.update(|cx| {
                    let api = MusicApi::global(cx);
                    api.runtime
                        .spawn(MusicApi::song_detail(api.client.clone(), id))
                });
                // 任务被取消按请求失败处理。
                request
                    .await
                    .unwrap_or_else(|_| Err("歌曲详情请求任务失败".into()))
                    .map(|song| vec![song])
            }
            OrpheusTarget::Playlist(id) => {
                let request = cx.update(|cx| {
                    let api = MusicApi::global(cx);
                    api.runtime
                        .spawn(MusicApi::playlist(api.client.clone(), id))
                });
                request
                    .await
                    .unwrap_or_else(|_| Err("歌单请求任务失败".into()))
                    .map(|(_, songs)| songs)
            }
        };
        let songs = match fetched {
            Ok(songs) => songs,
            Err(error) => {
                log::warn!("打开网页唤起的内容失败：{error}");
                return;
            }
        };
        playback.update(cx, |playback, cx| match target {
            OrpheusTarget::Song(id) => playback.play_list(0, songs, id, cx),
            OrpheusTarget::Playlist(id) => playback.play_list_from_start(id, songs, cx),
        });
    })
    .detach();
}

pub fn configure_window(window: &Window, cx: &App) {
    window.on_window_should_close(cx, |window, cx| {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            close_window(window, cx);
            false
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            if cx.global::<Desktop>()._tray.is_none() {
                close_window(window, cx);
                return false;
            }
            cx.global_mut::<Desktop>().bounds = window.window_bounds();
            true
        }
    });
}

pub fn close_window(window: &mut Window, cx: &mut App) {
    cx.global_mut::<Desktop>().bounds = window.window_bounds();
    // 没有 Dock 的平台若托盘初始化失败，保留任务栏入口，避免窗口无法找回。
    #[cfg(not(target_os = "macos"))]
    if cx.global::<Desktop>()._tray.is_none() {
        window.minimize_window();
        return;
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let handle = window.window_handle();
        // 避免在原生 should-close 回调内部触发新的窗口事件。
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, _| {
                    if let Err(error) = hide_native_window(window) {
                        log::warn!("隐藏窗口失败：{error}");
                    }
                })
                .ok();
        });
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    window.remove_window();
}

pub fn show_window(cx: &mut App) {
    let Some(desktop) = cx.try_global::<Desktop>() else {
        return;
    };
    let window = desktop.window;
    if window
        .update(cx, |_, window, cx| {
            #[cfg(target_os = "windows")]
            if let Err(error) = show_native_window(window) {
                log::warn!("显示窗口失败：{error}");
            }
            cx.activate(true);
            window.activate_window();
        })
        .is_ok()
    {
        return;
    }
    // Linux 没有统一的隐藏窗口 API，关闭原生窗口后重新挂载保留的视图。
    let desktop = cx.global::<Desktop>();
    let view = desktop.view.clone();
    let options = WindowOptions {
        window_bounds: Some(desktop.bounds),
        app_owns_titlebar_drag: true,
        titlebar: Some(TitlebarOptions {
            appears_transparent: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        configure_window(window, cx);
        view
    }) {
        Ok((window, _)) => {
            cx.global_mut::<Desktop>().window = window;
            cx.activate(true);
        }
        Err(error) => log::warn!("显示窗口失败：{error}"),
    }
}

#[cfg(target_os = "macos")]
fn hide_native_window(window: &Window) -> Result<()> {
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).map_err(|error| anyhow!("{error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        anyhow::bail!("需要 AppKit 窗口句柄");
    };
    // 句柄由存活的 GPUI Window 提供，本函数只在 AppKit 主线程执行。
    let view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
    let native = view
        .window()
        .ok_or_else(|| anyhow!("NSView 没有关联窗口"))?;
    native.orderOut(None);
    Ok(())
}

#[cfg(target_os = "windows")]
fn hide_native_window(window: &Window) -> Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    let handle = HasWindowHandle::window_handle(window).map_err(|error| anyhow!("{error}"))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        anyhow::bail!("需要 Win32 窗口句柄");
    };
    // 保留 HWND，Windows 系统媒体会话也依赖它。
    unsafe {
        ShowWindow(handle.hwnd.get() as _, SW_HIDE);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn show_native_window(window: &Window) -> Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SW_SHOW, ShowWindowAsync,
    };
    let handle = HasWindowHandle::window_handle(window).map_err(|error| anyhow!("{error}"))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        anyhow::bail!("需要 Win32 窗口句柄");
    };
    let hwnd = handle.hwnd.get() as _;
    unsafe {
        let command = if IsIconic(hwnd) != 0 {
            SW_RESTORE
        } else {
            SW_SHOW
        };
        if ShowWindowAsync(hwnd, command) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

fn create_tray(cx: &mut App) -> Result<TrayIcon> {
    let menu = Menu::new();
    let show = MenuItem::new("显示窗口", true, None);
    let quit = MenuItem::new("退出", true, None);
    menu.append_items(&[&show, &quit])?;
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();
    let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
    let clicks = sender.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = clicks.send(false);
        }
    }));
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == show_id {
            let _ = sender.send(false);
        } else if event.id == quit_id {
            let _ = sender.send(true);
        }
    }));
    let pixels = tray_pixels(cx)?;
    let builder = TrayIconBuilder::new()
        .with_tooltip("网易云音乐")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.with_icon(tray_icon::Icon::from_rgba(pixels, 32, 32)?);
    let tray = builder.build()?;
    #[cfg(target_os = "macos")]
    set_mac_tray_icon(&tray, &pixels)?;
    // 原生托盘回调只发送命令，窗口操作回到 GPUI 主线程。
    cx.spawn(async move |cx| {
        while let Some(quit) = events.recv().await {
            cx.update(|cx| {
                if quit {
                    cx.quit();
                } else {
                    show_window(cx);
                }
            });
        }
    })
    .detach();
    Ok(tray)
}

fn tray_pixels(cx: &App) -> Result<Vec<u8>> {
    let bytes = cx
        .asset_source()
        .load("icons/logo/tray.svg")?
        .ok_or_else(|| anyhow!("缺少托盘图标 tray.svg"))?;
    rasterize_logo(&cx.svg_renderer(), &bytes)
}

#[cfg(target_os = "macos")]
fn set_mac_tray_icon(tray: &TrayIcon, pixels: &[u8]) -> Result<()> {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage};
    use objc2_foundation::NSSize;
    anyhow::ensure!(pixels.len() == 32 * 32 * 4, "托盘图标必须为 32×32 RGBA");
    // 直接使用像素，避开当前 macOS ImageIO 在托盘 PNG 解码路径上的启动崩溃。
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), 32, 32, 8, 4, true, false,
            NSDeviceRGBColorSpace, NSBitmapFormat::AlphaNonpremultiplied, 32 * 4, 32,
        )
    }.ok_or_else(|| anyhow!("无法创建托盘位图"))?;
    let data = bitmap.bitmapData();
    anyhow::ensure!(!data.is_null(), "托盘位图没有像素缓冲区");
    // AppKit 分配并持有 32×32、每行 128 字节的缓冲区；长度已在上面校验。
    unsafe {
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), data, pixels.len());
    }
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(16., 16.));
    image.addRepresentation(&bitmap);
    image.setTemplate(true);
    let item = tray
        .ns_status_item()
        .ok_or_else(|| anyhow!("缺少菜单栏状态项"))?;
    let main_thread = MainThreadMarker::new().ok_or_else(|| anyhow!("托盘必须在主线程创建"))?;
    let button = item
        .button(main_thread)
        .ok_or_else(|| anyhow!("缺少菜单栏按钮"))?;
    button.setImage(Some(&image));
    item.setLength(22.);
    Ok(())
}

fn rasterize_logo(renderer: &SvgRenderer, bytes: &[u8]) -> Result<Vec<u8>> {
    let parsed = renderer.parse_svg(bytes)?;
    let image = renderer.render_parsed(
        &parsed,
        SvgSize::ExactSize(size(DevicePixels(32), DevicePixels(32))),
    )?;
    let mut rgba = image
        .as_bytes(0)
        .ok_or_else(|| anyhow!("图标栅格化失败"))?
        .to_vec();
    // GPUI 图像为 BGRA，系统托盘接口要求 RGBA。
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    // 避免 glob 引入 GPUI 的 test 宏。
    use super::{BASE64, OrpheusTarget, parse_orpheus_url};
    use base64::Engine as _;

    #[test]
    fn parses_song_url_from_web_page() {
        // music.163.com 歌曲页「点击打开客户端」实际发出的链接。
        let url = "orpheus://eyJ0eXBlIjoic29uZyIsImlkIjoiMjE5NDgwNzciLCJjbWQiOiJwbGF5In0=";
        assert_eq!(parse_orpheus_url(url), Some(OrpheusTarget::Song(21948077)));
    }

    #[test]
    fn parses_playlist_and_tolerates_missing_padding() {
        let url = "orpheus://eyJ0eXBlIjoicGxheWxpc3QiLCJpZCI6IjM3Nzg2NzgiLCJjbWQiOiJwbGF5In0";
        assert_eq!(
            parse_orpheus_url(url),
            Some(OrpheusTarget::Playlist(3778678))
        );
    }

    #[test]
    fn rejects_unrelated_payloads() {
        // base64 有效但不是 JSON。
        assert_eq!(parse_orpheus_url("orpheus://aGVsbG8="), None);
        assert_eq!(parse_orpheus_url("https://music.163.com/song?id=1"), None);
        assert_eq!(parse_orpheus_url("orpheus://"), None);
        // JSON 有效但资源类型不支持。
        let encoded = BASE64.encode(br#"{"type":"album","id":"1"}"#);
        assert_eq!(parse_orpheus_url(&format!("orpheus://{encoded}")), None);
    }
}
