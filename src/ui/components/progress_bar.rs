use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{
    Slider, SliderIndicator, SliderThumb, SliderTrack, Theme, Transition, transition,
};
use gpui_kit::component::slider::{SliderEvent, SliderState};

use super::{ALBUM_REVEAL_DURATION, format_duration};
use crate::playback::PlaybackController;
use crate::ui::cover_color::{
    Backdrop, CoverGradient, blend_color, blend_colors, dark_colors, dark_gradient,
};
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

fn slider_state(position: Duration, duration: Duration) -> SliderState {
    SliderState::new()
        .min(0.)
        .max(duration.as_millis() as f32)
        .step(1.)
        .default_value(progress_millis(position.min(duration)))
}

fn progress_millis(position: Duration) -> f32 {
    (position.as_secs() * 1000) as f32
}

fn seek_position(milliseconds: f32, duration: Duration) -> Option<Duration> {
    milliseconds
        .is_finite()
        .then(|| Duration::from_millis(milliseconds.max(0.).round() as u64).min(duration))
}

pub struct ProgressBar {
    playback: Entity<PlaybackController>,
    backdrop: Backdrop,
    slider: Entity<SliderState>,
    hovered: bool,
    dark: bool,
    drag: SeekDrag,
    _playback_subscription: Subscription,
    _slider_subscription: Subscription,
}

impl ProgressBar {
    pub fn new(
        playback: Entity<PlaybackController>,
        backdrop: Backdrop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let controller = playback.read(cx);
        let position = controller.snapshot().position;
        let duration = controller.snapshot().duration;
        let revision = controller.snapshot().revision;
        let slider = cx.new(|_| slider_state(position, duration));
        let playback_subscription =
            cx.observe_in(&playback, window, |this, playback, window, cx| {
                let playback = playback.read(cx);
                let revision = playback.snapshot().revision;
                let duration = playback.snapshot().duration;
                let position = playback.snapshot().position.min(duration);
                let milliseconds = progress_millis(position);
                let enabled = playback.can_seek();
                let changed = this.drag.sync_revision(revision);
                let range_changed = this.slider.read(cx).max_value() != duration.as_millis() as f32;
                if changed || range_changed || (!enabled && this.drag.dragging) {
                    this.drag.dragging = false;
                    // 换 Entity 更新时长范围，并清除库内部旧 DragThumb/DragSlider 手势。
                    this.slider = cx.new(|_| slider_state(position, duration));
                    this._slider_subscription =
                        Self::subscribe_slider(&this.slider, revision, window, cx);
                }
                // 播放时钟只更新非拖动状态，避免覆盖用户正在预览的位置。
                if !this.drag.dragging && this.slider.read(cx).value().end() != milliseconds {
                    this.slider
                        .update(cx, |slider, cx| slider.set_value(milliseconds, window, cx));
                }
                cx.notify();
            });
        let slider_subscription = Self::subscribe_slider(&slider, revision, window, cx);
        Self {
            playback,
            backdrop,
            slider,
            hovered: false,
            dark: false,
            drag: SeekDrag {
                revision,
                dragging: false,
            },
            _playback_subscription: playback_subscription,
            _slider_subscription: slider_subscription,
        }
    }

