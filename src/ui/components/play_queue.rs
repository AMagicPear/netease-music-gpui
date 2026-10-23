//! 播放队列内容（界面文案沿用它自己的「播放列表」）：挂在全局 [`super::side_panel::SidePanel`]
//! 里的一页内容，外壳的定位、动画、开合都在那边。
//!
//! 别和 [`crate::ui::pages::playlist`] 的"歌单"混了——那个是用户/编辑歌单的详情页，
//! 这个是当前会话的**播放队列**，所以代码里叫 `PlayQueue`。
//!
//! 列表交给 gpui-kit 的 [`List`]：队列可能上千首，而滑入/滑出的每一帧都会重绘
//! 整个面板，必须只渲染可见的十几行。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{
    Button, ColorTokens, IndexPath, InteractiveElementExt as _, Selectable, Theme,
};
use gpui_kit::component::list::{List, ListDelegate, ListState};

use super::{
    SURFACE_RADIUS, artist_label, format_duration, like_icon_path, quality_badge_path,
    side_panel::panel_height,
};
use crate::models::{PlayMode, Song};
use crate::playback::{PlaybackController, PlaybackEvent};
use crate::state::library::MusicLibrary;
use crate::ui::assets::{track_cover_image, track_cover_url};
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA};

/// 面板自己的头部高度；「播放列表」和计数排在这一行里。
pub(in crate::ui) const PANEL_HEADER_HEIGHT: Pixels = px(58.);
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

/// `group_hover` 按名字全局查分组，同名会互相点亮，所以每行要一个不同的组名。
fn row_group(index: usize) -> SharedString {
    SharedString::from(format!("playlist-panel-row-{index}"))
}

/// 头部右侧的文字按钮（「收藏全部」「清空」「一键全读」共用）：
/// 灰字，悬停加深，字号和计数一致。图标在左、文字在右。
pub(in crate::ui) fn header_text_button(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    colors: ColorTokens,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    // SVG 不继承父级的 text_color，颜色必须直接设在 svg 上；想跟文字同步变色
    // 就得借组悬停：组名用按钮 id 派生，`group_hover` 按名字全局查，同名会互相点亮。
    let group = SharedString::from(format!("playlist-header-{id}"));
    div()
        .id(id)
        .group(group.clone())
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(colors.muted_foreground)
        .hover(|style| style.text_color(colors.foreground))
        .role(Role::Button)
        .aria_label(label)
        .on_click(on_click)
        .child(
            svg()
                .path(icon)
                .size(px(14.))
                .flex_none()
                .text_color(colors.muted_foreground)
                .group_hover(group, move |style| style.text_color(colors.foreground)),
        )
        .child(label)
}

pub struct PlayQueue {
    playback: Entity<PlaybackController>,
    list: Entity<ListState<QueueDelegate>>,
    /// 面板当前是否展示本内容（由 [`super::side_panel::SidePanel`] 维护）。
    /// 订阅刷新只在可见时做；不可见时列表根本不渲染。
    visible: bool,
    /// 下次渲染时把列表滚到当前歌（面板展开/换内容进来时置位）。
    scroll_to_current: bool,
    _playback_subscription: Subscription,
    _library_subscription: Subscription,
}

impl PlayQueue {
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

        // 队列内容、游标、当前歌曲、播放态变化都走 SnapshotChanged；
        // 100ms 的位置推进是另一个变体，面板不订阅，不会白跑十几次。
        let _playback_subscription = cx.subscribe(&playback, move |this, _, event, cx| {
            if matches!(event, PlaybackEvent::SnapshotChanged) && this.visible {
                this.list.update(cx, |_, cx| cx.notify());
                cx.notify();
            }
        });

        // 喜欢状态由音乐库维护：别处（歌单页、播放栏红心）改动后，可见行的爱心要跟着变。
        // 不可见时不渲染，等展开时 `set_visible` 会统一刷新一次。
        let _library_subscription = cx.observe(&library, |this, _, cx| {
            if this.visible {
                this.list.update(cx, |_, cx| cx.notify());
                cx.notify();
            }
        });

        Self {
            playback,
            list,
            visible: false,
            scroll_to_current: false,
            _playback_subscription,
            _library_subscription,
        }
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            self.scroll_to_current = true;
            // 不可见期间队列可能已经换了，展开前先让列表重新量一遍；
            // 悬停行也一并清掉，免得展开时顶着一行过期的操作图标。
            self.list.update(cx, |list, cx| {
                list.delegate_mut().hovered = None;
                cx.notify();
            });
        }
        cx.notify();
    }
}

impl Render for PlayQueue {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let count = self.playback.read(cx).queue().len();
        let state = self.playback.read(cx).snapshot();
        let heart = state.mode == PlayMode::Heart || state.heart_loading;
        let heart_loading = state.heart_loading;
        let heart_error = state.heart_error.clone();
        if self.scroll_to_current {
            self.scroll_to_current = false;
            let index = self.playback.read(cx).queue_cursor().unwrap_or(0);
            self.list.update(cx, |list, cx| {
                list.scroll_to_item(IndexPath::new(index), ScrollStrategy::Top, window, cx);
            });
        }

