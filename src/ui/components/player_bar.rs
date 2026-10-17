use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme, Transition, transition};

use super::progress_bar::ProgressBar;
use super::side_panel::{SidePanel, SidePanelContent};
use super::volume_control::VolumeControl;
use super::{
    LAYER_PROGRESS_BAR, SurfacePopover, Vinyl, artist_label, icon_hover_color, like_icon_path,
};
use crate::api::MusicApi;
use crate::models::AudioQualityLevel;
use crate::playback::PlaybackController;
use crate::state::library::MusicLibrary;
use crate::ui::cover_color::{blend_color, blend_colors, dark_colors};
use crate::ui::now_playing::NowPlaying;
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA, PRESSED_OPACITY};

/// 播放栏固定高度；专辑歌词页据此估算可视区域高度。
pub const PLAYER_BAR_HEIGHT: f32 = 86.;

pub struct PlayerBar {
    playback: Entity<PlaybackController>,
    /// 喜欢状态不在播放控制器里，而在音乐库中，红心要据此显示实心/空心。
    library: Entity<MusicLibrary>,
    progress_bar: Entity<ProgressBar>,
    /// 音量是 hover 弹出、形状还带小三角，和通用弹层不一样，自己实现。
    volume: Entity<VolumeControl>,
    play_button_hovered: bool,
    play_button_pressed: bool,
    song_id: Option<u64>,
    counts: [Option<u64>; 2],
    /// 哪块互动区正被悬停，`(互动区 id, 命中部件)`。
    /// 图标和计数是两个独立 hitbox（计数还可能伸出格子），靠这份状态让两者同步变色。
    hovered_interaction: Option<(&'static str, usize)>,
    mini_vinyl: Entity<Vinyl>,
    /// 音质弹层的开合。位置、延迟绘制和窗口边缘避让交给 `base::Popup`，
    /// 但开合状态、点外面关闭和进出场动画得自己管：gpui-kit 的 Popover 一关
    /// 就在同一帧卸载内容，没有东西留给退场动画。
    quality_open: bool,
    /// 打开弹层前的焦点，关闭时还回去。
    /// 「整页展开/收起」的单一状态源。展开状态、共享渐变底、动效节拍都从这里读，
    /// 播放栏不再自己存 `album_expanded` 镜像。
    now_playing: Entity<NowPlaying>,
    /// 全局侧面板（内含播放队列/消息两页内容）。由组合根创建后传入；
    /// 播放栏只负责挂载和触发播放队列页。
    side_panel: Entity<SidePanel>,
    _playback_subscription: Subscription,
    _progress_subscription: Subscription,
    _library_subscription: Subscription,
    _now_playing_subscription: Subscription,
}

impl PlayerBar {
    pub fn new(
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        now_playing: Entity<NowPlaying>,
        side_panel: Entity<SidePanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let progress_bar =
            cx.new(|cx| ProgressBar::new(playback.clone(), now_playing.clone(), window, cx));
        let volume =
            cx.new(|cx| VolumeControl::new(playback.clone(), now_playing.clone(), window, cx));
        // 旋转时钟由 NowPlaying 持有，和整页大碟共用同一份。
        let mini_vinyl = cx.new(|cx| {
            Vinyl::new(
                playback.clone(),
                now_playing.read(cx).rotation_clock(),
                "images/miniVinyl.png",
                cx,
            )
        });
        let mut playback_state = {
            let controller = playback.read(cx);
            let state = controller.snapshot();
            (
                state.revision,
                controller.is_play_requested(),
                state.mode,
                state.quality,
                state.actual_quality,
            )
        };
        let playback_subscription = cx.observe(&playback, move |this, playback, cx| {
            let controller = playback.read(cx);
            let state = controller.snapshot();
            let next = (
                state.revision,
                controller.is_play_requested(),
                state.mode,
                state.quality,
                state.actual_quality,
            );
            if next != playback_state {
                playback_state = next;
                this.load_counts(cx);
                cx.notify();
            }
        });
        let mut progress_expanded = progress_bar.read(cx).expanded();
        let progress_subscription = cx.observe(&progress_bar, move |_, progress, cx| {
            let expanded = progress.read(cx).expanded();
            if expanded != progress_expanded {
                progress_expanded = expanded;
                cx.notify();
            }
        });
        // 喜欢状态由音乐库维护：歌单页点亮红心后，播放栏也要跟着重绘。
        let library_subscription = cx.observe(&library, |_, _, cx| cx.notify());
        // 展开状态与共享渐变底都由 NowPlaying 广播；播放栏只负责跟着重绘。
        let now_playing_subscription = cx.observe(&now_playing, |_, _, cx| cx.notify());
        let mut this = Self {
            playback,
            library,
            progress_bar,
            volume,
            play_button_hovered: false,
            play_button_pressed: false,
            song_id: None,
            counts: [None; 2],
            hovered_interaction: None,
            mini_vinyl,
            quality_open: false,
            now_playing,
            side_panel,
            _playback_subscription: playback_subscription,
            _progress_subscription: progress_subscription,
            _library_subscription: library_subscription,
            _now_playing_subscription: now_playing_subscription,
        };
        this.load_counts(cx);
        this
    }

