//! 「收藏到歌单」弹窗：从播放栏和歌曲行的收藏图标打开。
//!
//! 三条硬性约束都在这里落地：
//! - **单实例**：句柄存在 `Desktop` 里，已经开着就只把它带到前台，不新建。
//! - **居中于点开它的那个窗口**：位置直接取点击时传进来的父窗口外框，见 [`open_collect_window`]。
//! - **标题栏可拖拽**：窗口声明 `app_owns_titlebar_drag`，拖动交给 `window_drag_area`
//!   调 `start_window_move`（macOS 上 AppKit 不再代劳拖拽）。
//!
//! 窗口按钮走平台惯例：macOS 用系统红绿灯，只保留关闭红灯（不可缩放、不可最小化，
//! 系统便不会画绿 / 黄灯）；其它平台在右上角自绘一个叉号。
//!
//! 内容是一个可滚动的歌单列表：第一项「创建新歌单」，第二项「喜欢的音乐」，
//! 之后是其它歌单。列表数据来自 [`MusicLibrary`]，加载完成会通过订阅刷新。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{ColorTokens, Scrollbar, ScrollbarMode, Theme};

use crate::desktop;
use crate::models::Playlist;
use crate::state::library::MusicLibrary;
use crate::ui::assets::thumbnail_url;
use crate::ui::theme::PRESSED_ICON_ALPHA;

/// 竖排版弹窗的初始尺寸。
const WINDOW_SIZE: Size<Pixels> = size(px(441.), px(528.));
/// 标题距顶部留出的空白：macOS 的红绿灯就在这段里。
const TITLE_TOP_GAP: Pixels = px(27.);
/// 标题与歌单列表之间的留白。
const TITLE_BOTTOM_GAP: Pixels = px(18.);
/// 歌单项左右内边距。窗口整体不加 padding，靠它把内容推进来。
const ROW_PADDING_X: Pixels = px(26.);
/// 歌单项上下内边距；加上 46px 封面正好 66px 一行。
const ROW_PADDING_Y: Pixels = px(10.);
/// 封面尺寸与圆角。
const COVER_SIZE: Pixels = px(46.);
const COVER_RADIUS: Pixels = px(8.);
/// 封面与文字之间的间隔。
const COVER_GAP: Pixels = px(18.);

/// 打开（或聚焦）收藏弹窗。
///
/// `parent` 是点开图标的那个窗口。**必须用它来定位置**，不能用 `Desktop.window`
/// 再去 `update` 主窗口：点击处理本身就跑在主窗口的 update 里，此时该窗口已被 GPUI
/// 从窗口表里取出，`update` 必定返回 `Err`，算出来的位置会静默退回屏幕中央。
pub fn open_collect_window(parent: &Window, cx: &mut App) {
    // 已经开着就只把它带到前台：同一时刻不允许出现多个收藏窗口。
    if let Some(handle) = desktop::collect_window(cx) {
        let focused = handle
            .update(cx, |_, window, cx| {
                cx.activate(true);
                window.activate_window();
            })
            .is_ok();
        if focused {
            return;
        }
        // 窗口已经被关掉了（例如点了 macOS 红灯），句柄失效，先清掉再重开。
        desktop::set_collect_window(None, cx);
    }

    // 父窗口外框的中心就是新窗口的中心。macOS 上这里是屏幕坐标、左上原点，
    // 和 `WindowOptions::window_bounds` 用的是同一套坐标。
    let parent_bounds = parent.window_bounds().get_bounds();
    let bounds = Bounds::centered_at(parent_bounds.center(), WINDOW_SIZE);
    // 跟着父窗口所在的显示器：macOS 创建窗口时把 bounds.origin 当作「相对该显示器」
    // 的坐标，不指定的话会落到主显示器上，多显示器下位置就错了。
    let display_id = parent.display(cx).map(|display| display.id());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        titlebar: Some(TitlebarOptions {
            appears_transparent: true,
            ..Default::default()
        }),
        // 自定义标题栏：拖拽必须由应用自己发起（见 `window_drag_area`）。
        app_owns_titlebar_drag: true,
        // macOS 的红绿灯由 style mask 决定：既不可缩放也不可最小化，就只剩关闭的红灯，
        // 而且是系统原生绘制，不用自绘、也不用去隐藏别的按钮。
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let Some(library) = desktop::library(cx) else {
        eprintln!("音乐库尚未就绪，无法打开收藏窗口");
        return;
    };
    match gpui_kit::open_window(options, cx, move |window, cx| {
        // 原生关闭按钮（macOS 红灯 / 其它平台的窗口关闭）走的都是 should-close，
        // 在这里清掉句柄，避免留下一个失效的窗口引用。
        window.on_window_should_close(cx, |_, cx| {
            desktop::set_collect_window(None, cx);
            true
        });
        cx.new(|cx| CollectWindow::new(library, cx))
    }) {
        Ok((window, _)) => desktop::set_collect_window(Some(window), cx),
        Err(error) => eprintln!("打开收藏窗口失败：{error}"),
    }
}

struct CollectWindow {
    library: Entity<MusicLibrary>,
    scroll: ScrollHandle,
    /// 歌单列表是异步加载的，加载完要重绘。
    _library_subscription: Subscription,
}

impl CollectWindow {
    fn new(library: Entity<MusicLibrary>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&library, |_, _, cx| cx.notify());
        Self {
            library,
            scroll: ScrollHandle::default(),
            _library_subscription: subscription,
        }
    }
}

