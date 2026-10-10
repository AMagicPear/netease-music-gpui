//! 「播放列表」浮层：贴窗口右缘滑出，展示当前播放队列。
//!
//! 面板本身是独立实体，由播放栏作为子元素挂载（见 `player_bar`）。它用绝对定位向上
//! 越出播放栏、盖住页面内容，收起时完全不参与布局。列表交给 gpui-kit 的 [`List`]：
//! 队列可能上千首，而滑入/滑出的每一帧都会重绘整个面板，必须只渲染可见的十几行。

use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{
    ColorTokens, IndexPath, InteractiveElementExt as _, Presence, Selectable, Theme, Transition,
};
use gpui_kit::component::list::{List, ListDelegate, ListState};

use super::{
    LAYER_POPOVER, PLAYER_BAR_HEIGHT, SURFACE_RADIUS, artist_label, format_duration,
    like_icon_path, quality_badge_path, surface_shadow,
};
use crate::models::Song;
use crate::playback::PlaybackController;
use crate::state::library::MusicLibrary;
use crate::ui::assets::{track_cover_image, track_cover_url};
use crate::ui::shell::HEADER_HEIGHT;
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA};

/// 面板固定宽度。
const PANEL_WIDTH: Pixels = px(386.);
/// 面板与上方头部、下方播放栏之间留出的间隔。
const PANEL_GAP: Pixels = px(12.);
/// 面板自己的头部高度；「播放列表」和计数排在这一行里。
const PANEL_HEADER_HEIGHT: Pixels = px(58.);
/// 每一首歌占的高度。
const ROW_HEIGHT: Pixels = px(64.);
/// 头部与每一行的左右留白。面板本身不再额外加 padding。
const ROW_PADDING: Pixels = px(20.);
/// 行内封面尺寸。
const COVER_SIZE: Pixels = px(48.);
/// 封面圆角。
const COVER_RADIUS: Pixels = px(8.);
/// 封面上播放键的宽度。
const PLAY_BUTTON_SIZE: Pixels = px(20.);
/// 滑入/滑出时长。
const REVEAL_DURATION: Duration = Duration::from_millis(260);

/// `group_hover` 按名字全局查分组，同名会互相点亮，所以每行要一个不同的组名。
fn row_group(index: usize) -> SharedString {
    SharedString::from(format!("playlist-panel-row-{index}"))
}

/// 面板里会变化、又影响画面的那几项：队列内容、当前歌曲、播放/暂停意图。
///
/// 播放控制器每 100ms 为进度通知一次，直接跟着重绘会白跑十几次；这里只在真正
/// 影响列表的东西变了才刷新。
fn panel_playback_state(playback: &PlaybackController) -> (u64, Option<usize>, bool) {
    (
        playback.queue_revision(),
        playback.queue_cursor(),
        playback.is_play_requested(),
    )
}

pub struct PlaylistPanel {
    playback: Entity<PlaybackController>,
    list: Entity<ListState<QueueDelegate>>,
    /// 期望的开合状态。画面上的位置由 `Presence` 采样出的进度决定，收起后还要滑出去。
    open: bool,
    scroll_to_current: bool,
    _playback_subscription: Subscription,
    _library_subscription: Subscription,
}

