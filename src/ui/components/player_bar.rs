mod quality;

use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme, Transition, transition};

use super::progress_bar::ProgressBar;
use super::side_panel::{SidePanel, SidePanelContent};
use super::volume_control::VolumeControl;
use super::{LAYER_PROGRESS_BAR, Vinyl, artist_label, icon_hover_color, like_icon_path};
use crate::models::PlayMode;
use crate::playback::{PlaybackController, PlaybackEvent};
use crate::state::library::MusicLibrary;
use crate::state::song_counts::SongCounts;
use crate::ui::cover_color::{blend_color, blend_colors, dark_colors};
use crate::ui::events::{Navigate, NavigationBus};
use crate::ui::now_playing::NowPlaying;
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA, PRESSED_OPACITY};

/// 播放栏固定高度；专辑歌词页据此估算可视区域高度。
pub const PLAYER_BAR_HEIGHT: f32 = 86.;

pub struct PlayerBar {
    playback: Entity<PlaybackController>,
    library: Entity<MusicLibrary>,
    song_counts: Entity<SongCounts>,
    progress_bar: Entity<ProgressBar>,
    volume: Entity<VolumeControl>,
    play_button_hovered: bool,
    play_button_pressed: bool,
    /// 图标与计数的独立命中区共用颜色，键为（互动区 ID，部件）。
    hovered_interaction: Option<(&'static str, usize)>,
    mini_vinyl: Entity<Vinyl>,
    quality_open: bool,
    now_playing: Entity<NowPlaying>,
    side_panel: Entity<SidePanel>,
    _playback_subscription: Subscription,
    _counts_subscription: Subscription,
    _progress_subscription: Subscription,
    _library_subscription: Subscription,
    _now_playing_subscription: Subscription,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InteractionClick {
    Like(bool),
    Comments,
}

impl PlayerBar {
    pub fn new(
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        song_counts: Entity<SongCounts>,
        now_playing: Entity<NowPlaying>,
        side_panel: Entity<SidePanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let progress_bar =
            cx.new(|cx| ProgressBar::new(playback.clone(), now_playing.clone(), window, cx));
        let volume =
            cx.new(|cx| VolumeControl::new(playback.clone(), now_playing.clone(), window, cx));
        let mini_vinyl = cx.new(|cx| {
            Vinyl::new(
                playback.clone(),
                now_playing.read(cx).rotation_clock(),
                "images/miniVinyl.png",
                cx,
            )
        });
        // 进度推进由进度条处理，不重绘整条播放栏。
        let playback_subscription = cx.subscribe(&playback, move |_, _, event, cx| {
            if matches!(event, PlaybackEvent::SnapshotChanged) {
                cx.notify();
            }
        });
        let counts_subscription = cx.observe(&song_counts, |_, _, cx| cx.notify());
        let mut progress_expanded = progress_bar.read(cx).expanded();
        let progress_subscription = cx.observe(&progress_bar, move |_, progress, cx| {
            let expanded = progress.read(cx).expanded();
            if expanded != progress_expanded {
                progress_expanded = expanded;
                cx.notify();
            }
        });
        let library_subscription = cx.observe(&library, |_, _, cx| cx.notify());
        let now_playing_subscription = cx.subscribe(&now_playing, |_, _, _, cx| cx.notify());
        Self {
            playback,
            library,
            song_counts,
            progress_bar,
            volume,
            play_button_hovered: false,
            play_button_pressed: false,
            hovered_interaction: None,
            mini_vinyl,
            quality_open: false,
            now_playing,
            side_panel,
            _playback_subscription: playback_subscription,
            _counts_subscription: counts_subscription,
            _progress_subscription: progress_subscription,
            _library_subscription: library_subscription,
            _now_playing_subscription: now_playing_subscription,
        }
    }

    pub(in crate::ui) fn vinyl_overlay(&self) -> AnyElement {
        self.mini_vinyl.clone().into_any_element()
    }

    /// 离开事件只清除自己，避免乱序回调清掉相邻命中区的 hover。
    fn set_interaction_hover(
        &mut self,
        key: (&'static str, usize),
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if hovered {
            Some(key)
        } else if self.hovered_interaction == Some(key) {
            None
        } else {
            self.hovered_interaction
        };
        if next != self.hovered_interaction {
            self.hovered_interaction = next;
            cx.notify();
        }
    }

    fn interaction_count(
        &self,
        click: InteractionClick,
        count: Option<u64>,
        colors: ColorTokens,
        primary: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (id, path, liked) = match click {
            InteractionClick::Like(liked) => ("player-like-button", like_icon_path(liked), liked),
            InteractionClick::Comments => ("player-comment-button", "icons/comment.svg", false),
        };
        let (color, hover_color, pressed_color) = if liked {
            let hover = primary.alpha(primary.a * 0.88);
            (primary, hover, hover)
        } else {
            (
                colors.muted_foreground,
                icon_hover_color(colors.muted_foreground, colors),
                colors.foreground,
            )
        };
        let hovered = self
            .hovered_interaction
            .is_some_and(|(interaction, _)| interaction == id);
        let shown_color = if hovered { hover_color } else { color };

        div()
            .relative()
            .ml_0p5()
            .w(px(28.))
            .h(px(24.))
            .flex_none()
            .id((id, INTERACTION_AREA))
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                this.set_interaction_hover((id, INTERACTION_AREA), *hovered, cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let Some(song) = this.playback.read(cx).snapshot().current_song.clone() else {
                    return;
                };
                match click {
                    InteractionClick::Like(_) => this
                        .library
                        .update(cx, |library, cx| library.toggle_like(song, cx)),
                    InteractionClick::Comments => {
                        NavigationBus::emit(cx, Navigate::SongComments(song));
                    }
                }
            }))
            .child(
                div().absolute().left_0().bottom_0().child(
                    svg()
                        .path(path)
                        .size(IconSize::Large.pixels())
                        .flex_none()
                        .text_color(shown_color)
                        .id((id, INTERACTION_ICON))
                        .active(move |style| {
                            style.text_color(pressed_color.alpha(PRESSED_ICON_ALPHA))
                        }),
                ),
            )
            .when_some(count, |block, count| {
                block.child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(16.))
                        .px(px(2.))
                        .rounded_full()
                        .bg(colors.surface)
                        .text_color(shown_color)
                        .text_size(px(9.))
                        .font_family(DOLPHIN_FAMILY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(px(10.))
                        // 计数自己也是命中区：它伸出格子的那部分不在区域 hitbox 里。
                        .id((id, INTERACTION_BADGE))
                        .on_hover(cx.listener(move |this, hovered, _, cx| {
                            this.set_interaction_hover((id, INTERACTION_BADGE), *hovered, cx);
                        }))
                        .child(format_count(count)),
                )
            })
            .into_any_element()
    }
}