    pub(super) fn set_dark(&mut self, dark: bool, cx: &mut Context<Self>) {
        self.dark = dark;
        cx.notify();
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
                            if playback.snapshot().revision == revision
                                && playback.can_seek()
                                && let Some(position) =
                                    seek_position(value.end(), playback.snapshot().duration)
                            {
                                playback.seek_to(position, cx);
                            }
                        });
                    }
                    if !this.drag.dragging {
                        let snapshot = this.playback.read(cx).snapshot();
                        let milliseconds =
                            progress_millis(snapshot.position.min(snapshot.duration));
                        this.slider
                            .update(cx, |slider, cx| slider.set_value(milliseconds, window, cx));
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
        let theme_colors = Theme::global(cx).tokens.colors;
        // 与播放栏同一时长、同一时刻启动：展开/收起时轨道的颜色一起过渡。
        let expand = transition(
            "player-progress-expand",
            if self.dark { 1_f32 } else { 0. },
            Transition::new(ALBUM_REVEAL_DURATION).ease(ease_out_quint()),
            window,
            cx,
        );
        let background = self
            .backdrop
            .gradient()
            .unwrap_or_else(|| dark_gradient(None));
        let colors = blend_colors(
            theme_colors,
            dark_colors(theme_colors, background.0[1].into()),
            expand,
        );
        let track_color = blend_color(
            rgb(0xe2e3e5).into(),
            dark_colors(theme_colors, background.0[1].into()).border,
            expand,
        );
        let progress = self.slider.read(cx).percentage().end;
        let playback = self.playback.read(cx);
        let duration = playback.snapshot().duration;
        let enabled = playback.can_seek();
        let elapsed = if self.drag.dragging {
            seek_position(self.slider.read(cx).value().end(), duration).unwrap_or_default()
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
        let backdrop = self.backdrop.clone();

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
                                    .bg(track_color),
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
                                            .child(
                                                canvas(
                                                    |_, _, _| {},
                                                    move |bounds, _, window, _| {
                                                        // 绘制时读取同一帧的渐变：填充色从主题主色过渡到渐变进度色。
                                                        let gradient = backdrop
                                                            .gradient()
                                                            .unwrap_or_else(|| dark_gradient(None));
                                                        let color = blend_color(
                                                            theme_colors.primary,
                                                            progress_color(gradient),
                                                            expand,
                                                        );
                                                        window.paint_quad(fill(bounds, color));
                                                    },
                                                )
                                                .absolute()
                                                .inset_0(),
                                            ),
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
                                            .rounded_full()
                                            .bg(white())
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
                                                        .bg(blend_color(
                                                            colors.surface,
                                                            rgb(0x000000).into(),
                                                            0.18 * expand,
                                                        ))
                                                        .text_color(colors.foreground)
                                                        .font_family(DOLPHIN_FAMILY)
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_size(px(13.5))
                                                        .line_height(px(16.))
                                                        .whitespace_nowrap()
                                                        .opacity(t)
                                                        .child(format_duration(elapsed))
                                                        .child(div().mx(px(2.)).child("/"))
                                                        .child(format_duration(duration)),
                                                ),
                                        ),
                                )
                            }),
                    ),
            )
    }
}

fn progress_color(gradient: CoverGradient) -> Hsla {
    let color = Hsla::from(gradient.0[0]);
    hsla(color.h, color.s, (color.l + 0.25).clamp(0.5, 0.7), 1.)
}

#[cfg(test)]
mod tests {
    use super::{SeekDrag, seek_position, slider_state};
    use std::time::Duration;

    #[test]
    fn slider_uses_milliseconds_and_bounds_seek_targets() {
        let duration = Duration::from_millis(240_000);
        let slider = slider_state(Duration::from_millis(120_000), duration);
        assert_eq!(slider.max_value(), 240_000.);
        assert_eq!(slider.step_value(), 1.);
        assert_eq!(slider.percentage().end, 0.5);
        assert_eq!(
            seek_position(120_001., duration),
            Some(Duration::from_millis(120_001))
        );
        assert_eq!(seek_position(-1., duration), Some(Duration::ZERO));
        assert_eq!(seek_position(300_000., duration), Some(duration));
        assert_eq!(seek_position(f32::NAN, duration), None);
        assert_eq!(seek_position(f32::INFINITY, duration), None);
    }

    #[test]
    fn drag_lifecycle_rejects_stale_events_and_observer_races() {
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
        // 新的控制器 revision 已生效，但观察通知尚未更新本地 revision。
        drag.change(8, 9);
        assert!(!drag.dragging);
        drag.change(8, 8);
        assert!(!drag.release(8, 9));
        assert!(!drag.dragging);
    }
}
