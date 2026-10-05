use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{ColorTokens, Slider, SliderIndicator, SliderThumb, SliderTrack, Theme};
use gpui_kit::component::slider::{SliderEvent, SliderState};

use crate::playback::PlaybackController;
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA};

/// 气泡圆角。
const BALLOON_RADIUS: Pixels = px(10.);
/// 底部小三角：宽度沿矩形底边展开，高度单独占气泡底部的一段。
const ARROW_WIDTH: Pixels = px(12.);
const ARROW_HEIGHT: Pixels = px(6.);
/// 气泡整体尺寸（含三角占的那一段高度）。
const BALLOON_WIDTH: Pixels = px(36.);
const BALLOON_HEIGHT: Pixels = px(140.);
/// 三角尖端与图标顶部的间距。
const BALLOON_GAP: Pixels = px(2.);
/// 轨道与滑块的视觉尺寸；命中宽度比视觉宽度宽，手指不必瞄准 4px。
const TRACK_WIDTH: Pixels = px(6.);
const TRACK_HIT_WIDTH: Pixels = px(20.);
const THUMB_SIZE: Pixels = px(10.);
/// 鼠标在图标与气泡之间移动时会短暂离开两者，延迟关闭才不会闪一下。
const CLOSE_DELAY: Duration = Duration::from_millis(160);
/// 延迟绘制的优先级：进度条占 1，气泡要盖在它上面。
const BALLOON_PRIORITY: usize = 2;

/// 悬停状态的来源，用于区分是谁的 hover 发生了变化。
#[derive(Clone, Copy)]
enum HoverTarget {
    Trigger,
    Balloon,
}

pub struct VolumeControl {
    playback: Entity<PlaybackController>,
    slider: Entity<SliderState>,
    trigger_hovered: bool,
    balloon_hovered: bool,
    /// 鼠标按在气泡里（多半是在拖音量）：此时即使指针跑出气泡也不能收起。
    pressed: bool,
    open: bool,
    /// 每次悬停/按键变化自增；过期的延迟关闭任务醒来时发现自己过期就不再动作。
    hover_generation: usize,
    _playback_subscription: Subscription,
    _slider_subscription: Subscription,
}

impl VolumeControl {
    pub fn new(
        playback: Entity<PlaybackController>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let volume = playback.read(cx).snapshot().volume;
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(volume * 100.)
        });
        // 音量可能在别处被改（或初始化时还没拿到设备），反过来同步滑块。
        let playback_subscription =
            cx.observe_in(&playback, window, |this, playback, window, cx| {
                let volume = playback.read(cx).snapshot().volume * 100.;
                if (this.slider.read(cx).value().end() - volume).abs() > 0.5 {
                    this.slider
                        .update(cx, |slider, cx| slider.set_value(volume, window, cx));
                }
                cx.notify();
            });
        // 拖动实时生效：Change 是拖动中，Release 是松手，两者都提交。
        let slider_subscription = cx.subscribe_in(&slider, window, |this, _, event, _, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.end(),
            };
            this.playback.update(cx, |playback, cx| {
                playback.set_volume((value / 100.).clamp(0., 1.), cx);
            });
        });
        Self {
            playback,
            slider,
            trigger_hovered: false,
            balloon_hovered: false,
            pressed: false,
            open: false,
            hover_generation: 0,
            _playback_subscription: playback_subscription,
            _slider_subscription: slider_subscription,
        }
    }

    fn set_hover(&mut self, target: HoverTarget, hovered: bool, cx: &mut Context<Self>) {
        match target {
            HoverTarget::Trigger => self.trigger_hovered = hovered,
            HoverTarget::Balloon => self.balloon_hovered = hovered,
        }
        self.hover_generation += 1;
        if hovered {
            self.open = true;
            cx.notify();
            return;
        }
        self.schedule_close(cx);
    }

    fn set_pressed(&mut self, pressed: bool, cx: &mut Context<Self>) {
        self.pressed = pressed;
        self.hover_generation += 1;
        if pressed {
            return;
        }
        // 松手时指针可能已经在气泡外面了，这时按正常规则收起。
        self.schedule_close(cx);
    }

    /// 延迟关闭：鼠标在图标与气泡之间移动、或拖动时短暂离开气泡，都不该立刻收起。
    fn schedule_close(&mut self, cx: &mut Context<Self>) {
        let generation = self.hover_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOSE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.hover_generation == generation
                    && !this.trigger_hovered
                    && !this.balloon_hovered
                    && !this.pressed
                {
                    this.open = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn balloon(&self, percent: f32, colors: ColorTokens, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("player-volume-balloon")
            .relative()
            .w(BALLOON_WIDTH)
            .h(BALLOON_HEIGHT)
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.set_hover(HoverTarget::Balloon, *hovered, cx);
            }))
            // 捕获阶段接按下/抬起：滑块自己会在冒泡阶段 stop_propagation，
            // 冒泡监听收不到按在滑块上的那一下，拖到气泡外就会被收起。
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                this.set_pressed(true, cx);
            }))
            .capture_any_mouse_up(cx.listener(|this, _, _, cx| {
                this.set_pressed(false, cx);
            }))
            // 在气泡外松手时只有这个回调会到。
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.set_pressed(false, cx);
                }),
            )
            // 背景先画，内容后画：子元素按声明顺序绘制。
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let rect = Bounds::new(
                            bounds.origin,
                            Size {
                                width: bounds.size.width,
                                height: bounds.size.height - ARROW_HEIGHT,
                            },
                        );
                        // 投影只支持矩形（paint_drop_shadows 只吃 corner_radii），
                        // 三角没有自己的投影；当前投影很轻，看不出断裂。
                        window.paint_drop_shadows(
                            rect,
                            Corners::all(BALLOON_RADIUS),
                            &[BoxShadow::new(px(0.), px(2.), colors.foreground.alpha(0.1))
                                .blur_radius(px(6.))],
                        );
                        let Ok(path) = balloon_path(bounds) else {
                            return;
                        };
                        window.paint_path(path, colors.surface);
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    // 内容只占矩形区，底部那一段留给三角。
                    .h(BALLOON_HEIGHT - ARROW_HEIGHT)
                    .flex()
                    .flex_col()
                    .items_center()
                    // 数字贴着顶边往上提，再和竖条之间留一段呼吸的空隙。
                    .pt(px(10.))
                    .gap(px(10.))
                    .pb(px(12.))
                    .text_color(colors.foreground)
                    .child(
                        div()
                            .font_family(DOLPHIN_FAMILY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(12.))
                            .line_height(px(14.))
                            .child(format!("{}", (percent * 100.).round())),
                    )
                    .child(self.volume_slider(percent, colors)),
            )
    }

    fn volume_slider(&self, percent: f32, colors: ColorTokens) -> Slider {
        Slider::new(&self.slider)
            .vertical()
            .flex_1()
            .w(TRACK_HIT_WIDTH)
            .child(
                SliderTrack::new(&self.slider)
                    .axis(Axis::Vertical)
                    .relative()
                    .size_full()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(0.5))
                            .ml(-TRACK_WIDTH / 2.)
                            .w(TRACK_WIDTH)
                            .rounded_full()
                            .bg(colors.border),
                    )
                    // Indicator 的 bounds 决定指针位置到音量的映射，所以让它铺满轨道。
                    .child(
                        SliderIndicator::new(&self.slider)
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(0.5))
                            .ml(-TRACK_WIDTH / 2.)
                            .w(TRACK_WIDTH)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .h(relative(percent))
                                    .rounded_full()
                                    .bg(colors.primary),
                            )
                            .child(
                                SliderThumb::new(&self.slider)
                                    .axis(Axis::Vertical)
                                    .absolute()
                                    .left(relative(0.5))
                                    .ml(-THUMB_SIZE / 2.)
                                    .bottom(relative(percent))
                                    .mb(-THUMB_SIZE / 2.)
                                    .size(THUMB_SIZE)
                                    .rounded_full()
                                    .bg(colors.primary_foreground)
                                    .shadow(vec![
                                        BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.25))
                                            .blur_radius(px(1.)),
                                    ]),
                            ),
                    ),
            )
    }
}