        // 内容填满 side_panel 的外壳；这里只管排版，不管定位与动画。
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(PANEL_HEADER_HEIGHT.min(panel_height(window)))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(ROW_PADDING)
                    .pr(ROW_PADDING)
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
                    )
                    // 右侧操作区：收藏全部弹「收藏到歌单」窗口，清空移除整个队列。
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(12.))
                            .child(header_text_button(
                                "player-playlist-collect-all",
                                "icons/add_all.svg",
                                "收藏全部",
                                colors,
                                {
                                    // 「收藏全部」收的是整个队列，所以把队列整份交给弹窗；
                                    // 弹窗自己不知道播放器，这里给它快照就行。
                                    let playback = self.playback.clone();
                                    move |_, window, cx| {
                                        let songs = playback.read(cx).queue().to_vec();
                                        crate::ui::components::open_collect_window(
                                            window, songs, cx,
                                        );
                                    }
                                },
                            ))
                            .child(header_text_button(
                                "player-playlist-clear",
                                "icons/delete.svg",
                                "清空",
                                colors,
                                cx.listener(|this, _, _, cx| {
                                    this.playback
                                        .update(cx, |playback, cx| playback.clear_queue(cx));
                                }),
                            )),
                    ),
            )
            .when(heart || heart_error.is_some(), |panel| {
                panel.child(
                    div()
                        .px(ROW_PADDING)
                        .pb(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(12.))
                                .text_color(colors.muted_foreground)
                                .child(heart_error.clone().unwrap_or_else(|| {
                                    if heart_loading {
                                        "正在获取心动推荐…".into()
                                    } else {
                                        "心动模式 · 红心歌曲与相似推荐".into()
                                    }
                                })),
                        )
                        .when(heart_error.is_some(), |row| {
                            row.child(
                                Button::new("heart-retry")
                                    .aria_label("重试心动推荐")
                                    .child("重试")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.playback.update(cx, |playback, cx| {
                                            if playback.snapshot().mode == PlayMode::Heart {
                                                playback.retry_heart_mode(cx);
                                            } else {
                                                playback.set_mode(PlayMode::Heart, cx);
                                            }
                                        });
                                    })),
                            )
                        })
                        .when(heart, |row| {
                            row.child(
                                Button::new("heart-exit")
                                    .aria_label("退出心动模式")
                                    .child("退出")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.playback.update(cx, |playback, cx| {
                                            playback.exit_heart_mode(cx)
                                        });
                                    })),
                            )
                        }),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    // 滚动与滚动条都由 List 自己管。
                    .child(List::new(&self.list)),
            )
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

        // 行上右键：和行内「更多」图标弹同一份原生菜单，只是换个触发方式。
        // 位置同样取鼠标点下的窗口坐标，所以两处菜单落点行为一致。
        let playback_for_menu = self.playback.clone();
        let on_right_click: RowCallback<ClickEvent> = Box::new(cx.listener(
            move |_: &mut ListState<QueueDelegate>,
                  event: &ClickEvent,
                  window: &mut Window,
                  cx: &mut Context<ListState<QueueDelegate>>| {
                cx.stop_propagation();
                let Some(song) = playback_for_menu.read(cx).queue().get(index).cloned() else {
                    return;
                };
                crate::ui::components::show_song_menu(event.position(), song, window, cx);
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
            on_double_click,
            on_right_click,
            on_hover,
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
                        .child(artist_label(song, "", colors)),
                ),
        )
        .into_any_element();

    // 时长与右侧那排图标互斥，悬停整行时才挂上图标。
    // 这里刻意不用 `group_hover` 切 `display`：`display: none` 会剪掉整棵子树，
    // 而组悬停状态要到 paint 阶段才登记进 `GroupHitboxes`——prepaint 与 paint
    // 一旦判断不一致，就会踩到 "must call prepaint before paint"。
    if hovered {
        vec![
            cover,
            text,
            row_actions(index, song, liked, colors, on_like_click),
        ]
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
    song: &Song,
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
            .on_click({
                let song = song.clone();
                move |_, window, cx| {
                    cx.stop_propagation();
                    crate::ui::components::open_collect_window(window, vec![song.clone()], cx);
                }
            }),
        )
        // 更多：弹出原生菜单。
        .child(
            plain(("playlist-panel-more", index), "icons/xpoint.svg", "更多").on_click({
                let song = song.clone();
                move |event, window, cx| {
                    cx.stop_propagation();
                    crate::ui::components::show_song_menu(
                        event.position(),
                        song.clone(),
                        window,
                        cx,
                    );
                }
            }),
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
    on_double_click: RowCallback<ClickEvent>,
    /// 行上右键（非左键的 click）。挂了它就弹「更多」那份菜单，效果和行内图标一致。
    on_right_click: RowCallback<ClickEvent>,
    /// 进出本行时回调。悬停状态由委托保存，这里只负责把事件送回去。
    on_hover: RowCallback<bool>,
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
            on_right_click,
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
            .on_double_click(on_double_click)
            .on_aux_click(on_right_click)
            .on_hover(on_hover)
            .children(children)
    }
}