impl PlaylistPanel {
    pub fn new(
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let delegate = QueueDelegate {
            playback: playback.clone(),
            library: library.clone(),
            hovered: None,
        };
        // 只当作品虚拟滚动容器用：不做选中高亮，点击行为由每一行自己处理。
        let list = cx.new(|cx| ListState::new(delegate, window, cx).selectable(false));

        let mut playback_state = panel_playback_state(&playback.read(cx));
        let _playback_subscription = cx.observe(&playback, move |this, playback, cx| {
            let next = panel_playback_state(&playback.read(cx));
            if next == playback_state {
                return;
            }
            playback_state = next;
            if this.open {
                this.list.update(cx, |_, cx| cx.notify());
                cx.notify();
            }
        });

        // 喜欢状态由音乐库维护：别处（歌单页、播放栏红心）改动后，可见行的爱心要跟着变。
        // 收起时不渲染，等展开时 `set_open` 会统一刷新一次。
        let _library_subscription = cx.observe(&library, |this, _, cx| {
            if this.open {
                this.list.update(cx, |_, cx| cx.notify());
                cx.notify();
            }
        });

        Self {
            playback,
            list,
            open: false,
            scroll_to_current: false,
            _playback_subscription,
            _library_subscription,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.scroll_to_current = open;
        if open {
            // 收起期间队列可能已经换了，展开前先让列表重新量一遍；
            // 悬停行也一并清掉，免得展开时顶着一行过期的操作图标。
            self.list.update(cx, |list, cx| {
                list.delegate_mut().hovered = None;
                cx.notify();
            });
        }
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        let open = !self.open;
        self.set_open(open, cx);
    }
}

impl Render for PlaylistPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 进出场都靠这一个采样：收起后仍会继续渲染到进度归零，才滑得出去。
        let presence = Presence::new("player-playlist-panel", self.open)
            .transition(Transition::new(REVEAL_DURATION))
            .sample(window, cx);
        if !presence.should_render() {
            return div().into_any_element();
        }
        let width = PANEL_WIDTH.min(window.viewport_size().width);
        let offset = -(1. - presence.progress) * f32::from(width);
        let height = (window.viewport_size().height
            - px(HEADER_HEIGHT)
            - px(PLAYER_BAR_HEIGHT)
            - PANEL_GAP * 2.)
            .max(px(0.));
        let colors = Theme::global(cx).tokens.colors;
        let count = self.playback.read(cx).queue().len();
        if self.scroll_to_current {
            self.scroll_to_current = false;
            let index = self.playback.read(cx).queue_cursor().unwrap_or(0);
            self.list.update(cx, |list, cx| {
                list.scroll_to_item(IndexPath::new(index), ScrollStrategy::Top, window, cx);
            });
        }

        // 与其它浮层一样延后绘制，避免被页面和进度条的 deferred 元素盖住。
        deferred(
            div()
                .id("player-playlist-panel")
                .absolute()
                // 右缘对齐窗口；被滑出时整体挪到窗口右侧之外，由窗口边界裁掉。
                .right(px(offset))
                .bottom(px(PLAYER_BAR_HEIGHT) + PANEL_GAP)
                .w(width)
                .h(height)
                .flex()
                .flex_col()
                .text_color(colors.foreground)
                .bg(colors.surface)
                // 贴着窗口右缘，只需要把左侧两角磨圆；圆角量取自音质浮窗。
                .rounded_l(SURFACE_RADIUS)
                .shadow(vec![surface_shadow(colors)])
                // 遮住后方命中区：面板上的悬停和点击都不穿透到底下的页面。
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // 面板外的任何按下都收起。再点一次列表图标时，这里会先一步关掉它，
                // 图标那边的监听比对着渲染时的值，于是不会再翻回来（见播放栏触发按钮）。
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.set_open(false, cx)))
                .child(
                    div()
                        .h(PANEL_HEADER_HEIGHT.min(height))
                        .flex_none()
                        .flex()
                        .items_center()
                        .pl(ROW_PADDING)
                        // 标题在本面板里放大一档；计数的字号、字重仍和页面 tab 同一套。
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("播放列表"),
                        )
                        .child(
                            div()
                                .ml(px(1.))
                                .text_size(px(13.))
                                .font_weight(FontWeight::BOLD)
                                .font_family(DOLPHIN_FAMILY)
                                .child(count.to_string()),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        // 滚动与滚动条都由 List 自己管。
                        .child(List::new(&self.list)),
                ),
        )
        .with_priority(LAYER_POPOVER)
        .into_any_element()
    }
}

/// 把播放队列喂给 gpui-kit 的列表：只负责"第几行是什么"，样式全在 [`QueueRow`] 里。
struct QueueDelegate {
    playback: Entity<PlaybackController>,
    /// 喜欢状态在音乐库里，行内的爱心要据此显示实心 / 空心。
    library: Entity<MusicLibrary>,
    /// 鼠标压在哪一行（按行号）。悬停时那一行把时长换成操作图标，所以要进实体。
    hovered: Option<usize>,
}

impl ListDelegate for QueueDelegate {
    type Item = QueueRow;

