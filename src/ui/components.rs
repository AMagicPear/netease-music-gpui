use crate::models::AudioQualityLevel;
use gpui::{
    Div, Entity, Hsla, InteractiveElement, IntoElement, MouseButton, Role, Stateful,
    StatefulInteractiveElement, WindowControlArea, div, img, px, svg,
};
use gpui::{ParentElement as _, Styled as _};
use gpui_kit::base::ColorTokens;
use gpui_kit::base::input::InputState;

use crate::ui::theme::PRESSED_ICON_ALPHA;

mod collect_window;
mod drag_preview;
mod message_panel;
mod play_queue;
mod player_bar;
mod playing_indicator;
mod popover;
mod progress_bar;
mod side_panel;
mod song_menu;
mod song_table;
mod spinner;
mod tab_bar;
mod text;
mod vinyl;
mod vinyl_stage;
mod virtual_table;
mod volume_control;

pub use collect_window::open_collect_window;
pub use drag_preview::ResizeDragPreview;
pub use player_bar::{PLAYER_BAR_HEIGHT, PlayerBar};
pub use playing_indicator::PlayingIndicator;
pub use side_panel::{SidePanel, SidePanelContent};
pub use song_menu::{Collect, PlayNext, show_song_menu};
// 播放队列浮层、音质弹窗、账号弹层共用同一份圆角与投影；
// 子模块与 ui 模块（shell）都通过这里取名。
pub(in crate::ui) use popover::{SURFACE_RADIUS, SurfacePopover, surface_shadow};
pub use song_table::{
    ACTION_BUTTON_HEIGHT, ACTION_BUTTON_PADDING, DURATION_COLUMN_WIDTH, KeepRowHover,
    LIKE_COLUMN_WIDTH, SECONDARY_FONT_WEIGHT, SongDisplay, centered_empty, hovered_row_after,
    index_column_width, play_all_button, play_count_label, row_hover_icon, title_with_subtitle,
};
pub use spinner::spinner;
pub use tab_bar::{TabBar, TabChanged, TabItem};
use text::match_highlight;
pub(super) use text::{keyword_ranges, match_highlights, text_with_matches};
pub(in crate::ui) use vinyl::{RotationClock, Vinyl};
pub use vinyl_stage::{Tonearm, vinyl_stage};
pub use virtual_table::{
    CELL_PADDING, COLUMN_GAP, HEADER_HEIGHT, PAGE_SIDE_PADDING, ROW_HEIGHT, TableColumn,
    virtual_table,
};

// 浮层都用 deferred 绘制，数字大的画得更晚、盖在上面。
//
// 进度条在最底下；播放队列浮层压在它上面。音量气泡最小也最临时，它从图标上方
// 弹出时会和弹层重叠，所以压在最上面——音质弹窗由 gpui-kit 的 `base::Popup` 绘制，
// 占用 `POPUP_PRIORITY`，气泡必须比它更高才盖得住。
pub(super) const LAYER_PROGRESS_BAR: usize = 1;
pub(super) const LAYER_POPOVER: usize = 2;
pub(super) const LAYER_VOLUME_BALLOON: usize = gpui_kit::base::POPUP_PRIORITY + 1;

/// 暗背景图标只稍微提亮，浅背景仍使用主题前景色。
pub(super) fn icon_hover_color(mut base: Hsla, colors: ColorTokens) -> Hsla {
    if colors.background.l >= 0.5 {
        return colors.foreground;
    }
    if base.a < 1. {
        base.a = (base.a + 0.08).min(1.);
    } else {
        base.l = (base.l + 0.03).min(1.);
    }
    base
}

// 下面是跨页面共用的展示规则。同一个规则只在歌单列表和播放栏各写一遍很容易漂移
// （比如换了素材只改一处），所以集中在这里，由调用方决定怎么渲染。

/// 隐藏 macOS 窗口红绿灯里的部分按钮。登录窗只隐藏绿灯（保留可最小化），
/// 收藏窗黄灯绿灯都隐藏。gpui 的 WindowOptions 没有“隐藏某个灯”的开关，
/// style mask 只能整体去能力（隐藏 / 灰显），所以直接操作原生 NSWindow。
#[cfg(target_os = "macos")]
pub(super) fn hide_traffic_lights(window: &gpui::Window, hide_minimize: bool) {
    use objc2::msg_send;
    use objc2_app_kit::{NSWindowButton, NSWindowStyleMask};
    use raw_window_handle::AppKitWindowHandle;

    // Window 的固有方法 window_handle() 返回 AnyWindowHandle，会遮蔼 trait
    // 同名方法，拿不到原始句柄；必须全限定调用 trait 方法。
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return;
    };
    let raw_window_handle::RawWindowHandle::AppKit(AppKitWindowHandle { ns_view, .. }) =
        handle.as_raw()
    else {
        return;
    };
    unsafe {
        // gpui 的 macOS 后端把根 NSView 存在 handle 里，取它所在的 NSWindow。
        let view: *mut objc2_app_kit::NSView = ns_view.as_ptr().cast();
        let window: Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> = msg_send![view, window];
        let Some(window) = window else { return };
        if let Some(zoom) = window.standardWindowButton(NSWindowButton::ZoomButton) {
            zoom.setHidden(true);
        }
        if hide_minimize
            && let Some(minimize) = window.standardWindowButton(NSWindowButton::MiniaturizeButton)
        {
            minimize.setHidden(true);
        }
        // 双击标题栏按系统设置可能触发最大化，去掉 FullScreen 可用性一并禁掉。
        window.setStyleMask(window.styleMask() & !NSWindowStyleMask::FullScreen);
    }
}

