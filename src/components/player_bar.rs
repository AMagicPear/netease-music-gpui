use std::time::Duration;

use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme, Transition, transition};

use super::progress_bar::ProgressBar;
use crate::state::playback::PlaybackState;
use crate::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA, PRESSED_OPACITY};

pub struct PlayerBar {
    playback: Entity<PlaybackState>,
    progress_bar: Entity<ProgressBar>,
    play_button_hovered: bool,
    play_button_pressed: bool,
    _playback_subscription: Subscription,
    _progress_subscription: Subscription,
}

impl PlayerBar {
    pub fn new(
        playback: Entity<PlaybackState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let progress_bar = cx.new(|cx| ProgressBar::new(playback.clone(), window, cx));
        let playback_subscription = cx.observe(&playback, |_, _, cx| cx.notify());
        let progress_subscription = cx.observe(&progress_bar, |_, _, cx| cx.notify());
        Self {
            playback,
            progress_bar,
            play_button_hovered: false,
            play_button_pressed: false,
            _playback_subscription: playback_subscription,
            _progress_subscription: progress_subscription,
        }
    }
}

fn interaction_count(
    icon_path: &'static str,
    count: &'static str,
    color: Hsla,
    colors: ColorTokens,
) -> impl IntoElement {
    div()
        .ml_0p5()
        .w(px(28.))
        .h(px(24.))
        .flex_none()
        .child(
            svg()
                .path(icon_path)
                .absolute()
                .left_0()
                .bottom_0()
                .size(IconSize::Large.pixels())
                .text_color(color),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(16.))
                .px(px(2.))
                .rounded_full()
                .bg(colors.surface)
                .text_color(color)
                .text_size(px(9.))
                .font_family(DOLPHIN_FAMILY)
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(px(10.))
                .child(count),
        )
}

fn hover_icon(
    id: &'static str,
    icon_path: &'static str,
    size: Pixels,
    color: Hsla,
    colors: ColorTokens,
) -> impl IntoElement {
    svg()
        .path(icon_path)
        .size(size)
        .flex_none()
        .text_color(color)
        .hover(|style| style.text_color(colors.foreground))
        .id(id)
        // active 属于 StatefulInteractiveElement，必须跟在 .id() 之后（此时是 Stateful<Svg>）。
        // 按下换成一个明确的「按下色」而不是 opacity：图标没有底色，叠 opacity 会在按住拖出时
        // 因失去 hover、退回更浅底色而双重变淡。active 最后生效会覆盖 hover，颜色始终一致。
        .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
}

impl Render for PlayerBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let expanded = self.progress_bar.read(cx).expanded();
        let shadow_opacity = transition(
            "player-bar-shadow",
            if expanded { 1. } else { 0. },
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
        let (title, artist, is_playing) = {
            let playback = self.playback.read(cx);
            playback
                .current_song
                .as_ref()
                .map(|song| (song.title.clone(), song.artist.clone(), playback.is_playing))
                .unwrap_or_else(|| (String::new(), String::new(), playback.is_playing))
        };
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
            .h(px(86.))
            .flex_none()
            .flex()
            .flex_col()
            // GPUI 先绘制阴影，再绘制不透明背景；背景自然遮住下方阴影。
            .shadow(vec![
                BoxShadow::new(
                    px(0.),
                    px(-12.),
                    colors.foreground.alpha(0.18 * shadow_opacity),
                )
                .blur_radius(px(24.))
                .spread_radius(px(12.)),
            ])
            .bg(colors.surface)
            .border_t_1()
            .border_color(colors.border)
            .relative()
            // 进度条覆盖渲染
            .child(deferred(self.progress_bar.clone()).with_priority(1))
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
                            // 旋转黑胶封面
                            .child(img("images/miniVinyl.png").size(px(60.)).flex_none())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .max_w(px(200.))
                                    .flex_shrink_1()
                                    .overflow_hidden()
                                    // 歌曲标题
                                    .child(
                                        div()
                                            .text_color(colors.foreground)
                                            .text_size(px(16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(title),
                                    )
                                    // 歌手/制作人
                                    .child(
                                        div()
                                            .text_color(colors.muted_foreground)
                                            .text_size(px(13.))
                                            .truncate()
                                            .child(artist),
                                    ),
                            )
                            .child(interaction_count(
                                "icons/like.svg",
                                "10w+",
                                colors.primary,
                                colors,
                            ))
                            .child(interaction_count(
                                "icons/comment.svg",
                                "999+",
                                colors.muted_foreground,
                                colors,
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
                            .child(hover_icon(
                                "player-order-button",
                                "icons/播放顺序/顺序.svg",
                                IconSize::Large.pixels(),
                                colors.muted_foreground,
                                colors,
                            ))
                            .child(hover_icon(
                                "player-previous-button",
                                "icons/pre.svg",
                                IconSize::Large.pixels(),
                                colors.secondary_foreground,
                                colors,
                            ))
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
                                            .absolute()
                                            .left(px(if play_button_enlarged { -1. } else { 0. }))
                                            .top(px(if play_button_enlarged { -1. } else { 0. }))
                                            .size(play_button_size)
                                            .rounded_full()
                                            .bg(colors.primary)
                                            .text_color(colors.primary_foreground)
                                            // hover 的反馈交给外层区域的放大（40 → 42），
                                            // 这里不动透明度；按下统一降整体透明度
                                            .hover(|style| style.bg(colors.primary.alpha(0.88)))
                                            .active(|style| style.opacity(PRESSED_OPACITY))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.playback.update(cx, |playback, cx| {
                                                    playback.is_playing = !playback.is_playing;
                                                    cx.notify();
                                                });
                                            }))
                                            .child(play_pause_icon),
                                    ),
                            )
                            .child(hover_icon(
                                "player-next-button",
                                "icons/next.svg",
                                IconSize::Large.pixels(),
                                colors.secondary_foreground,
                                colors,
                            ))
                            .child(hover_icon(
                                "player-playlist-button",
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
                            .child(hover_icon(
                                "player-sq-button",
                                "icons/音质选项/sq.svg",
                                IconSize::Middle.pixels(),
                                colors.muted_foreground,
                                colors,
                            ))
                            .child(hover_icon(
                                "player-collect-button",
                                "icons/collect.svg",
                                IconSize::Middle.pixels(),
                                colors.muted_foreground,
                                colors,
                            ))
                            .child(hover_icon(
                                "player-volume-button",
                                "icons/volume.svg",
                                IconSize::Middle.pixels(),
                                colors.muted_foreground,
                                colors,
                            ))
                            .child(hover_icon(
                                "player-more-button",
                                "icons/xpoint.svg",
                                IconSize::Middle.pixels(),
                                colors.muted_foreground,
                                colors,
                            )),
                    ),
            )
    }
}