    fn items_count(&self, _section: usize, cx: &App) -> usize {
        self.playback.read(cx).queue().len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let colors = Theme::global(cx).tokens.colors;
        let index = ix.row;
        let count = self.playback.read(cx).queue().len();
        let song = self.playback.read(cx).queue().get(index)?.clone();
        let is_current = self.playback.read(cx).queue_cursor() == Some(index);
        // 当前歌正在播放时按钮变成暂停，其余一律是播放键。
        let show_pause = is_current && self.playback.read(cx).is_play_requested();
        let liked = self.library.read(cx).liked_song_ids.contains(&song.id);
        let state = RowState {
            is_current,
            show_pause,
            liked,
            hovered: self.hovered == Some(index),
        };
        let group = row_group(index);

        let playback = self.playback.clone();
        let on_cover_click: RowCallback<ClickEvent> = Box::new(cx.listener(
            move |_: &mut ListState<QueueDelegate>,
                  event: &ClickEvent,
                  _: &mut Window,
                  cx: &mut Context<ListState<QueueDelegate>>| {
                cx.stop_propagation();
                if event.click_count() != 1 {
                    return;
                }
                playback.update(cx, |playback, cx| {
                    if playback.queue_cursor() == Some(index) {
                        playback.toggle(cx);
                    } else {
                        playback.play_queue_index(index, cx);
                    }
                });
            },
        ));

        let library = self.library.clone();
        let like_song = song.clone();
        let on_like_click: RowCallback<ClickEvent> = Box::new(cx.listener(
            move |_: &mut ListState<QueueDelegate>,
                  event: &ClickEvent,
                  _: &mut Window,
                  cx: &mut Context<ListState<QueueDelegate>>| {
                cx.stop_propagation();
                if event.click_count() != 1 {
                    return;
                }
                library.update(cx, |library, cx| library.toggle_like(like_song.clone(), cx));
            },
        ));

        let playback = self.playback.clone();
        let on_double_click: RowCallback<ClickEvent> = Box::new(cx.listener(
            move |_: &mut ListState<QueueDelegate>,
                  _: &ClickEvent,
                  _: &mut Window,
                  cx: &mut Context<ListState<QueueDelegate>>| {
                cx.stop_propagation();
                playback.update(cx, |playback, cx| playback.play_queue_index(index, cx));
            },
        ));

        let on_hover: RowCallback<bool> = Box::new(cx.listener(
            move |list: &mut ListState<QueueDelegate>,
                  hovered: &bool,
                  _: &mut Window,
                  cx: &mut Context<ListState<QueueDelegate>>| {
                let delegate = list.delegate_mut();
                // 离开事件只清除自己：跨行时「新行进入」和「旧行离开」的先后不确定，
                // 无条件置 None 会把刚亮起来的那一行又熄掉。
                let next = if *hovered {
                    Some(index)
                } else if delegate.hovered == Some(index) {
                    None
                } else {
                    delegate.hovered
                };
                if next != delegate.hovered {
                    delegate.hovered = next;
                    cx.notify();
                }
            },
        ));

        Some(QueueRow {
            id: ElementId::from(("playlist-panel-row", index)),
            group,
            hover_color: colors.muted,
            round_bottom_left: index + 1 == count,
            on_double_click: Some(on_double_click),
            on_hover: Some(on_hover),
            children: row_content(&song, index, state, colors, on_cover_click, on_like_click),
        })
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
    }

    /// 队列为空时什么都不画，留白比一个孤零零的空状态图标更合适。
    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
    }
}

/// 行内交互回调。几处回调只差事件类型，统一成一个别名，省得每处都写一遍
/// `Box<dyn Fn(..) + 'static>`。
type RowCallback<E> = Box<dyn Fn(&E, &mut Window, &mut App) + 'static>;

/// 一行里随播放 / 悬停状态变的那几项。
///
/// 四个 bool 连排时调用点根本读不出谁是谁，打包成一个参数传。
#[derive(Clone, Copy)]
struct RowState {
    /// 这一行就是当前歌：歌名和封面遮罩要按它高亮。
    is_current: bool,
    /// 当前歌正在播放：封面按钮显示暂停键。
    show_pause: bool,
    /// 在音乐库里已喜欢：爱心是实心红心。
    liked: bool,
    /// 鼠标正压在这一行：右侧显示操作图标而不是时长。
    hovered: bool,
}

