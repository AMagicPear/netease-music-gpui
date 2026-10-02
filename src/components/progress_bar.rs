use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{
    Slider, SliderIndicator, SliderThumb, SliderTrack, Theme, Transition, transition,
};
use gpui_kit::component::slider::{SliderEvent, SliderState};

use crate::state::playback::PlaybackState;
use crate::theme::DOLPHIN_FAMILY;

/// 轨道静止 / 悬浮时的高度
const REST_HEIGHT: f32 = 2.;
const HOVER_HEIGHT: f32 = 6.;

/// 悬浮时显示在播放头上的白色圆点直径
const HANDLE_SIZE: f32 = 16.;
/// 向上覆盖页面边缘的命中区域，视觉轨道仍留在播放栏原来的位置。
const HIT_SLOP_TOP: f32 = 6.;

pub struct ProgressBar {
    playback: Entity<PlaybackState>,
    slider: Entity<SliderState>,
    hovered: bool,
    dragging: bool,
    _playback_subscription: Subscription,
    _slider_subscription: Subscription,
}

impl ProgressBar {
    pub fn new(
        playback: Entity<PlaybackState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let progress = playback.read(cx).progress();
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.0001)
                .default_value(progress)
        });
        let playback_subscription =
            cx.observe_in(&playback, window, |this, playback, window, cx| {
                // 播放时钟只更新非拖动状态，避免覆盖用户正在预览的位置。
                if !this.dragging {
                    let progress = playback.read(cx).progress();
                    if this.slider.read(cx).value().end() != progress {
                        this.slider
                            .update(cx, |slider, cx| slider.set_value(progress, window, cx));
                    }
                }
                cx.notify();
            });
        let slider_subscription = cx.subscribe_in(&slider, window, |this, _, event, _, cx| {
            match event {
                SliderEvent::Change(_) => this.dragging = true,
                SliderEvent::Release(value) => {
                    this.dragging = false;
                    this.playback.update(cx, |playback, cx| {
                        playback.seek_to_progress(value.end());
                        cx.notify();
                    });
                }
            }
            cx.notify();
        });
        Self {
            playback,
            slider,
            hovered: false,
            dragging: false,
            _playback_subscription: playback_subscription,
            _slider_subscription: slider_subscription,
        }
    }

    pub fn expanded(&self) -> bool {
        self.hovered || self.dragging
    }
}

impl Render for ProgressBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let progress = self.slider.read(cx).percentage().end;
        let playback = self.playback.read(cx);
        let duration = playback
            .current_song
            .as_ref()
            .map_or(Duration::ZERO, |song| song.duration);
        let enabled = !duration.is_zero();
        let elapsed = if self.dragging {
            duration.mul_f64(f64::from(progress))
        } else {
            playback.position.min(duration)
        };
        // 稳定 ID 的 transition 从当前采样值反向，并由库处理 reduced motion。
        let t = transition(
            "player-progress-hover",
            if self.expanded() { 1. } else { 0. },
            Transition::new(Duration::from_millis(130)).ease(ease_out_quint()),
            window,
            cx,
        );
        let height = REST_HEIGHT + (HOVER_HEIGHT - REST_HEIGHT) * t;

        div()
            .id("player-progress-bar")
            .absolute()
            .top(px(-HIT_SLOP_TOP))
            .left_0()
            .right_0()
            .h(px(12. + HIT_SLOP_TOP))
            .occlude()
            .on_hover(cx.listener(|this, hovered, _, cx| {
                if this.hovered == *hovered {
                    return;
                }
                this.hovered = *hovered;
                cx.notify();
            }))
            .child(
                Slider::new(&self.slider)
                    .disabled(!enabled)
                    .absolute()
                    .left_0()
                    .right_0()
                    .top_0()
                    .h_full()
                    .child(
                        SliderTrack::new(&self.slider)
                            .disabled(!enabled)
                            .relative()
                            .size_full()
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .top(px(HIT_SLOP_TOP + (REST_HEIGHT - height) / 2.))
                                    .h(px(height))
                                    .bg(colors.border),
                            )
                            .child(
                                // Indicator 的完整宽度用于指针到进度的映射；填充放在内部。
                                SliderIndicator::new(&self.slider)
                                    .absolute()
                                    .left_0()
                                    .w_full()
                                    .top(px(HIT_SLOP_TOP + (REST_HEIGHT - height) / 2.))
                                    .h(px(height))
                                    .child(
                                        div()
                                            .absolute()
                                            .top_0()
                                            .left_0()
                                            .h_full()
                                            .w(relative(progress))
                                            .bg(colors.primary),
                                    ),
                            )
                            .child(
                                SliderThumb::new(&self.slider)
                                    .disabled(!enabled)
                                    .absolute()
                                    .left(relative(progress))
                                    .ml(px(-HANDLE_SIZE / 2.))
                                    .top(px(HIT_SLOP_TOP + (REST_HEIGHT - HANDLE_SIZE) / 2.))
                                    // 命中区域保持固定，只有可见白色播放头随悬停缩放。
                                    .size(px(HANDLE_SIZE))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        div()
                                            .size(px(HANDLE_SIZE * t))
                                            .opacity(t)
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
                            .when(enabled && t > 0., |track| {
                                let elapsed = elapsed.as_secs();
                                let total = duration.as_secs();
                                track.child(
                                    // 零尺寸锚点跟随播放头，anchored 负责窗口边缘避让。
                                    div()
                                        .absolute()
                                        .left(relative(progress))
                                        .top(px(
                                            HIT_SLOP_TOP + (REST_HEIGHT - HANDLE_SIZE) / 2. - 6.
                                        ))
                                        .size(px(0.))
                                        .child(
                                            anchored()
                                                .anchor(Anchor::BottomCenter)
                                                .snap_to_window_with_margin(px(4.))
                                                .child(
                                                    div()
                                                        .h(px(28.))
                                                        .px(px(16.))
                                                        .flex()
                                                        .items_center()
                                                        .rounded_full()
                                                        .bg(colors.surface)
                                                        .text_color(colors.foreground)
                                                        .font_family(DOLPHIN_FAMILY)
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_size(px(13.5))
                                                        .line_height(px(16.))
                                                        .whitespace_nowrap()
                                                        .opacity(t)
                                                        .child(format!(
                                                            "{:02}:{:02}",
                                                            elapsed / 60,
                                                            elapsed % 60
                                                        ))
                                                        .child(div().mx(px(2.)).child("/"))
                                                        .child(format!(
                                                            "{:02}:{:02}",
                                                            total / 60,
                                                            total % 60
                                                        )),
                                                ),
                                        ),
                                )
                            }),
                    ),
            )
    }
}