    pub(in crate::ui) fn vinyl_overlay(&self) -> AnyElement {
        self.mini_vinyl.clone().into_any_element()
    }

    fn load_counts(&mut self, cx: &mut Context<Self>) {
        let song_id = self
            .playback
            .read(cx)
            .snapshot()
            .current_song
            .as_ref()
            .map(|song| song.id);
        if self.song_id == song_id {
            return;
        }
        self.song_id = song_id;
        self.counts = [None; 2];
        let Some(song_id) = song_id else {
            return;
        };
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::song_counts(api.client.clone(), song_id));
        cx.spawn(async move |this, cx| {
            match request.await {
                Ok(counts) => {
                    let _ = this.update(cx, |this, cx| {
                        // 切歌后的旧请求不能覆盖当前歌曲的计数。
                        if this.song_id != Some(song_id) {
                            return;
                        }
                        for (index, count) in counts.into_iter().enumerate() {
                            match count {
                                Ok(count) => this.counts[index] = Some(count),
                                Err(message) => eprintln!("{message}"),
                            }
                        }
                        cx.notify();
                    });
                }
                Err(_) => eprintln!("歌曲互动计数任务失败"),
            }
        })
        .detach();
    }

    /// 离开事件只清除自己：图标和计数是两块相邻的 hitbox，鼠标从图标移到数字上时，
    /// 「离开图标」和「进入数字」谁先到并不确定；一律清除会让后到的进入事件失效，
    /// 于是颜色闪一下就没了。只清除与当前键相同的状态就没有这个问题。
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

    /// 左下角互动图标 + 右上角计数，两者同色。
    ///
    /// 图标和计数是两块独立的 hitbox，而且计数常常比 28px 的格子宽、会伸出格子，
    /// 没法用一个 `group` 的形状同时罩住它们。所以改用 view 里的悬停状态把两者绑在一起：
    /// 谁被悬停都算这块互动区被悬停，两边都按同一份状态取色。
    ///
    /// 只有红心接了动作，它会在点击处理里停传播；评论还没接面板，那一下会照常冒到
    /// 栏根，跟着整栏一起展开 / 收起。
    fn interaction_count(
        &self,
        id: &'static str,
        path: &'static str,
        count: Option<u64>,
        // 红心按钮传 true：点击时自己去取当时正在播放的那首歌，避免每帧克隆整首。
        like_button: bool,
        color: Hsla,
        hover_color: Hsla,
        pressed_color: Hsla,
        colors: ColorTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            .when(like_button, |element| {
                element.on_click(cx.listener(|this, _, _, cx| {
                    // 整条栏现在是一个"展开/收起"的点击区（见 render 里栏根的 on_click），
                    // 自带行为的部件都要先停传播，否则点红心会顺手把歌词页也收起。
                    cx.stop_propagation();
                    let Some(song) = this.playback.read(cx).snapshot().current_song.clone() else {
                        return;
                    };
                    this.library
                        .update(cx, |library, cx| library.toggle_like(song, cx));
                }))
            })
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

    fn quality_popover(
        &self,
        selected_quality: AudioQualityLevel,
        quality_label: &'static str,
        colors: ColorTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 触发按钮：点自己开/关。守卫比较的是**本次渲染时**的开合值——
        // "点外面"的捕获监听可能已经先把状态关掉了，那时不能再翻回去。
        // 末尾的 `stop_propagation` 还兼一件事：按下不冒到栏根，
        // 点音质按钮不会顺手把歌词页收起来。
        let was_open = self.quality_open;
        let trigger = Button::new("player-quality-button")
            .aria_label("音质选项")
            .h(px(16.))
            .px(px(2.))
            .flex_none()
            .border_1()
            .border_color(colors.muted_foreground)
            .rounded(px(4.))
            .text_size(px(10.))
            .text_color(colors.muted_foreground)
            .hover(|style| {
                let hover = icon_hover_color(colors.muted_foreground, colors);
                style.text_color(hover).border_color(hover)
            })
            .child(quality_label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.quality_open == was_open {
                        this.set_quality_popover(!was_open, cx);
                    }
                    cx.stop_propagation();
                }),
            );

