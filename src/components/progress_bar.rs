use gpui::*;
use gpui_kit::base::{ColorTokens, Theme};

use crate::state::playback::PlaybackState;
use crate::theme::ANIMATION_DURATION;

/// 轨道静止 / 悬浮时的高度
const REST_HEIGHT: f32 = 2.;
const HOVER_HEIGHT: f32 = 6.;

/// 悬浮时显示在播放头上的白色圆点直径
const HANDLE_SIZE: f32 = 16.;

pub struct ProgressBar {
    playback: Entity<PlaybackState>,
    hovered: bool,
    /// 悬浮状态每切换一次就 +1。
    /// `AnimationElement` 的播放进度是按 ElementId 缓存的，换一个 id 就等于让它从头重新播一遍。
    animation_generation: u64,
    _playback_subscription: Subscription,
}

impl ProgressBar {
    pub fn new(playback: Entity<PlaybackState>, cx: &mut Context<Self>) -> Self {
        let playback_subscription = cx.observe(&playback, |_, _, cx| cx.notify());
        Self {
            playback,
            hovered: false,
            animation_generation: 0,
            _playback_subscription: playback_subscription,
        }
    }
}

impl Render for ProgressBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors: ColorTokens = Theme::global(cx).tokens.colors;
        let progress = self.playback.read(cx).progress();
        let hovered = self.hovered;
        let generation = self.animation_generation;

        div()
            .id("player-progress-bar")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(12.))
            .on_hover(cx.listener(|this, hovered, _, cx| {
                if this.hovered == *hovered {
                    return;
                }
                this.hovered = *hovered;
                this.animation_generation += 1;
                cx.notify();
            }))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bg(colors.border)
                    .with_animation(
                        ElementId::NamedInteger("player-progress-track".into(), generation),
                        Animation::new(ANIMATION_DURATION).with_easing(ease_out_quint()),
                        move |this, delta| {
                            // generation 为 0 表示还没发生过悬浮切换，直接停在静止状态，
                            // 避免首次渲染时莫名播一段入场动画。
                            let t = match generation {
                                0 => 0.,
                                _ if hovered => delta, // 展开：0 → 1
                                _ => 1. - delta,       // 收起：1 → 0
                            };
                            let height = REST_HEIGHT + (HOVER_HEIGHT - REST_HEIGHT) * t;
                            // 上移增长量的一半，让轨道的水平中心线始终不动
                            this.h(px(height))
                                .top(px((REST_HEIGHT - height) / 2.))
                                .child(
                                    div()
                                        .relative()
                                        .h_full()
                                        .w(relative(progress))
                                        .bg(colors.primary)
                                        // 播放头上的白色圆点：直径随 t 一起长大，
                                        // 水平方向骑在进度填充的右边缘上，垂直方向与轨道中心线对齐
                                        .child(
                                            div()
                                                .absolute()
                                                .top(px((height - HANDLE_SIZE * t) / 2.))
                                                .right(px(-HANDLE_SIZE * t / 2.))
                                                .size(px(HANDLE_SIZE * t))
                                                .rounded_full()
                                                .bg(colors.primary_foreground)
                                                .shadow(vec![
                                                    BoxShadow::new(
                                                        px(0.),
                                                        px(1.),
                                                        hsla(0., 0., 0., 0.25),
                                                    )
                                                    .blur_radius(px(1.)),
                                                ]),
                                        ),
                                )
                        },
                    ),
            )
    }
}