/// 互动区的三个命中部件，用作 hitbox 和元素 id 的后缀。
const INTERACTION_AREA: usize = 0;
const INTERACTION_BADGE: usize = 1;
const INTERACTION_ICON: usize = 2;

fn format_count(count: u64) -> String {
    match count {
        0..1000 => count.to_string(),
        1000..10000 => "999+".into(),
        10000..100000 => "1w+".into(),
        _ => "10w+".into(),
    }
}

/// 无预设样式的图标按钮：Button 管交互，SVG 管颜色。
fn hover_icon(
    id: &'static str,
    label: &'static str,
    icon_path: &'static str,
    size: Pixels,
    color: Hsla,
    colors: ColorTokens,
) -> Button {
    Button::new(id)
        .aria_label(label)
        .size(size)
        .flex_none()
        .child(
            svg()
                .path(icon_path)
                .size(size)
                .flex_none()
                .text_color(color)
                .hover(move |style| style.text_color(icon_hover_color(color, colors)))
                .id((id, 0usize))
                .active(move |style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA))),
        )
}

impl Render for PlayerBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_colors = Theme::global(cx).tokens.colors;
        let (album_expanded, album_backdrop) = {
            let now_playing = self.now_playing.read(cx);
            (now_playing.is_expanded(), now_playing.backdrop())
        };
        // 展开/收起时整条栏的颜色跟着黑胶页的滑入一起过渡，而不是瞬间跳变。
        let expand = NowPlaying::reveal(album_expanded, window, cx);
        self.mini_vinyl.read(cx).clear();
        let backdrop_color = album_backdrop
            .gradient()
            .map(|gradient| Hsla::from(gradient.0[1]))
            .unwrap_or(hsla(0., 0., 0.12, 1.));
        let colors = blend_colors(
            theme_colors,
            dark_colors(theme_colors, backdrop_color),
            expand,
        );
        let progress_expanded = self.progress_bar.read(cx).expanded();
        let shadow_opacity = transition(
            "player-bar-shadow",
            if progress_expanded && !album_expanded {
                1.
            } else {
                0.
            },
            Transition::new(Duration::from_millis(130)).ease(ease_out_quint()),
            window,
            cx,
        );
        let play_button_enlarged = self.play_button_hovered && !self.play_button_pressed;
        let play_button_size = if play_button_enlarged {
            px(42.)
        } else {
            px(40.)
        };
        let play_pause_icon_size = if play_button_enlarged {
            px(25.44)
        } else {
            IconSize::Large.pixels()
        };
        let (title, artist, song_id, is_playing) = {
            let playback = self.playback.read(cx);
            playback
                .snapshot()
                .current_song
                .as_ref()
                .map(|song| {
                    (
                        song.name.clone(),
                        artist_label(song, "", colors),
                        Some(song.id),
                        playback.is_play_requested(),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        String::new(),
                        StyledText::new(""),
                        None,
                        playback.is_play_requested(),
                    )
                })
        };
        let liked =
            song_id.is_some_and(|song_id| self.library.read(cx).liked_song_ids.contains(&song_id));
        let counts = self.song_counts.read(cx).counts();
        let snapshot = self.playback.read(cx).snapshot();
        let mode = snapshot.mode;
        // 心动模式生效后模式本身就是那一档；还在加载时队列尚未换上，
        // 模式还是原来那一档，就先用心动图标提示正在准备。
        let loading_heart = snapshot.heart_loading && mode != PlayMode::Heart;
        let selected_quality = snapshot.quality;
        // 优先显示实际拿到的音质，降级时不会谎报。
        let quality_label = snapshot
            .actual_quality
            .unwrap_or(snapshot.quality)
            .quality_label();
        let play_pause_icon_path = if is_playing {
            "icons/pause.svg"
        } else {
            "icons/play.svg"
        };
        let play_pause_icon = svg()
            .path(play_pause_icon_path)
            .size(play_pause_icon_size)
            .text_color(colors.primary_foreground)
            .into_any_element();
        div()
            // 控件自行停止冒泡，空白区域的点击才会开合歌词页。
            .id("player-bar")
            .on_click(cx.listener(|this, _, _, cx| {
                this.now_playing
                    .update(cx, |now_playing, cx| now_playing.toggle(cx));
            }))
            .w_full()
            .h(px(PLAYER_BAR_HEIGHT))
            .flex_none()
            .flex()
            .flex_col()
            // 普通页面保留进度条悬停阴影；展开页不绘制。
            .when(!album_expanded, |bar| {
                bar.shadow(vec![
                    BoxShadow::new(
                        px(0.),
                        px(-12.),
                        colors.foreground.alpha(0.18 * shadow_opacity),
                    )
                    .blur_radius(px(24.))
                    .spread_radius(px(12.)),
                ])
            })
            .bg(colors.surface)
            .relative()
            .when(album_expanded, |bar| {
                let backdrop = album_backdrop.clone();
                let surface = theme_colors.surface;
                bar.child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            // 绘制时读取同一帧的渐变底端，再按展开进度从普通底色过渡过去。
                            let bottom = backdrop
                                .gradient()
                                .map(|gradient| Hsla::from(gradient.0[1]))
                                .unwrap_or_else(|| hsla(0., 0., 0.12, 1.));
                            window.paint_quad(fill(bounds, blend_color(surface, bottom, expand)));
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
            })
            // 进度条覆盖渲染
            .child(deferred(self.progress_bar.clone()).with_priority(LAYER_PROGRESS_BAR))
            // 主控件栏
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .px(px(30.))
                    // 左侧：封面、歌曲信息和互动数据
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            // 封面尺寸固定 60，动画只改它在布局里占的位置：收起时整块向左滑出
                            // 70px（自身 60 + 间隔 10），后边的歌名与互动区随之平滑左移；
                            // 滑出部分由左侧容器的 overflow_hidden 裁掉。
                            .child(
                                div()
                                    .id("player-album-cover")
                                    // 指针只留在封面和按钮上：整栏可点不代表整栏都长得像按钮。
                                    .cursor_pointer()
                                    .size(px(60.))
                                    .flex_none()
                                    .ml(px(-70. * expand))
                                    .opacity(1. - expand)
                                    .when(expand < 1., |cover| {
                                        cover.child(self.mini_vinyl.read(cx).placeholder(
                                            60.,
                                            1. - expand,
                                            None,
                                        ))
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .max_w(px(200.))
                                    .flex_shrink_1()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            // 命中区跟着栏走，这里不再需要为了点击区收窄宽度：
                                            // 整栏都是点击区，歌名被拉满一列也不会多拦住什么。
                                            // `self_start` 让宽度收缩到文字，`max_w_full` 把上限卡在
                                            // 本列宽度内，歌名过长时仍能省略号截断而不是溢出。
                                            .self_start()
                                            .max_w_full()
                                            .text_color(colors.foreground)
                                            .text_size(px(16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_color(colors.muted_foreground)
                                            .text_size(px(13.))
                                            .truncate()
                                            .child(artist),
                                    ),
                            )
                            // 已喜欢 hover 淡一档的红；未喜欢和评论按背景亮度轻微提亮。
                            .child(self.interaction_count(
                                InteractionClick::Like(liked),
                                counts[0],
                                colors,
                                theme_colors.primary,
                                cx,
                            ))
                            .child(self.interaction_count(
                                InteractionClick::Comments,
                                counts[1],
                                colors,
                                theme_colors.primary,
                                cx,
                            )),
                    )
                    // 中间：收藏与播放控制
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(20.))
                            .text_color(colors.foreground)
                            .child(
                                hover_icon(
                                    "player-order-button",
                                    if loading_heart {
                                        "正在获取心动推荐…"
                                    } else {
                                        mode.label()
                                    },
                                    if loading_heart {
                                        PlayMode::Heart.icon_path()
                                    } else {
                                        mode.icon_path()
                                    },
                                    IconSize::Large.pixels(),
                                    colors.muted_foreground,
                                    colors,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.playback
                                            .update(cx, |playback, cx| playback.cycle_mode(cx));
                                    },
                                )),
                            )
                            .child(
                                hover_icon(
                                    "player-previous-button",
                                    "上一首",
                                    "icons/pre.svg",
                                    IconSize::Large.pixels(),
                                    colors.secondary_foreground,
                                    colors,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.playback
                                            .update(cx, |playback, cx| playback.previous(cx));
                                    },
                                )),
                            )
                            .child(
                                div()
                                    .id("play-pause-hover-region")
                                    .size(px(40.))
                                    .flex_none()
                                    .relative()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.play_button_pressed = true;
                                            cx.notify();
                                        }),
                                    )
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.play_button_pressed = false;
                                            cx.notify();
                                        }),
                                    )
                                    .on_mouse_up_out(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            if this.play_button_pressed {
                                                this.play_button_pressed = false;
                                                cx.notify();
                                            }
                                        }),
                                    )
                                    .on_hover(cx.listener(|this, hovered, _, cx| {
                                        if this.play_button_hovered == *hovered {
                                            return;
                                        }
                                        this.play_button_hovered = *hovered;
                                        cx.notify();
                                    }))
                                    .child(
                                        Button::new("play-pause-button")
                                            .aria_label(if is_playing {
                                                "暂停"
                                            } else {
                                                "播放"
                                            })
                                            .absolute()
                                            .left(px(if play_button_enlarged { -1. } else { 0. }))
                                            .top(px(if play_button_enlarged { -1. } else { 0. }))
                                            .size(play_button_size)
                                            .rounded_full()
                                            .bg(colors.primary)
                                            .text_color(colors.primary_foreground)
                                            // 在原透明度上轻微变淡，避免把深色页的 12% 白色直接改成 88%。
                                            .hover(|style| {
                                                style.bg(colors
                                                    .primary
                                                    .alpha(colors.primary.a * 0.88))
                                            })
                                            .active(|style| style.opacity(PRESSED_OPACITY))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.playback.update(cx, |playback, cx| {
                                                    playback.toggle(cx);
                                                });
                                            }))
                                            .child(play_pause_icon),
                                    ),
                            )
                            .child(
                                hover_icon(
                                    "player-next-button",
                                    "下一首",
                                    "icons/next.svg",
                                    IconSize::Large.pixels(),
                                    colors.secondary_foreground,
                                    colors,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.playback.update(cx, |playback, cx| playback.next(cx));
                                    },
                                )),
                            )
                            .child(
                                hover_icon(
                                    "player-playlist-button",
                                    "播放列表",
                                    "icons/playlist.svg",
                                    IconSize::Large.pixels(),
                                    colors.muted_foreground,
                                    colors,
                                )
                                // 按下（capture 阶段，先于面板的 out 监听）时若面板开着，
                                // 拦住「面板外按下收起」，让松开后的 click 自己决定开合。
                                .capture_any_mouse_down(cx.listener(
                                    |this, event: &MouseDownEvent, _, cx| {
                                        if event.button == MouseButton::Left {
                                            this.side_panel
                                                .update(cx, |panel, _| panel.suppress_out_close());
                                        }
                                    },
                                ))
                                // 松开时才开合，和其它浮层触发一致；点击不冒泡到
                                // 播放栏根部的歌词页开合。
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.side_panel.update(cx, |panel, cx| {
                                            panel.toggle(SidePanelContent::PlayQueue, cx)
                                        });
                                    },
                                )),
                            ),
                    )
                    // 右侧：收藏、音量等工具
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(18.))
                            .text_color(colors.muted_foreground)
                            .child(self.quality_popover(
                                selected_quality,
                                quality_label,
                                colors,
                                cx,
                            ))
                            .child(
                                hover_icon(
                                    "player-collect-button",
                                    "收藏",
                                    "icons/collect.svg",
                                    IconSize::Middle.pixels(),
                                    colors.muted_foreground,
                                    colors,
                                )
                                // 右侧第二个图标就是收藏入口：弹出「收藏到歌单」弹窗。
                                // 收藏的是当前这首，所以要从播放快照里取——弹窗不认识播放器。
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        cx.stop_propagation();
                                        let Some(song) =
                                            this.playback.read(cx).snapshot().current_song.clone()
                                        else {
                                            return;
                                        };
                                        super::open_collect_window(window, vec![song], cx);
                                    },
                                )),
                            )
                            .child(
                                // 音量是独立控件（还带自己的 hover 气泡），整栏可点之后必须由
                                // 这里吞掉按下，否则点音量图标会顺带收起歌词页。
                                div()
                                    .flex_none()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .child(self.volume.clone()),
                            )
                            .child(hover_icon(
                                "player-more-button",
                                "更多",
                                "icons/xpoint.svg",
                                IconSize::Middle.pixels(),
                                colors.muted_foreground,
                                colors,
                            )),
                    )
                    // 侧面板自己绝对定位并延后绘制，不参与这一栏的布局。
                    .child(self.side_panel.clone()),
            )
    }
}
