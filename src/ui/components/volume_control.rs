use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{
    ColorTokens, Presence, Slider, SliderIndicator, SliderThumb, SliderTrack, Theme, Transition,
    transition,
};
use gpui_kit::component::slider::{SliderEvent, SliderState};

use super::{ALBUM_REVEAL_DURATION, LAYER_VOLUME_BALLOON, icon_hover_color};
use crate::playback::PlaybackController;
use crate::ui::cover_color::{blend_colors, dark_colors};
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
/// 命中区"桥"额外压住图标的高度：让气泡的命中区直接搭到图标上，
/// 连成一整片，鼠标在两者之间移动不会掉出去（也就不需要延迟关闭）。
const BRIDGE_OVERLAP: Pixels = px(2.);
/// 轨道与滑块的视觉尺寸；命中宽度比视觉宽度宽，手指不必瞄准 6px。
const TRACK_WIDTH: Pixels = px(6.);
const TRACK_HIT_WIDTH: Pixels = px(20.);
const THUMB_SIZE: Pixels = px(10.);
/// 气泡进出场：和 gpui-kit dropdown 同一路数的 150ms，位移 4px。
const BALLOON_MOTION_DURATION: Duration = Duration::from_millis(150);
const BALLOON_RISE: Pixels = px(4.);

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
    dark: bool,
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
                    cx.notify();
                }
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
            dark: false,
            _playback_subscription: playback_subscription,
            _slider_subscription: slider_subscription,
        }
    }

    pub(super) fn set_dark(&mut self, dark: bool, cx: &mut Context<Self>) {
        self.dark = dark;
        cx.notify();
    }

    /// 只有图标能让它出现；图标、气泡、以及两者之间那块"桥"都能让它留着。
    /// 桥是气泡的子元素，只在展开时才存在，所以它天生只管消失、不管出现。
    fn refresh_open(&mut self, cx: &mut Context<Self>) {
        // 拖动中指针会跑出气泡，此时必须压住不要收起。
        let open = self.pressed || self.trigger_hovered || self.balloon_hovered;
        if self.open != open {
            self.open = open;
            cx.notify();
        }
    }

    fn set_hover(&mut self, target: HoverTarget, hovered: bool, cx: &mut Context<Self>) {
        match target {
            HoverTarget::Trigger => self.trigger_hovered = hovered,
            HoverTarget::Balloon => self.balloon_hovered = hovered,
        }
        self.refresh_open(cx);
    }

    fn set_pressed(&mut self, pressed: bool, cx: &mut Context<Self>) {
        self.pressed = pressed;
        self.refresh_open(cx);
    }

    /// 命中区容器：可见气泡 + 下面一段透明的"桥"。
    ///
    /// 桥把图标顶边和气泡底边之间那道缝补上，两者连成一整片命中区，
    /// 鼠标走过去不会掉到"谁都不算"的空档里，于是可以即时关闭、不用延迟。
    ///
    /// `interactive` 为假时不挂任何命中/悬停监听：退场动画那 150ms 里气泡还在树上，
    /// 但它只是"正在消失的画面"，不该继续吞鼠标。
    fn balloon(
        &self,
        percent: f32,
        interactive: bool,
        colors: ColorTokens,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id("player-volume-balloon")
            .relative()
            .w(BALLOON_WIDTH)
            .h(BALLOON_HEIGHT + BALLOON_GAP + BRIDGE_OVERLAP)
            .flex()
            .flex_col()
            .when(interactive, |balloon| {
                balloon
                    // 气泡和透明桥共同遮住下方命中区，避免 hover 穿透。
                    .occlude()
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.set_hover(HoverTarget::Balloon, *hovered, cx);
                    }))
                    // 捕获阶段接按下/抬起：滑块自己会在冒泡阶段 stop_propagation，
                    // 冒泡监听收不到按在滑块上的那一下，拖到气泡外就会被收起。
                    .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        if event.button == MouseButton::Left {
                            this.set_pressed(true, cx);
                        }
                    }))
                    .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, _, cx| {
                        if event.button == MouseButton::Left {
                            this.set_pressed(false, cx);
                        }
                    }))
                    // 在气泡外松手时只有这个回调会到。
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.set_pressed(false, cx);
                        }),
                    )
            })
            .child(self.balloon_body(percent, colors))
            // 桥：透明、不画东西，只贡献命中区。
            .child(div().w_full().h(BALLOON_GAP + BRIDGE_OVERLAP))
    }

    /// 可见的那部分：圆角矩形 + 底部三角，高度正好 BALLOON_HEIGHT。
    fn balloon_body(&self, percent: f32, colors: ColorTokens) -> Div {
        div()
            .relative()
            .w_full()
            .h(BALLOON_HEIGHT)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_colors = Theme::global(cx).tokens.colors;
        // 与播放栏同一时长、同一时刻启动，音量图标也跟着一起过渡。
        let expand = transition(
            "player-volume-expand",
            if self.dark { 1_f32 } else { 0. },
            Transition::new(ALBUM_REVEAL_DURATION).ease(ease_out_quint()),
            window,
            cx,
        );
        let colors = blend_colors(
            theme_colors,
            dark_colors(theme_colors, hsla(0., 0., 0.12, 1.)),
            expand,
        );
        let percent = self.slider.read(cx).percentage().end;
        // 指针还在图标上就已经算"展开"了，所以图标高亮直接看 open。
        let icon_color = if self.open {
            icon_hover_color(colors.muted_foreground, colors)
        } else {
            colors.muted_foreground
        };
        // 进出场共用一条 Presence：`should_render()` 在退场期间仍为真，
        // 气泡会继续挂着直到动画放完才卸载——这正是"消失动画"能存在的前提。
        let presence = Presence::new("player-volume-balloon", self.open)
            .transition(Transition::new(BALLOON_MOTION_DURATION).ease(ease_out_quint()))
            .sample(window, cx);
        let progress = presence.progress;

        div()
            .id("player-volume")
            .relative()
            .flex_none()
            .size(IconSize::Middle.pixels())
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
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.set_hover(HoverTarget::Trigger, *hovered, cx);
            }))
            // 气泡钉死在图标正上方：水平中心与图标对齐，底部的"桥"再压住图标 2px。
            // 不做窗口边界避让——音量按钮离窗口右边界还隔着"更多"按钮，越不了界。
            .when(presence.should_render(), |this| {
                this.child(
                    // 进度条和通用弹层都是延迟绘制且层级更低，气泡画在最后才不会被横穿。
                    deferred(
                        self.balloon(percent, self.open, colors, cx)
                            .absolute()
                            // 进场时从图标那头往上浮：起点比终点低 BALLOON_RISE。
                            .bottom(
                                IconSize::Middle.pixels() - BRIDGE_OVERLAP
                                    - BALLOON_RISE * (1. - progress),
                            )
                            .left(relative(0.5))
                            .ml(-BALLOON_WIDTH / 2.)
                            .opacity(progress),
                    )
                    .with_priority(LAYER_VOLUME_BALLOON),
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