        // 歌词页上播放栏会过渡到暗色板：弹层换一套专为暗底设计的近黑面板，
        // 靠亮度差 + 白系描边与背景分层（墨色阴影在深背景上不可见）；
        // 浅色页面维持白面板 + 墨色阴影。两种面板共用同一份列表结构。
        let dark = colors.background.l < 0.5;
        let title_color = if dark {
            white().alpha(0.92)
        } else {
            colors.secondary_foreground
        };
        let detail_color = if dark {
            white().alpha(0.45)
        } else {
            colors.muted_foreground.alpha(0.4)
        };
        let hover_bg = if dark {
            white().alpha(0.07)
        } else {
            colors.muted
        };

        // 品质标记：环绕/母带用金色线性图标，VIP 等级红系，普通档灰系。
        #[derive(Clone, Copy)]
        enum QualityMark {
            Gold(&'static str),
            Red(&'static str),
            Neutral(&'static str),
        }
        let mark = move |kind: QualityMark| -> Div {
            let (bg, fg) = match kind {
                QualityMark::Gold(_) => (
                    if dark {
                        Hsla::from(rgb(0xe8bd77)).alpha(0.16)
                    } else {
                        Hsla::from(rgb(0xf7edd6))
                    },
                    if dark {
                        Hsla::from(rgb(0xe8bd77))
                    } else {
                        Hsla::from(rgb(0xd9a648))
                    },
                ),
                QualityMark::Red(_) => (
                    if dark {
                        Hsla::from(rgb(0xc97f71)).alpha(0.18)
                    } else {
                        Hsla::from(rgb(0xfdf0ed))
                    },
                    if dark {
                        Hsla::from(rgb(0xe59d93))
                    } else {
                        Hsla::from(rgb(0xc97f71))
                    },
                ),
                QualityMark::Neutral(_) => (
                    if dark {
                        white().alpha(0.1)
                    } else {
                        colors.muted
                    },
                    if dark {
                        white().alpha(0.6)
                    } else {
                        colors.muted_foreground
                    },
                ),
            };
            let inner: AnyElement = match kind {
                QualityMark::Gold(icon) => svg()
                    .path(icon)
                    .size(px(18.))
                    .text_color(fg)
                    .into_any_element(),
                QualityMark::Red(text) | QualityMark::Neutral(text) => {
                    div().text_size(px(11.)).child(text).into_any_element()
                }
            };
            div()
                .size(px(32.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(bg)
                .text_color(fg)
                .child(inner)
        };

        let entries = [
            (
                AudioQualityLevel::Sky,
                "沉浸环绕声",
                "环绕音感 最高5.1声道",
                QualityMark::Gold("icons/音质选项/immersive_audio.svg"),
                "icons/VIP/svip.svg",
            ),
            (
                AudioQualityLevel::JyMaster,
                "超清母带",
                "极致细节 192kHz/24bit",
                QualityMark::Gold("icons/音质选项/master_audio.svg"),
                "icons/VIP/svip.svg",
            ),
            (
                AudioQualityLevel::JyEffect,
                "高清臻音 (Spatial Audio)",
                "高频细节还原与清晰沉浸感，96kHz/24bit",
                QualityMark::Red("臻"),
                "icons/VIP/vip.svg",
            ),
            (
                AudioQualityLevel::HiRes,
                "高解析度无损 (Hi-Res)",
                "更饱满清晰的高解析度音质，最高192kHz/24bit",
                QualityMark::Red("H"),
                "icons/VIP/vip.svg",
            ),
            (
                AudioQualityLevel::Lossless,
                "无损 (SQ)",
                "高保真无损音质，最高48kHz/16bit",
                QualityMark::Red("SQ"),
                "icons/VIP/vip.svg",
            ),
            (
                AudioQualityLevel::ExHigh,
                "极高 (HQ)",
                "近 CD 音质的细节体验，最高 320kbps",
                QualityMark::Neutral("HQ"),
                "",
            ),
            (
                AudioQualityLevel::Standard,
                "标准",
                "128kbps",
                QualityMark::Neutral("标"),
                "",
            ),
        ];

        SurfacePopover::new("player-quality-popover", was_open, trigger)
            .anchor(Anchor::BottomCenter)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.set_quality_popover(false, cx);
            }))
            .w(px(340.))
            .flex()
            .flex_col()
            .pb(px(8.))
            .when(dark, |panel| {
                panel
                    .bg(hsla(0., 0., 0.12, 1.))
                    .border_1()
                    .border_color(white().alpha(0.08))
            })
            .text_color(title_color)
            .child(
                div()
                    .px(px(16.))
                    .pt(px(12.))
                    .pb(px(8.))
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("当前歌曲音质"),
            )
            .children(
                entries
                    .into_iter()
                    .map(|(quality, title, detail, mark_kind, badge)| {
                        let selected = selected_quality == quality;
                        div()
                            .id((
                                match quality {
                                    AudioQualityLevel::Sky => "quality-sky",
                                    AudioQualityLevel::JyMaster => "quality-master",
                                    AudioQualityLevel::JyEffect => "quality-jyeffect",
                                    AudioQualityLevel::HiRes => "quality-hires",
                                    AudioQualityLevel::Lossless => "quality-lossless",
                                    AudioQualityLevel::ExHigh => "quality-exhigh",
                                    _ => "quality-standard",
                                },
                                0usize,
                            ))
                            .h(px(52.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .mx(px(8.))
                            .px(px(6.))
                            .rounded(px(8.))
                            .hover(|style| style.bg(hover_bg))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.playback
                                    .update(cx, |playback, cx| playback.set_quality(quality, cx));
                            }))
                            .child(mark(mark_kind))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .text_size(px(14.))
                                            .line_height(px(16.))
                                            .child(title)
                                            .when(!badge.is_empty(), |line| {
                                                line.child(
                                                    img(badge).w_auto().h(px(13.5)).flex_none(),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .line_height(px(14.))
                                            .text_color(detail_color)
                                            .child(detail),
                                    ),
                            )
                            .when(selected, |line| {
                                line.child(
                                    img("icons/音质选项/quality_selected.svg")
                                        .size(px(24.))
                                        .flex_none(),
                                )
                            })
                    }),
            )
    }