/// 行内容：封面（含遮罩和播放键）、歌名 + 作者 / 音质、时长（悬停整行时换成操作图标）。
///
/// 截断方式照抄歌单页的同一行；封面和歌名在本面板里放大过，作者仍用同一字号。
fn row_content(
    song: &Song,
    index: usize,
    state: RowState,
    colors: ColorTokens,
    on_cover_click: RowCallback<ClickEvent>,
    on_like_click: RowCallback<ClickEvent>,
) -> Vec<AnyElement> {
    let RowState {
        is_current,
        show_pause,
        liked,
        hovered,
    } = state;
    // 封面遮罩的悬停分组名。悬停状态在委托里，这里只借用它做纯样式切换。
    let group = row_group(index);

    let cover = div()
        .relative()
        .size(COVER_SIZE)
        .flex_none()
        .rounded(COVER_RADIUS)
        .overflow_hidden()
        .bg(colors.muted)
        .child(
            img(track_cover_image(track_cover_url(
                song.al.pic_url.as_deref(),
                72,
            )))
            .size_full()
            .rounded(COVER_RADIUS)
            .object_fit(ObjectFit::Cover),
        )
        .child(
            // 遮罩平时透明；鼠标压在整行上、或这一行就是当前歌时显出来。
            // 只改 `opacity` 这种纯样式，可以走 `group_hover`，不必读委托里的悬停状态；
            // 换元素（时长 ↔ 图标）就没这个便宜可占，见 `row_content` 末尾。
            div()
                .id(("playlist-panel-play", index))
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                // 外层 `overflow_hidden` 只按矩形裁剪，圆角得自己带上，否则遮罩会露出直角。
                .rounded(COVER_RADIUS)
                .bg(colors.foreground.alpha(0.3))
                .cursor_pointer()
                .role(Role::Button)
                .aria_label(if show_pause { "暂停" } else { "播放" })
                // 不让整行双击监听记录这次按下，否则按住时移动会清掉行悬停。
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(on_cover_click)
                .opacity(if is_current { 1. } else { 0. })
                .when(!is_current, |overlay| {
                    overlay.group_hover(group, |style| style.opacity(1.))
                })
                .child(
                    svg()
                        .path(if show_pause {
                            "icons/pause.svg"
                        } else {
                            "icons/play.svg"
                        })
                        .size(PLAY_BUTTON_SIZE)
                        .flex_none()
                        .text_color(colors.primary_foreground.alpha(0.9)),
                ),
        )
        .into_any_element();

    let text = div()
        .flex_1()
        .min_w(px(0.))
        .child(
            div()
                .truncate()
                .text_size(px(16.))
                .text_color(if is_current {
                    colors.primary
                } else {
                    colors.foreground
                })
                .child(song.name.clone()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .min_w(px(0.))
                // 音质徽章排在作者前面，尺寸与歌单行一致。
                .when_some(song.best_quality_level(), |row, level| {
                    row.child(img(quality_badge_path(level)).h(px(13.)).flex_none())
                })
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(if is_current {
                            colors.primary
                        } else {
                            colors.muted_foreground
                        })
                        .child(artist_label(song, colors)),
                ),
        )
        .into_any_element();

    // 时长与右侧那排图标互斥，悬停整行时才挂上图标。
    // 这里刻意不用 `group_hover` 切 `display`：`display: none` 会剪掉整棵子树，
    // 而组悬停状态要到 paint 阶段才登记进 `GroupHitboxes`——prepaint 与 paint
    // 一旦判断不一致，就会踩到 "must call prepaint before paint"。
    if hovered {
        vec![cover, text, row_actions(index, liked, colors, on_like_click)]
    } else {
        // 时长是辅助信息：比 muted_foreground 再淡一档，和歌单行一致。
        let duration = div()
            .flex_none()
            .font_weight(FontWeight::NORMAL)
            .text_color(colors.foreground.alpha(0.45))
            .child(format_duration(song.duration()))
            .into_any_element();
        vec![cover, text, duration]
    }
}

