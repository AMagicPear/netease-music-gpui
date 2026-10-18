//! 「收藏到歌单」弹窗：从播放栏、歌曲行和播放队列的收藏图标打开。
//!
//! 三条硬性约束都在这里落地：
//! - **单实例**：句柄存在 `Desktop` 里，已经开着就只把它带到前台，并把目标换成这一次的歌曲。
//! - **居中于点开它的那个窗口**：位置直接取点击时传进来的父窗口外框，见 [`open_collect_window`]。
//! - **标题栏可拖拽**：窗口声明 `app_owns_titlebar_drag`，拖动交给 `window_drag_area`
//!   调 `start_window_move`（macOS 上 AppKit 不再代劳拖拽）。
//!
//! 窗口按钮走平台惯例：macOS 用系统红绿灯，黄灯绿灯真隐藏，只留关闭红灯；
//! 其它平台在右上角自绘一个叉号。
//!
//! 内容是一个可滚动的歌单列表：第一项「创建新歌单」，第二项「喜欢的音乐」，
//! 之后是其它歌单。点歌单就真的把歌曲收进去——请求发在 [`MusicLibrary`] 里，
//! 结果通过 [`SongsCollected`] 事件回到这里：成功就静默关窗，失败留在窗口里可以重试。
//! 请求不在这里 `spawn`：窗口随时会被关掉，而请求和本地记账必须活着。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::input::{Input, InputEditorStyle, InputEvent, InputState};
use gpui_kit::base::{ColorTokens, Root, Scrollbar, ScrollbarMode, Theme};

