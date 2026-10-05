use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{
    Slider, SliderIndicator, SliderThumb, SliderTrack, Theme, Transition, transition,
};
use gpui_kit::component::slider::{SliderEvent, SliderState};

use crate::playback::PlaybackController;
use crate::ui::theme::DOLPHIN_FAMILY;

/// 轨道静止 / 悬浮时的高度
const REST_HEIGHT: f32 = 2.;
const HOVER_HEIGHT: f32 = 6.;

/// 悬浮时显示在播放头上的白色圆点直径
const HANDLE_SIZE: f32 = 16.;
/// 向上覆盖页面边缘的命中区域，视觉轨道仍留在播放栏原来的位置。
const HIT_SLOP_TOP: f32 = 6.;

/// 事件携带订阅创建时的 revision，旧手势不能被当作新歌曲的拖动。
struct SeekDrag {
    revision: u64,
    dragging: bool,
}

impl SeekDrag {
    fn sync_revision(&mut self, revision: u64) -> bool {
        if self.revision == revision {
            return false;
        }
        self.revision = revision;
        self.dragging = false;
        true
    }

    fn change(&mut self, event_revision: u64, current_revision: u64) {
        if event_revision == self.revision && event_revision == current_revision {
            self.dragging = true;
        }
    }

    fn release(&mut self, event_revision: u64, current_revision: u64) -> bool {
        if event_revision != self.revision {
            return false;
        }
        std::mem::take(&mut self.dragging) && event_revision == current_revision
    }
}

fn slider_state(progress: f32) -> SliderState {
    SliderState::new()
        .min(0.)
        .max(1.)
        .step(0.0001)
        .default_value(progress)
}

pub struct ProgressBar {
    playback: Entity<PlaybackController>,
    slider: Entity<SliderState>,
    hovered: bool,
    drag: SeekDrag,
    _playback_subscription: Subscription,
    _slider_subscription: Subscription,
}

impl ProgressBar {
    pub fn new(
        playback: Entity<PlaybackController>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let controller = playback.read(cx);
        let progress = controller.progress();
        let revision = controller.snapshot().revision;
        let slider = cx.new(|_| slider_state(progress));
        let playback_subscription =
            cx.observe_in(&playback, window, |this, playback, window, cx| {
                let playback = playback.read(cx);
                let revision = playback.snapshot().revision;
                let progress = playback.progress();
                let enabled = playback.can_seek();
                let changed = this.drag.sync_revision(revision);
                if changed || (!enabled && this.drag.dragging) {
                    this.drag.dragging = false;
                    // set_value 不清除库内部手势。换 Entity 同时隔离旧 DragThumb/DragSlider。
                    this.slider = cx.new(|_| slider_state(progress));
                    this._slider_subscription =
                        Self::subscribe_slider(&this.slider, revision, window, cx);
                }
                // 播放时钟只更新非拖动状态，避免覆盖用户正在预览的位置。
                if !this.drag.dragging && this.slider.read(cx).value().end() != progress {
                    this.slider
                        .update(cx, |slider, cx| slider.set_value(progress, window, cx));
                }
                cx.notify();
            });
        let slider_subscription = Self::subscribe_slider(&slider, revision, window, cx);
        Self {
            playback,
            slider,
            hovered: false,
            drag: SeekDrag {
                revision,
                dragging: false,
            },
            _playback_subscription: playback_subscription,
            _slider_subscription: slider_subscription,
        }
    }

    fn subscribe_slider(
        slider: &Entity<SliderState>,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(slider, window, move |this, slider, event, window, cx| {
            if slider != &this.slider {
                return;
            }
            // 订阅通知与 observe 的执行顺序无关，提交前直接核对控制器的 revision。
            let current_revision = this.playback.read(cx).snapshot().revision;
            match event {
                SliderEvent::Change(_) => {
                    if this.playback.read(cx).can_seek() {
                        this.drag.change(revision, current_revision);
                    }
                }
                SliderEvent::Release(value) => {
                    let commit = this.drag.release(revision, current_revision);
                    if commit {
                        this.playback.update(cx, |playback, cx| {
                            let progress = value.end();
                            if playback.snapshot().revision == revision
                                && playback.can_seek()
                                && progress.is_finite()
                            {
                                let position = playback
                                    .snapshot()
                                    .duration
                                    .mul_f64(f64::from(progress.clamp(0., 1.)));
                                playback.seek_to(position, cx);
                            }
                        });
                    }
                    if !this.drag.dragging {
                        let progress = this.playback.read(cx).progress();
                        this.slider
                            .update(cx, |slider, cx| slider.set_value(progress, window, cx));
                    }
                }
            }
            cx.notify();
        })
    }

    pub fn expanded(&self) -> bool {
        self.hovered || self.drag.dragging
    }
}

impl Render for ProgressBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let progress = self.slider.read(cx).percentage().end;
        let playback = self.playback.read(cx);
        let duration = playback.snapshot().duration;
        let enabled = playback.can_seek();
        let elapsed = if self.drag.dragging {
            duration.mul_f64(f64::from(progress))
        } else {
            playback.snapshot().position.min(duration)
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

#[cfg(test)]
mod tests {
    use super::SeekDrag;

    #[test]
    fn release_requires_a_change_in_the_current_revision() {
        let mut drag = SeekDrag {
            revision: 7,
            dragging: false,
        };
        assert!(!drag.release(7, 7));
        drag.change(7, 7);
        assert!(!drag.sync_revision(7));
        assert!(drag.dragging);
        assert!(drag.release(7, 7));
        assert!(!drag.dragging);
        assert!(!drag.release(7, 7));
    }

    #[test]
    fn reload_cancels_drag_even_without_release_and_ignores_old_events() {
        let mut drag = SeekDrag {
            revision: 7,
            dragging: false,
        };
        drag.change(7, 7);
        assert!(drag.sync_revision(8));
        assert!(!drag.dragging);
        drag.change(7, 8);
        assert!(!drag.dragging);
        assert!(!drag.release(7, 8));
        drag.change(8, 8);
        assert!(!drag.release(7, 8));
        assert!(drag.dragging);
        assert!(drag.release(8, 8));
    }

    #[test]
    fn release_checks_revision_before_the_observer_runs() {
        let mut drag = SeekDrag {
            revision: 7,
            dragging: false,
        };
        drag.change(7, 8);
        assert!(!drag.dragging);
        drag.change(7, 7);
        assert!(!drag.release(7, 8));
        assert!(!drag.dragging);
    }
}
