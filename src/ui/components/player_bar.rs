use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme, Transition, transition};

use super::progress_bar::ProgressBar;
use super::volume_control::VolumeControl;
use super::{
    LAYER_PROGRESS_BAR, Popover, RotationClock, Vinyl, artist_label, icon_hover_color,
    like_icon_path,
};
use crate::api::MusicApi;
use crate::models::AudioQualityLevel;
use crate::playback::PlaybackController;
use crate::state::library::MusicLibrary;
use crate::ui::cover_color::{Backdrop, CoverGradient, blend_color, blend_colors, dark_colors};
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA, PRESSED_OPACITY};

/// 播放栏固定高度；专辑歌词页据此估算可视区域高度。
pub const PLAYER_BAR_HEIGHT: f32 = 86.;

/// 黑胶页展开/收起的时长。播放栏左封面滑出、整栏配色、进度条、音量、
/// 页面滑入都用这一个值，它们在同一帧启动、同一时长结束；改这里整体一起变。
pub const ALBUM_REVEAL_DURATION: Duration = Duration::from_millis(500);

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
    rotation_clock: Rc<Cell<RotationClock>>,
    mini_vinyl: Entity<Vinyl>,
    album_expanded: bool,
    album_backdrop: Backdrop,
    backdrop_gradient: Option<CoverGradient>,
    _playback_subscription: Subscription,
    _progress_subscription: Subscription,
    _library_subscription: Subscription,
}

pub struct OpenAlbumLyrics;

impl EventEmitter<OpenAlbumLyrics> for PlayerBar {}