use super::hide_traffic_lights;
use crate::desktop;
use crate::models::{Playlist, Song};
use crate::state::library::{MusicLibrary, SongsCollected};
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
/// `songs` 是这次要收藏的歌曲：歌曲行是一首，「收藏全部」是整个播放队列。
///
/// `parent` 是点开图标的那个窗口。**必须用它来定位置**，不能用 `Desktop.window`
/// 再去 `update` 主窗口：点击处理本身就跑在主窗口的 update 里，此时该窗口已被 GPUI
/// 从窗口表里取出，`update` 必定返回 `Err`，算出来的位置会静默退回屏幕中央。
pub fn open_collect_window(parent: &Window, songs: Vec<Song>, cx: &mut App) {
    if songs.is_empty() {
        return;
    }
    // 已经开着就只把它带到前台：同一时刻不允许出现多个收藏窗口。
    if let Some(handle) = desktop::collect_window(cx) {
        let replacement = songs.clone();
        let focused = handle
            .update(cx, |view, window, cx| {
                cx.activate(true);
                window.activate_window();
                // 窗口是单实例的，但「点哪首歌收藏哪首歌」不能被上一次的打开状态带偏：
                // 把目标换成这次点开的那一批。
                //
                // 注意这一层包了两层：`open_window` 把页面塞进 gpui-kit 的 `Root` 里，
                // 所以窗口的根视图是 `Root` 而不是 `CollectWindow`，要再往里取一层。
                let Ok(root) = view.downcast::<Root>() else {
                    return;
                };
                let content = root.read(cx).view().clone();
                if let Ok(view) = content.downcast::<CollectWindow>() {
                    view.update(cx, |this, cx| this.set_songs(replacement, cx));
                }
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
        // 黄灯绿灯真隐藏（不是灰显），只留关闭红灯；登录窗同款机制。
        #[cfg(target_os = "macos")]
        hide_traffic_lights(window, true);
        // 原生关闭按钮（macOS 红灯 / 其它平台的窗口关闭）走的都是 should-close，
        // 在这里清掉句柄，避免留下一个失效的窗口引用。
        window.on_window_should_close(cx, |_, cx| {
            desktop::set_collect_window(None, cx);
            true
        });
        cx.new(|cx| CollectWindow::new(library, songs, window, cx))
    }) {
        Ok((window, _)) => desktop::set_collect_window(Some(window), cx),
        Err(error) => eprintln!("打开收藏窗口失败：{error}"),
    }
}

struct CollectWindow {
    library: Entity<MusicLibrary>,
    /// 这次要收藏的歌曲。歌曲行的收藏图标是一首，「收藏全部」是整个队列。
    songs: Vec<Song>,
    scroll: ScrollHandle,
    /// 「创建新歌单」展开后的名称输入框。
    name_input: Entity<InputState>,
    /// 是否处于「输入歌单名」的状态。
    creating: bool,
    /// 请求在飞：期间不再接受点击，避免一次收藏被点成两次。
    submitting: bool,
    /// 歌单列表是异步加载的，加载完要重绘。
    _library_subscription: Subscription,
    /// 收藏结果由音乐库广播回来。
    _collect_subscription: Subscription,
    _input_subscription: Subscription,
}

impl CollectWindow {
    fn new(
        library: Entity<MusicLibrary>,
        songs: Vec<Song>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("歌单名");
            state.set_editor_style(InputEditorStyle {
                selection: Theme::global(cx).tokens.colors.selection,
                ..Default::default()
            });
            state
        });
        let subscription = cx.observe(&library, |_, _, cx| cx.notify());
        let collect_subscription = cx.subscribe_in(
            &library,
            window,
            |this, _, event: &SongsCollected, window, cx| this.collected(event, window, cx),
        );
        // 回车即创建：输入框是这里唯一的输入，不需要额外的「按按钮」提示。
        let input_subscription =
            cx.subscribe_in(&name_input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.create_and_collect(cx);
                }
            });
        Self {
            library,
            songs,
            scroll: ScrollHandle::default(),
            name_input,
            creating: false,
            submitting: false,
            _library_subscription: subscription,
            _collect_subscription: collect_subscription,
            _input_subscription: input_subscription,
        }
    }

    /// 换成一批新的目标歌曲（窗口已开着时又点了别的歌）。
    fn set_songs(&mut self, songs: Vec<Song>, cx: &mut Context<Self>) {
        // 正在提交时不换目标：那一次请求已经发出去了，换掉只会让状态对不上。
        if self.submitting || songs.is_empty() {
            return;
        }
        self.songs = songs;
        self.creating = false;
        cx.notify();
    }

    /// 提交收藏。真正的请求和本地记账都在音乐库里做。
    fn collect(&mut self, playlist_id: u64, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        self.submitting = true;
        let songs = self.songs.clone();
        self.library.update(cx, |library, cx| {
            library.collect_songs(playlist_id, songs, cx)
        });
        cx.notify();
    }

    /// 展开「创建新歌单」的输入框并聚焦，键盘直接可以打字。
    fn start_creating(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        self.creating = true;
        self.name_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn cancel_creating(&mut self, cx: &mut Context<Self>) {
        self.creating = false;
        cx.notify();
    }

    fn create_and_collect(&mut self, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let name = self.name_input.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return;
        }
        self.submitting = true;
        let songs = self.songs.clone();
        self.library.update(cx, |library, cx| {
            library.create_playlist_and_collect(name, songs, cx)
        });
        cx.notify();
    }

    /// 音乐库把结果广播回来了：成功直接关窗——这个窗口只为收藏存在，关掉就是答复。
    fn collected(&mut self, event: &SongsCollected, window: &mut Window, cx: &mut Context<Self>) {
        self.submitting = false;
        match &event.result {
            Ok(()) => {
                self.creating = false;
                self.close(window, cx);
            }
            // 界面上不摆提示，至少别让失败彻底无声：终端留一行，排查接口问题用。
            Err(message) => eprintln!("收藏失败：{message}"),
        }
        cx.notify();
    }

    /// 关窗放到下一个任务里：这里正处在音乐库的 `update` / 事件广播中，
    /// 立刻 `remove_window` 会把正在跑的订阅链打断。
    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |_, cx| {
            // 用户可能已经自己关掉了：`update` 失败就什么都不用做。
            let _ = cx.update(|window, cx| {
                desktop::set_collect_window(None, cx);
                window.remove_window();
            });
        })
        .detach();
    }

    /// 组装列表：新建歌单 → 喜欢的音乐 → 其余歌单（保持接口返回的顺序）。
    fn playlist_rows(&self, colors: ColorTokens, cx: &Context<Self>) -> Vec<AnyElement> {
        let library = self.library.read(cx);
        let mut rows = Vec::with_capacity(library.playlists.len() + 1);
        // 第一项特殊处理：没有封面，用加号占位，也不显示歌曲数。
        if self.creating {
            rows.push(self.create_row(colors, cx));
        } else {
            rows.push(playlist_row(
                ("collect-window-new", 0),
                create_playlist_cover(colors),
                "创建新歌单".to_owned(),
                None,
                colors,
                cx.listener(|this, _, window, cx| this.start_creating(window, cx)),
            ));
        }
        // 第二项是「喜欢的音乐」（`special_type == 5`），它不参与下面的其它列表。
        if let Some(favorite) = library.favorite_playlist() {
            let id = favorite.id;
            rows.push(playlist_row(
                ("collect-window-playlist", id),
                playlist_cover(favorite.cover_img_url.as_deref(), colors),
                favorite.name.clone(),
                Some(track_count_label(favorite)),
                colors,
                cx.listener(move |this, _, _, cx| this.collect(id, cx)),
            ));
        }
        rows.extend(
            library
                .playlists
                .iter()
                .filter(|playlist| playlist.special_type != 5)
                .map(|playlist| {
                    let id = playlist.id;
                    playlist_row(
                        ("collect-window-playlist", id),
                        playlist_cover(playlist.cover_img_url.as_deref(), colors),
                        playlist.name.clone(),
                        Some(track_count_label(playlist)),
                        colors,
                        cx.listener(move |this, _, _, cx| this.collect(id, cx)),
                    )
                }),
        );
        rows
    }

    /// 「创建新歌单」展开后的一行：名称输入框 + 取消 / 创建。
    fn create_row(&self, colors: ColorTokens, cx: &Context<Self>) -> AnyElement {
        div()
            .id("collect-window-create")
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .py(ROW_PADDING_Y)
            .px(ROW_PADDING_X)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h(COVER_SIZE)
                    .flex()
                    .items_center()
                    .px(px(12.))
                    .rounded(COVER_RADIUS)
                    .border_1()
                    .border_color(colors.border)
                    .text_size(px(14.))
                    // 输入框自己吃掉按下事件，否则会被窗口拖拽区当成拖拽起点。
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(Input::new(&self.name_input)),
            )
            .child(text_button(
                "collect-window-create-cancel",
                "取消",
                colors,
                cx.listener(|this, _, _, cx| this.cancel_creating(cx)),
            ))
            .child(text_button(
                "collect-window-create-confirm",
                "创建",
                colors,
                cx.listener(|this, _, _, cx| this.create_and_collect(cx)),
            ))
            .into_any_element()
    }
}

