use anyhow::anyhow;
use gpui::*;
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

use crate::ui::shell::MainWindow;

/// 应用持有视图和托盘；窗口不可见时，播放 Entity 和页面状态仍然存活。
struct Desktop {
    window: AnyWindowHandle,
    view: Entity<MainWindow>,
    bounds: WindowBounds,
    _tray: Option<TrayIcon>,
}

impl Global for Desktop {}

pub fn init(window: AnyWindowHandle, view: Entity<MainWindow>, cx: &mut App) {
    let bounds = window
        .update(cx, |_, window, _| window.window_bounds())
        .unwrap();
    cx.set_global(Desktop {
        window,
        view,
        bounds,
        _tray: None,
    });
    // 托盘需要在原生事件循环启动后、GPUI 主线程上创建。
    cx.spawn(async |cx| {
        cx.update(|cx| match create_tray(cx) {
            Ok(tray) => cx.global_mut::<Desktop>()._tray = Some(tray),
            Err(error) => eprintln!("系统托盘初始化失败：{error}"),
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
                        eprintln!("隐藏窗口失败：{error}");
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
                eprintln!("显示窗口失败：{error}");
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
        Err(error) => eprintln!("显示窗口失败：{error}"),
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
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(rgba)
}