/// Windows 使用原生命中区，macOS/Linux 在按下时将拖拽交给系统。
pub(super) fn window_drag_area(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if !cfg!(target_os = "windows") {
                if event.click_count == 2 {
                    if cfg!(target_os = "macos") {
                        window.titlebar_double_click();
                    } else {
                        window.zoom_window();
                    }
                } else {
                    window.start_window_move();
                }
            }
        })
}

/// 空状态占位：官方 cm_empty_* 位图（180px 逻辑尺寸，600px 源图足够 Retina）+ 说明文字。
/// 官方 CSS：`.BackImage{width:180px;height:180px}`，下方紧跟灰字提示。
pub(super) fn empty_state(image: &'static str, tip: &str, colors: ColorTokens) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .py_8()
        .gap(px(8.))
        .child(img(image).w(px(180.)).h(px(180.)))
        .child(
            div()
                .text_size(px(14.))
                .text_color(colors.muted_foreground)
                .child(tip.to_string()),
        )
}

/// 搜索框右侧的「清空」按钮：点了把输入框置空。顶栏全局搜索和歌单内搜索共用，
/// 显不显示由调用方决定（约定：输入框有内容才出现）。
///
/// 常态 45% 前景色：比 `PRESSED_ICON_ALPHA`（0.4）深、比 `muted_foreground`（0.6）浅，
/// 满足「按下最浅 < 常态 < hover」的三态次序，hover 才有一档可见的变化。
pub(super) fn search_clear_button(
    input: Entity<InputState>,
    id: &'static str,
    colors: ColorTokens,
) -> impl IntoElement {
    svg()
        .path("icons/clear.svg")
        // 18px：唯一的「删除」入口，事候区太小难点中，宁可略大于放大镜。
        .size(px(18.))
        .flex_none()
        .text_color(colors.foreground.alpha(0.45))
        .hover(|style| style.text_color(colors.muted_foreground))
        .id(id)
        .role(Role::Button)
        .aria_label("清空搜索")
        // active 属于 StatefulInteractiveElement，必须跟在 `.id()` 之后（此时是 Stateful<Svg>）。
        .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
        // `replace_all` 而不是 `set_value`：后者不发 `InputEvent::Change`，
        // 靠订阅驱动的过滤不会重算；前者照常发出，且记进撤销栈可 Cmd+Z 找回。
        .on_click(move |_, window, cx| {
            input.update(cx, |input, cx| input.replace_all("", window, cx));
        })
}

/// 音质徽章的图标，素材与官方「音质选项」一一对应。
/// 标准 / 较高 / 极高 共用 HQ，其余档位各有专属素材。
pub(super) fn quality_badge_path(quality: AudioQualityLevel) -> &'static str {
    match quality {
        AudioQualityLevel::Standard | AudioQualityLevel::Higher | AudioQualityLevel::ExHigh => {
            "icons/音质选项/HQ.svg"
        }
        AudioQualityLevel::Lossless => "icons/音质选项/sq.svg",
        AudioQualityLevel::HiRes => "icons/音质选项/Hi-Res.svg",
        AudioQualityLevel::JyEffect => "icons/音质选项/高清臻音.svg",
        AudioQualityLevel::Sky => "icons/音质选项/沉浸声.svg",
        AudioQualityLevel::Dolby => "icons/音质选项/全景声.svg",
        AudioQualityLevel::JyMaster => "icons/音质选项/超清母带.svg",
    }
}

/// 喜欢状态的图标：已喜欢是实心红心，未喜欢是描线灰心。
/// 只定形状不定颜色——歌单行和播放栏的灰度和 hover 规则并不相同，颜色由调用方给。
pub(super) fn like_icon_path(liked: bool) -> &'static str {
    if liked {
        "icons/heart.svg"
    } else {
        "icons/heart_outline.svg"
    }
}

/// 歌曲时长和播放进度共用的 mm:ss 格式；超过一小时仍按分钟累计。
pub(super) fn format_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// 未知歌手单独变灰，多歌手中已有的名字保持原色。
/// `keyword` 非空时把命中的片段标出来，让「这一行为什么被搜出来」看得见。
pub(super) fn artist_label(
    song: &crate::models::Song,
    keyword: &str,
    colors: gpui_kit::base::ColorTokens,
) -> gpui::StyledText {
    let mut text = String::new();
    let mut hits = Vec::new();
    let mut grays = Vec::new();
    let names = song.ar.iter().map(|artist| artist.name.as_deref());
    // 空歌手列表也显示一个「未知歌手」。
    for name in names.chain(song.ar.is_empty().then_some(None)) {
        if !text.is_empty() {
            text.push_str(" / ");
        }
        let start = text.len();
        match name.filter(|name| !name.trim().is_empty()) {
            Some(name) => {
                text.push_str(name);
                // 每段单独算区间再平移到整串：拼接多段时各段偏移不同。
                hits.extend(match_highlights(name, start, keyword));
            }
            None => {
                text.push_str("未知歌手");
                grays.push((
                    start..text.len(),
                    gpui::HighlightStyle {
                        color: Some(colors.foreground.alpha(1.)),
                        fade_out: Some(0.55),
                        ..Default::default()
                    },
                ));
            }
        }
    }
    gpui::StyledText::new(text)
        .with_highlights(gpui::combine_highlights(hits, grays).collect::<Vec<_>>())
}