impl Render for CollectWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let rows = self.playlist_rows(colors, cx);
        // 多首（「收藏全部」）时标题说明数量，否则和官方一样只写「收藏到歌单」。
        let title = if self.songs.len() > 1 {
            format!("收藏 {} 首到歌单", self.songs.len())
        } else {
            "收藏到歌单".to_owned()
        };
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
                            .child(title),
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

/// 「N首音乐」。
fn track_count_label(playlist: &Playlist) -> String {
    format!("{}首音乐", playlist.track_count)
}

/// 小文字按钮：「创建新歌单」那行的「取消 / 创建」用，灰字、悬停加深。
fn text_button(
    id: &'static str,
    label: &'static str,
    colors: ColorTokens,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(colors.muted_foreground)
        .hover(|style| style.text_color(colors.foreground))
        .on_click(on_click)
        .child(label)
}

/// 一个歌单项：封面 + 标题 + 歌曲数。整行铺满窗口宽度，hover 才显底色，点一下收藏。
fn playlist_row(
    id: (&'static str, u64),
    cover: AnyElement,
    title: String,
    subtitle: Option<String>,
    colors: ColorTokens,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
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
        .role(Role::Button)
        .aria_label(title.clone())
        // 窗口不加整体 padding，hover 底色自然左右通到窗口边缘，和官方一致。
        .hover(|style| style.bg(colors.muted))
        .on_click(on_click)
        .child(cover)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().truncate().text_size(px(16.)).child(title))
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
        .size(px(20.))
        .text_color(colors.foreground.alpha(0.6))
        .hover(|style| style.text_color(colors.foreground))
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