    /// 打开 / 关闭音质弹层。
    fn set_quality_popover(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.quality_open == open {
            return;
        }
        self.quality_open = open;
        cx.notify();
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
        // 展开状态和共享渐变底都来自 NowPlaying：播放栏只读，不再自持镜像。
        // `Entity::read` 会借住 `cx`，所以放在一个 block 里一次取完再往下走。
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
        // 红心跟随这首歌真实的喜欢状态，而不是固定实心。
        let liked =
            song_id.is_some_and(|song_id| self.library.read(cx).liked_song_ids.contains(&song_id));
        let snapshot = self.playback.read(cx).snapshot();
        // 播放模式的图标和按下后的目标都由控制器给出，这里只负责显示。
        let mode = snapshot.mode;
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
        // 触发按钮的按下/松开职责：capture 阶段申请抑制（见 SidePanel），
        // click 再决定开合或原地换内容；不再需要渲染时快照守卫。

        div()
            // 整条栏是一个点击区：收起态点任意空白处展开歌词页，展开态再点收起，
            // 统一 toggle 单一状态源（和页头的收起按钮殊途同归）。
            //
            // 冒泡规则：自带行为的部件在各自处理器里 `stop_propagation`
            // （播放控制、红心、音量、进度条、两个浮层和队列面板），
            // 所以那些控件上的点击不会走到这里；冒到这里的就是"点了栏本身"。
            // 反过来，评论数、更多这些还没接行为的图标暂时就归到这里管，
            // 等它们接上自己的动作时，再在各自处理器里补一句停传播。
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
                                "player-like-button",
                                like_icon_path(liked),
                                self.counts[0],
                                song_id.is_some(),
                                if liked {
                                    theme_colors.primary
                                } else {
                                    colors.muted_foreground
                                },
                                if liked {
                                    theme_colors.primary.alpha(theme_colors.primary.a * 0.88)
                                } else {
                                    icon_hover_color(colors.muted_foreground, colors)
                                },
                                if liked {
                                    theme_colors.primary.alpha(theme_colors.primary.a * 0.88)
                                } else {
                                    colors.foreground
                                },
                                colors,
                                cx,
                            ))
                            .child(self.interaction_count(
                                "player-comment-button",
                                "icons/comment.svg",
                                self.counts[1],
                                false,
                                colors.muted_foreground,
                                icon_hover_color(colors.muted_foreground, colors),
                                colors.foreground,
                                colors,
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
                                    mode.label(),
                                    mode.icon_path(),
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
                                .on_click(|_, window, cx| {
                                    cx.stop_propagation();
                                    super::open_collect_window(window, cx);
                                }),
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