impl PlayerBar {
    pub fn new(
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let album_backdrop = Backdrop::default();
        let progress_bar =
            cx.new(|cx| ProgressBar::new(playback.clone(), album_backdrop.clone(), window, cx));
        let volume = cx.new(|cx| VolumeControl::new(playback.clone(), window, cx));
        let rotation_clock: Rc<Cell<RotationClock>> = Rc::default();
        let mini_vinyl = cx.new(|cx| {
            Vinyl::new(
                playback.clone(),
                rotation_clock.clone(),
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
            rotation_clock,
            mini_vinyl,
            album_expanded: false,
            album_backdrop,
            backdrop_gradient: None,
            _playback_subscription: playback_subscription,
            _progress_subscription: progress_subscription,
            _library_subscription: library_subscription,
        };
        this.load_counts(cx);
        this
    }

    pub(crate) fn set_album_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let changed = self.album_expanded != expanded;
        let gradient = self.album_backdrop.gradient();
        let color_changed = expanded && self.backdrop_gradient != gradient;
        self.backdrop_gradient = gradient;
        if changed {
            self.album_expanded = expanded;
            self.progress_bar
                .update(cx, |bar, cx| bar.set_dark(expanded, cx));
            self.volume
                .update(cx, |volume, cx| volume.set_dark(expanded, cx));
        }
        if changed || color_changed {
            cx.notify();
        }
    }

    pub(crate) fn playback(&self) -> Entity<PlaybackController> {
        self.playback.clone()
    }

    pub(crate) fn library(&self) -> Entity<MusicLibrary> {
        self.library.clone()
    }

    pub(in crate::ui) fn album_backdrop(&self) -> Backdrop {
        self.album_backdrop.clone()
    }

    pub(in crate::ui) fn rotation_clock(&self) -> Rc<Cell<RotationClock>> {
        self.rotation_clock.clone()
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
    fn interaction_count(
        &self,
        id: &'static str,
        path: &'static str,
        count: Option<u64>,
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
    ) -> Popover {
        let theme_colors = Theme::global(cx).tokens.colors;
        Popover::new("player-quality-popover")
            .flex_none()
            .anchor(Anchor::BottomCenter)
            .offset(px(12.))
            .trigger(
                Button::new("player-quality-button")
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
                    .child(quality_label),
            )
            .text_color(theme_colors.secondary_foreground)
            .child({
                let colors = theme_colors;
                div()
                    .w(px(380.))
                    .h(px(480.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_4()
                            .py(px(10.))
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("当前歌曲音质"),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .gap(px(8.))
                            .px(px(16.))
                            .pb(px(10.))
                            .children(
                                [
                                    (
                                        AudioQualityLevel::Sky,
                                        "沉浸环绕声",
                                        "Surround Audio",
                                        "环绕音感 最高5.1声道",
                                    ),
                                    (
                                        AudioQualityLevel::JyMaster,
                                        "超清母带",
                                        "Master",
                                        "极致细节 192kHz/24bit",
                                    ),
                                ]
                                .into_iter()
                                .map(
                                    |(quality, title, subtitle, detail)| {
                                        div()
                                            .id(match quality {
                                                AudioQualityLevel::Sky => "quality-sky-card",
                                                _ => "quality-master-card",
                                            })
                                            .flex_1()
                                            .h(px(112.))
                                            .relative()
                                            .rounded(px(10.))
                                            .bg(rgb(0xfff3dc))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.playback.update(cx, |playback, cx| {
                                                    playback.set_quality(quality, cx)
                                                });
                                            }))
                                            .child(
                                                svg()
                                                    .path(match quality {
                                                        AudioQualityLevel::Sky => {
                                                            "icons/音质选项/immersive_audio.svg"
                                                        }
                                                        _ => "icons/音质选项/master_audio.svg",
                                                    })
                                                    .absolute()
                                                    .top(px(10.))
                                                    .left(px(10.))
                                                    .size(px(32.))
                                                    .text_color(rgb(0xe8bd77)),
                                            )
                                            .child(
                                                img("icons/VIP/svip.svg")
                                                    .absolute()
                                                    .top(px(10.))
                                                    .right(px(10.))
                                                    .w(px(33.75))
                                                    .h(px(13.5)),
                                            )
                                            .child(
                                                div()
                                                    .absolute()
                                                    .left(px(10.))
                                                    .bottom(px(10.))
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_size(px(14.))
                                                            .line_height(px(17.))
                                                            .child(title),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(14.))
                                                            .line_height(px(17.))
                                                            .child(subtitle),
                                                    )
                                                    .child(
                                                        div()
                                                            .pt(px(8.))
                                                            .text_size(px(11.))
                                                            .line_height(px(14.))
                                                            .text_color(
                                                                colors.muted_foreground.alpha(0.4),
                                                            )
                                                            .child(detail),
                                                    ),
                                            )
                                    },
                                ),
                            ),
                    )
                    .child(
                        div().flex_1().flex().flex_col().children(
                            [
                                (
                                    AudioQualityLevel::JyEffect,
                                    "高清臻音 (Spatial Audio)",
                                    "高频细节还原与清晰沉浸感，96kHz/24bit",
                                    "臻",
                                    true,
                                ),
                                (
                                    AudioQualityLevel::HiRes,
                                    "高解析度无损 (Hi-Res)",
                                    "更饱满清晰的高解析度音质，最高192kHz/24bit",
                                    "H",
                                    true,
                                ),
                                (
                                    AudioQualityLevel::Lossless,
                                    "无损 (SQ)",
                                    "高保真无损音质，最高48kHz/16bit",
                                    "SQ",
                                    true,
                                ),
                                (
                                    AudioQualityLevel::ExHigh,
                                    "极高 (HQ)",
                                    "近 CD 音质的细节体验，最高320kbps",
                                    "HQ",
                                    false,
                                ),
                                (AudioQualityLevel::Standard, "标准", "128kbps", "标", false),
                            ]
                            .into_iter()
                            .map(
                                |(quality, title, detail, mark, vip)| {
                                    let selected = selected_quality == quality;
                                    let muted_icon = matches!(
                                        quality,
                                        AudioQualityLevel::ExHigh | AudioQualityLevel::Standard
                                    );
                                    let (icon_bg, icon_fg) = if muted_icon {
                                        (colors.muted, colors.muted_foreground.alpha(0.4))
                                    } else {
                                        (rgb(0xfdf0ed).into(), rgb(0xc97f71).into())
                                    };
                                    div()
                                        .id(match quality {
                                            AudioQualityLevel::JyEffect => "quality-jyeffect-row",
                                            AudioQualityLevel::HiRes => "quality-hires-row",
                                            AudioQualityLevel::Lossless => "quality-lossless-row",
                                            AudioQualityLevel::ExHigh => "quality-exhigh-row",
                                            _ => "quality-standard-row",
                                        })
                                        .w_full()
                                        .flex_1()
                                        .flex()
                                        .items_center()
                                        .gap(px(9.))
                                        .px(px(16.))
                                        // GPUI 的 overflow_hidden() 只做矩形裁剪，底部圆角让背景贴合弹层。
                                        .when(quality == AudioQualityLevel::Standard, |row| {
                                            row.rounded_b(px(10.))
                                        })
                                        .hover(|row| row.bg(colors.muted))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.playback.update(cx, |playback, cx| {
                                                playback.set_quality(quality, cx)
                                            });
                                        }))
                                        .child(
                                            div()
                                                .size(px(32.))
                                                .flex_none()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded_full()
                                                .bg(icon_bg)
                                                .text_color(icon_fg)
                                                .text_size(px(11.))
                                                .child(mark),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .flex_col()
                                                .gap(px(8.))
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap(px(4.))
                                                        .child(
                                                            div()
                                                                .text_size(px(14.))
                                                                .line_height(px(16.))
                                                                .child(title),
                                                        )
                                                        .when(vip, |line| {
                                                            line.child(
                                                                img("icons/VIP/vip.svg")
                                                                    .w(px(30.375))
                                                                    .h(px(13.5)),
                                                            )
                                                        }),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .line_height(px(14.))
                                                        .text_color(
                                                            colors.muted_foreground.alpha(0.4),
                                                        )
                                                        .child(detail),
                                                ),
                                        )
                                        .when(selected, |row| {
                                            row.child(
                                                img("icons/音质选项/quality_selected.svg")
                                                    .size(px(24.))
                                                    .flex_none(),
                                            )
                                        })
                                },
                            ),
                        ),
                    )
            })
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
        // 展开/收起时整条栏的颜色跟着黑胶页的滑入一起过渡，而不是瞬间跳变。
        let expand = transition(
            "player-bar-expand",
            if self.album_expanded { 1_f32 } else { 0. },
            Transition::new(ALBUM_REVEAL_DURATION).ease(ease_out_quint()),
            window,
            cx,
        );
        self.mini_vinyl.read(cx).clear();
        let backdrop_color = self
            .album_backdrop
            .gradient()
            .map(|gradient| Hsla::from(gradient.0[1]))
            .unwrap_or(hsla(0., 0., 0.12, 1.));
        let colors = blend_colors(
            theme_colors,
            dark_colors(theme_colors, backdrop_color),
            expand,
        );
        let expanded = self.progress_bar.read(cx).expanded();
        let shadow_opacity = transition(
            "player-bar-shadow",
            if expanded && !self.album_expanded {
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
                        artist_label(song, colors),
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

        div()
            .w_full()
            .h(px(PLAYER_BAR_HEIGHT))
            .flex_none()
            .flex()
            .flex_col()
            // 普通页面保留进度条悬停阴影；展开页不绘制。
            .when(!self.album_expanded, |bar| {
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
            .border_t_1()
            .border_color(colors.border)
            .relative()
            .when(self.album_expanded, |bar| {
                let backdrop = self.album_backdrop.clone();
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
                                    .cursor_pointer()
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(OpenAlbumLyrics);
                                    }))
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
                                        this.playback.update(cx, |playback, cx| playback.next(cx));
                                    },
                                )),
                            )
                            .child(hover_icon(
                                "player-playlist-button",
                                "播放列表",
                                "icons/playlist.svg",
                                IconSize::Large.pixels(),
                                colors.muted_foreground,
                                colors,
                            )),
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
                            .child(hover_icon(
                                "player-collect-button",
                                "收藏",
                                "icons/collect.svg",
                                IconSize::Middle.pixels(),
                                colors.muted_foreground,
                                colors,
                            ))
                            .child(self.volume.clone())
                            .child(hover_icon(
                                "player-more-button",
                                "更多",
                                "icons/xpoint.svg",
                                IconSize::Middle.pixels(),
                                colors.muted_foreground,
                                colors,
                            )),
                    ),
            )
    }
}