impl Render for VolumeControl {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let percent = self.slider.read(cx).percentage().end;
        // 气泡展开时图标跟着高亮，让人看出音量条是从这个图标弹出的。
        let highlighted = self.open || self.trigger_hovered;
        let icon_color = if highlighted {
            colors.foreground
        } else {
            colors.muted_foreground
        };

        div()
            .id("player-volume")
            .relative()
            .flex_none()
            .size(IconSize::Middle.pixels())
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.set_hover(HoverTarget::Trigger, *hovered, cx);
            }))
            .child(
                svg()
                    .path("icons/volume.svg")
                    .size(IconSize::Middle.pixels())
                    .flex_none()
                    .text_color(icon_color)
                    .id("player-volume-icon")
                    .active(move |style| {
                        style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA))
                    }),
            )
            // 气泡钉死在图标正上方：底边距图标 BALLOON_GAP，水平中心与图标对齐。
            // 不做窗口边界避让——音量按钮离窗口右边界还隔着"更多"按钮，越不了界。
            .when(self.open, |this| {
                this.child(
                    // 进度条也是延迟绘制（优先级 1），气泡必须比它更晚画，
                    // 否则气泡会被进度线横穿过去。
                    deferred(
                        self.balloon(percent, colors, cx)
                            .absolute()
                            .bottom(IconSize::Middle.pixels() + BALLOON_GAP)
                            .left(relative(0.5))
                            .ml(-BALLOON_WIDTH / 2.),
                    )
                    .with_priority(BALLOON_PRIORITY),
                )
            })
    }
}

/// 气泡轮廓：圆角矩形，底边中途绕出一个向下的三角。
///
/// 三角直接长在底边上，整条路径一次填充，
/// 不像两个形状拼接那样在小数 DPI 下露出接缝。
fn balloon_path(bounds: Bounds<Pixels>) -> Result<Path<Pixels>> {
    let mut path = PathBuilder::fill();
    let left = bounds.origin.x;
    let top = bounds.origin.y;
    let right = bounds.origin.x + bounds.size.width;
    // 矩形底边：三角从这里往下长出去。
    let bottom = top + bounds.size.height - ARROW_HEIGHT;
    let tip_y = top + bounds.size.height;
    let r = BALLOON_RADIUS;
    let tip_x = left + bounds.size.width / 2.;
    let arrow_left = tip_x - ARROW_WIDTH / 2.;
    let arrow_right = tip_x + ARROW_WIDTH / 2.;

    path.move_to(point(left + r, top));
    path.line_to(point(right - r, top));
    path.arc_to(point(r, r), px(0.), false, true, point(right, top + r));
    path.line_to(point(right, bottom - r));
    path.arc_to(point(r, r), px(0.), false, true, point(right - r, bottom));
    // 底边从右往左走，走到三角时绕出去再回来。
    path.line_to(point(arrow_right, bottom));
    path.line_to(point(tip_x, tip_y));
    path.line_to(point(arrow_left, bottom));
    path.line_to(point(left + r, bottom));
    path.arc_to(point(r, r), px(0.), false, true, point(left, bottom - r));
    path.line_to(point(left, top + r));
    path.arc_to(point(r, r), px(0.), false, true, point(left + r, top));
    path.close();
    path.build()
}
