use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::{
    Bounds, Context, EventEmitter, FontWeight, IntoElement, ParentElement, Pixels, Render,
    SharedString, Styled, Window, canvas, div, fill, point, px, size,
};
use gpui_kit::base::{Button, Theme};

use crate::theme::DOLPHIN_FAMILY;

const INDICATOR_DURATION: Duration = Duration::from_millis(200);
const INDICATOR_WIDTH: Pixels = px(16.);
const INDICATOR_HEIGHT: Pixels = px(3.);

pub struct TabItem {
    label: SharedString,
    count: Option<SharedString>,
}

impl TabItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            count: None,
        }
    }
}

/// 仅在选项改变时发出，动画帧不会触发页面内容的更新。
pub struct TabChanged;

pub struct TabBar {
    items: Vec<TabItem>,
    selected_index: usize,
    indicator: IndicatorState,
}

impl EventEmitter<TabChanged> for TabBar {}

impl TabBar {
    pub fn new(items: Vec<TabItem>) -> Self {
        assert!(!items.is_empty(), "TabBar requires at least one tab");
        Self {
            items,
            selected_index: 0,
            indicator: IndicatorState::default(),
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn set_count(&mut self, index: usize, count: Option<String>, cx: &mut Context<Self>) {
        self.items[index].count = count.map(Into::into);
        cx.notify();
    }

    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.selected_index == index {
            return;
        }
        self.indicator.start(Instant::now());
        self.selected_index = index;
        cx.emit(TabChanged);
        cx.notify();
    }
}

#[derive(Default)]
struct IndicatorState {
    painted: Option<Bounds<Pixels>>,
    target: Option<Bounds<Pixels>>,
    transition: Option<IndicatorTransition>,
}

struct IndicatorTransition {
    from: Bounds<Pixels>,
    started_at: Instant,
}

impl IndicatorState {
    fn start(&mut self, now: Instant) {
        // 连续点击从上一帧实际画出的形状出发，包括尚未收回的宽度。
        self.transition = self.painted.map(|from| IndicatorTransition {
            from,
            started_at: now,
        });
        self.target = None;
    }

    fn sample(
        &mut self,
        target: Bounds<Pixels>,
        now: Instant,
        reduce_motion: bool,
    ) -> (Bounds<Pixels>, bool) {
        // 滚动、缩放或重新布局时直接对齐；不要把布局位移当成标签切换。
        if reduce_motion || self.target.is_some_and(|previous| previous != target) {
            self.transition = None;
        }
        self.target = Some(target);
        let mut bounds = target;
        let mut running = false;
        if let Some(transition) = &self.transition {
            let t = now.duration_since(transition.started_at).as_secs_f32()
                / INDICATOR_DURATION.as_secs_f32();
            if t < 1. {
                bounds = stretched_bounds(transition.from, target, t);
                running = true;
            } else {
                self.transition = None;
            }
        }
        self.painted = Some(bounds);
        (bounds, running)
    }
}

/// 前沿先走、后沿追赶；缩小两端的进度差，让拉伸随距离变化但幅度更轻。
fn stretched_bounds(from: Bounds<Pixels>, target: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
    // Smoothstep：起止速度为零，中间加速，再逐渐减速。
    let middle = t * t * (3. - 2. * t);
    // 两端围绕位移进度错开，保留原先 30% 的最大拉伸幅度。
    let stretch = 0.45 * middle * (1. - middle);
    let lead = middle + stretch;
    let trail = middle - stretch;
    let (left_t, right_t) = if target.center().x >= from.center().x {
        (trail, lead)
    } else {
        (lead, trail)
    };
    let left = from.left() + (target.left() - from.left()) * left_t;
    let right = from.right() + (target.right() - from.right()) * right_t;
    Bounds::new(
        point(left, target.top()),
        size(right - left, target.size.height),
    )
}

impl Render for TabBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        div()
            .flex()
            .items_start()
            .gap(px(24.))
            .children(self.items.iter().enumerate().map(|(index, item)| {
                let selected = index == self.selected_index;
                let entity = cx.entity().downgrade();
                Button::new(("tab", index))
                    .flex_none()
                    .selected(selected)
                    .accessibility_label(item.label.clone())
                    .text_color(if selected {
                        colors.foreground
                    } else {
                        colors.muted_foreground
                    })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(item.label.clone()),
                                    )
                                    .child(
                                        // canvas 的 bounds 在本帧布局后立即可用，无需 notify 后再等一帧。
                                        canvas(
                                            move |target, _, cx| {
                                                if !selected {
                                                    return None;
                                                }
                                                entity
                                                    .update(cx, |this, cx| {
                                                        this.indicator.sample(
                                                            target,
                                                            Instant::now(),
                                                            cx.reduce_motion(),
                                                        )
                                                    })
                                                    .ok()
                                            },
                                            move |_, indicator, window, _| {
                                                if let Some((bounds, running)) = indicator {
                                                    window.paint_quad(
                                                        fill(bounds, colors.primary)
                                                            .corner_radii(px(1.5)),
                                                    );
                                                    if running {
                                                        window.request_animation_frame();
                                                    }
                                                }
                                            },
                                        )
                                        .w(INDICATOR_WIDTH)
                                        .h(INDICATOR_HEIGHT),
                                    ),
                            )
                            .when_some(item.count.clone(), |this, count| {
                                this.child(
                                    div()
                                        .ml(px(1.))
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::BOLD)
                                        .font_family(DOLPHIN_FAMILY)
                                        .child(count),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx)))
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicator_first_frame_stretch_interrupt_and_return() {
        let now = Instant::now();
        let first = Bounds::new(
            point(px(0.), px(20.)),
            size(INDICATOR_WIDTH, INDICATOR_HEIGHT),
        );
        let second = Bounds::new(point(px(100.), px(20.)), first.size);
        let mut indicator = IndicatorState::default();
        assert_eq!(indicator.sample(first, now, false), (first, false));
        indicator.start(now);
        let midway = now + INDICATOR_DURATION / 2;
        let (stretched, running) = indicator.sample(second, midway, false);
        assert!(running && stretched.size.width > INDICATOR_WIDTH);
        let farther = Bounds::new(point(px(200.), px(20.)), first.size);
        assert!(stretched_bounds(first, farther, 0.5).size.width > stretched.size.width);
        indicator.start(midway);
        assert_eq!(indicator.sample(first, midway, false), (stretched, true));
        assert_eq!(
            indicator.sample(first, midway + INDICATOR_DURATION, false),
            (first, false)
        );
        // 返回页面仍是目标位置，不会重新生成播放起点。
        assert_eq!(
            indicator.sample(first, midway + INDICATOR_DURATION * 10, false),
            (first, false)
        );
        assert!(stretched_bounds(second, first, 0.5).size.width > INDICATOR_WIDTH);
        indicator.start(midway + INDICATOR_DURATION * 10);
        assert_eq!(
            indicator.sample(second, midway + INDICATOR_DURATION * 10, true),
            (second, false)
        );
    }
}