impl Render for CollectWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let rows = self.playlist_rows(colors, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.foreground)
            // 列表滚动时内容会被窗口边界裁掉，这里统一收口。
            .overflow_hidden()
            // 顶部整条都是拖拽区，标题居中；其它平台在右侧叠一个叉号。
            .child(
                super::window_drag_area("collect-window-titlebar")
                    .relative()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pt(TITLE_TOP_GAP)
                    .pb(TITLE_BOTTOM_GAP)
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("收藏到歌单"),
                    )
                    .when(!cfg!(target_os = "macos"), |bar| {
                        bar.child(close_button(colors))
                    }),
            )
            // 歌单列表：外层 relative 给滚动条定位，内层负责滚动。
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("collect-window-playlists")
                            .flex_1()
                            .min_h(px(0.))
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .children(rows),
                    )
                    .child(Scrollbar::vertical(&self.scroll).mode(ScrollbarMode::Always)),
            )
    }
}

impl CollectWindow {
    /// 组装列表：新建歌单 → 喜欢的音乐 → 其余歌单（保持接口返回的顺序）。
    fn playlist_rows(&self, colors: ColorTokens, cx: &App) -> Vec<AnyElement> {
        let library = self.library.read(cx);
        let mut rows = Vec::with_capacity(library.playlists.len() + 1);
        // 第一项特殊处理：没有封面，用加号占位，也不显示歌曲数。
        rows.push(playlist_row(
            ("collect-window-new", 0),
            create_playlist_cover(colors),
            "创建新歌单".to_owned(),
            None,
            colors,
        ));
        // 第二项是「喜欢的音乐」（`special_type == 5`），它不参与下面的其它列表。
        if let Some(favorite) = library.favorite_playlist() {
            rows.push(playlist_row(
                ("collect-window-playlist", favorite.id),
                playlist_cover(favorite.cover_img_url.as_deref(), colors),
                favorite.name.clone(),
                Some(track_count_label(favorite)),
                colors,
            ));
        }
        rows.extend(
            library
                .playlists
                .iter()
                .filter(|playlist| playlist.special_type != 5)
                .map(|playlist| {
                    playlist_row(
                        ("collect-window-playlist", playlist.id),
                        playlist_cover(playlist.cover_img_url.as_deref(), colors),
                        playlist.name.clone(),
                        Some(track_count_label(playlist)),
                        colors,
                    )
                }),
        );
        rows
    }
}

/// 「N首音乐」。
fn track_count_label(playlist: &Playlist) -> String {
    format!("{}首音乐", playlist.track_count)
}

/// 一个歌单项：封面 + 标题 + 歌曲数。整行铺满窗口宽度，hover 才显底色。
fn playlist_row(
    id: (&'static str, u64),
    cover: AnyElement,
    title: String,
    subtitle: Option<String>,
    colors: ColorTokens,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(COVER_GAP)
        .py(ROW_PADDING_Y)
        .px(ROW_PADDING_X)
        .cursor_pointer()
        // 窗口不加整体 padding，hover 底色自然左右通到窗口边缘，和官方一致。
        .hover(|style| style.bg(colors.muted))
        .child(cover)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(16.))
                        .child(title),
                )
                .when_some(subtitle, |column, subtitle| {
                    column.child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(colors.muted_foreground)
                            .child(subtitle),
                    )
                }),
        )
        .into_any_element()
}

/// 「创建新歌单」的占位封面：灰底圆角方块里一个加号图标。
fn create_playlist_cover(colors: ColorTokens) -> AnyElement {
    div()
        .size(COVER_SIZE)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(COVER_RADIUS)
        .bg(colors.muted)
        .child(
            svg()
                .path("icons/加号.svg")
                .size(px(20.))
                .flex_none()
                .text_color(colors.muted_foreground),
        )
        .into_any_element()
}

/// 歌单封面：按 46px 外接框取 92px 的 CDN 缩略图，没封面时留一块灰底。
fn playlist_cover(url: Option<&str>, colors: ColorTokens) -> AnyElement {
    let source = url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|url| thumbnail_url(url, 92));
    div()
        .size(COVER_SIZE)
        .flex_none()
        .rounded(COVER_RADIUS)
        .overflow_hidden()
        .bg(colors.muted)
        .when_some(source, |cover, source| {
            cover.child(
                // `overflow_hidden` 只按矩形裁剪，圆角要直接设在图片上。
                img(source)
                    .size_full()
                    .rounded(COVER_RADIUS)
                    .object_fit(ObjectFit::Cover),
            )
        })
        .into_any_element()
}

/// 非 macOS 平台右上角的关闭叉号。
///
/// 直接用应用自带的 `close.svg`（就是一个叉），尺寸 / 灰度 / 三态和 header 那排图标一致；
/// `occlude` + `stop_propagation` 让按下叉号时不落到标题栏的拖拽逻辑上。
fn close_button(colors: ColorTokens) -> impl IntoElement {
    svg()
        .path("icons/window/close.svg")
        .size(px(16.))
        .absolute()
        // 与截图里叉号的位置对齐：略高于居中的标题。
        .top(px(24.))
        .right(px(20.))
        .flex_none()
        .cursor_pointer()
        .text_color(colors.foreground.alpha(0.6))
        .hover(|style| style.text_color(colors.foreground))
        .id("collect-window-close")
        .role(Role::Button)
        .aria_label("关闭")
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // active 属于 StatefulInteractiveElement，必须跟在 `.id()` 之后。
        .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
        .on_click(|_, window, cx| {
            desktop::set_collect_window(None, cx);
            window.remove_window();
        })
}