/// 悬停整行时替换时长的三个图标：爱心 / 收藏 / 更多。
///
/// 尺寸和间距对齐歌单页那排操作图标（`IconSize::Small`，图标之间 10px）。
fn row_actions(
    index: usize,
    liked: bool,
    colors: ColorTokens,
    on_like_click: RowCallback<ClickEvent>,
) -> AnyElement {
    // 返回 `Stateful<Svg>`，收藏 / 更多各自再挂自己的点击处理。
    let plain = |id: (&'static str, usize), path: &'static str, label: &'static str| {
        svg()
            .path(path)
            .size(IconSize::Small.pixels())
            .flex_none()
            .text_color(colors.foreground.alpha(0.6))
            .hover(|style| style.text_color(colors.foreground))
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // active 属于 StatefulInteractiveElement，必须跟在 `.id()` 之后。
            .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
    };

    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.))
        // 已喜欢是实心红心，未喜欢是描线灰心，和歌单页的红心同一套规则。
        .child(
            svg()
                .path(like_icon_path(liked))
                .size(IconSize::Small.pixels())
                .flex_none()
                .text_color(if liked {
                    colors.primary
                } else {
                    colors.foreground.alpha(0.6)
                })
                .hover(move |style| {
                    if liked {
                        style.text_color(colors.primary.alpha(0.88))
                    } else {
                        style.text_color(colors.foreground)
                    }
                })
                .id(("playlist-panel-like", index))
                .role(Role::Button)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(on_like_click)
                .aria_label(if liked { "已喜欢" } else { "未喜欢" }),
        )
        // 收藏：弹「收藏到歌单」弹窗。先停传播，避免双击时落到整行的播放处理上。
        .child(
            plain(
                ("playlist-panel-collect", index),
                "icons/collect.svg",
                "收藏",
            )
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                crate::ui::components::open_collect_window(window, cx);
            }),
        )
        // 更多：弹出原生菜单。
        .child(
            plain(("playlist-panel-more", index), "icons/xpoint.svg", "更多").on_click(
                |event, window, cx| {
                    cx.stop_propagation();
                    crate::ui::components::show_song_menu(event.position(), window, cx);
                },
            ),
        )
        .into_any_element()
}

/// 播放列表里的一行。
///
/// 没有用 gpui-kit 的 `ListItem`：它把行内边距和 hover 底色写死在内部，而这里的
/// 行高、留白、hover 底色都要和音质浮窗对上。自己实现只需要满足 `Selectable`。
#[derive(IntoElement)]
struct QueueRow {
    id: ElementId,
    /// 供封面遮罩做 `group_hover` 的分组名。
    group: SharedString,
    hover_color: Hsla,
    /// 末行的 hover 底色正好压在面板左下圆角上，单独把那角磨圆，
    /// 免得盖出一个直角（`overflow_hidden` 只做矩形裁剪）。
    round_bottom_left: bool,
    on_double_click: Option<RowCallback<ClickEvent>>,
    /// 进出本行时回调。悬停状态由委托保存，这里只负责把事件送回去。
    on_hover: Option<RowCallback<bool>>,
    children: Vec<AnyElement>,
}

impl Selectable for QueueRow {
    fn selected(self, _: bool) -> Self {
        self
    }

    fn is_selected(&self) -> bool {
        false
    }
}

impl RenderOnce for QueueRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let QueueRow {
            id,
            group,
            hover_color,
            round_bottom_left,
            on_double_click,
            on_hover,
            children,
        } = self;
        div()
            .id(id)
            .group(group)
            .w_full()
            .h(ROW_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(ROW_PADDING)
            // 作者、时长这些次要文字按歌单行的字号继承。
            .text_size(px(13.))
            // hover 只把底色压深一层，和音质浮窗的选项行同一效果。
            .hover(|style| style.bg(hover_color))
            .when(round_bottom_left, |row| row.rounded_bl(SURFACE_RADIUS))
            .map(|row| match on_double_click {
                Some(handler) => row.on_double_click(handler),
                None => row,
            })
            .map(|row| match on_hover {
                Some(handler) => row.on_hover(handler),
                None => row,
            })
            .children(children)
    }
}
