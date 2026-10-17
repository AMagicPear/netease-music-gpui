//! 两屏滚动：滚轮采样、停手吸附与「回到唱片」动画。

use std::time::{Duration, Instant};

use gpui::*;

use super::NowPlayingPage;

// 原生滚动边界会取整，页高却可能有小数。
const SCROLL_END_TOLERANCE: f32 = 1.;
const RETURN_DURATION: Duration = Duration::from_millis(600);
const TWEEN_EPSILON: f32 = 0.5;
// 临界阻尼弹簧，吸附时不越过目标。
const SETTLE_SPRING: SpringConfig = SpringConfig::new(225., 30., 1.);
const SETTLE_EPSILON: f32 = 0.5;
const SETTLE_QUIET_FINE: Duration = Duration::from_millis(110);
const SETTLE_QUIET_COARSE: Duration = Duration::from_millis(400);
const SETTLE_COMMIT: (f32, f32) = (0.4, 0.6);
const DIRECTION_NOISE: f32 = 0.5;

/// 两种程序化运动共用偏移守卫：用户接管后立即让位。
#[derive(Default)]
pub(super) struct ScrollMotion {
    active: Option<Motion>,
    written: Option<f32>,
}

enum Motion {
    Command {
        from: f32,
        target: f32,
        started: Instant,
    },
    Settle {
        target: f32,
        spring: SpringState,
        updated: Instant,
    },
}

impl ScrollMotion {
    pub(super) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    fn command(&mut self, handle: &ScrollHandle, target: Pixels, now: Instant) {
        let from = f32::from(handle.offset().y);
        let to = f32::from(target);
        if let Some(Motion::Command { target, .. }) = &self.active
            && *target == to
        {
            return;
        }
        if (to - from).abs() <= TWEEN_EPSILON {
            *self = Self::default();
            return;
        }
        self.active = Some(Motion::Command {
            from,
            target: to,
            started: now,
        });
        self.written = Some(from);
    }

    fn settle(&mut self, handle: &ScrollHandle, target: Pixels, now: Instant) {
        let from = f32::from(handle.offset().y);
        let to = f32::from(target);
        if (to - from).abs() <= SETTLE_EPSILON {
            *self = Self::default();
            return;
        }
        self.active = Some(Motion::Settle {
            target: to,
            spring: SpringState {
                position: from,
                velocity: 0.,
            },
            updated: now,
        });
        self.written = Some(from);
    }

    pub(super) fn step(
        &mut self,
        handle: &ScrollHandle,
        page_height: f32,
        window: &mut Window,
        now: Instant,
    ) {
        // 评论页吸附目标跟随窗口高度，避免停在缩放前的半屏位置。
        if let Some(Motion::Settle { target, .. }) = &mut self.active
            && *target < 0.
        {
            *target = -page_height;
        }
        if self.advance(handle, now) {
            window.request_animation_frame();
        }
    }

    fn advance(&mut self, handle: &ScrollHandle, now: Instant) -> bool {
        let Some(mut active) = self.active.take() else {
            return false;
        };
        if let Some(written) = self.written
            && (f32::from(handle.offset().y) - written).abs() > TWEEN_EPSILON
        {
            *self = Self::default();
            return false;
        }
        let (y, done) = match &mut active {
            Motion::Command {
                from,
                target,
                started,
            } => {
                let progress = (now.saturating_duration_since(*started).as_secs_f32()
                    / RETURN_DURATION.as_secs_f32())
                .min(1.);
                (
                    *from + (*target - *from) * ease_out_quint()(progress),
                    progress >= 1.,
                )
            }
            Motion::Settle {
                target,
                spring,
                updated,
            } => {
                *spring = SETTLE_SPRING.step(
                    *spring,
                    *target,
                    now.saturating_duration_since(*updated).as_secs_f32(),
                );
                *updated = now;
                let done = SETTLE_SPRING.is_settled(*spring, *target, SETTLE_EPSILON);
                (if done { *target } else { spring.position }, done)
            }
        };
        handle.set_offset(point(px(0.), px(y)));
        self.written = Some(y);
        if done {
            *self = Self::default();
            return false;
        }
        self.active = Some(active);
        true
    }
}

#[derive(Clone, Copy)]
struct WheelSample {
    at: Instant,
    fine: bool,
    lifted: bool,
    /// 正数去评论区，负数回歌词页，与原生 offset 的方向相反。
    direction: f32,
}

/// 惯性仍会产生 Moved 事件，须结合事件间隙判断停手。
#[derive(Default)]
pub(super) struct WheelWatch {
    last: Option<WheelSample>,
}

impl WheelWatch {
    pub(super) fn note(&mut self, now: Instant, event: &ScrollWheelEvent, line_height: Pixels) {
        let delta = f32::from(event.delta.pixel_delta(line_height).y);
        let carried = if event.touch_phase == TouchPhase::Started {
            0.
        } else {
            self.last.map_or(0., |sample| sample.direction)
        };
        let direction = if delta.abs() >= DIRECTION_NOISE {
            -delta.signum()
        } else {
            carried
        };
        let lifted = match event.touch_phase {
            TouchPhase::Started => false,
            TouchPhase::Ended | TouchPhase::Cancelled => true,
            TouchPhase::Moved => self.last.is_some_and(|sample| sample.lifted),
        };
        self.last = Some(WheelSample {
            at: now,
            fine: event.delta.precise(),
            lifted,
            direction,
        });
    }

    fn released(&mut self, now: Instant) -> Option<f32> {
        let sample = self.last?;
        let quiet = if sample.fine && sample.lifted {
            SETTLE_QUIET_FINE
        } else {
            SETTLE_QUIET_COARSE
        };
        if now.saturating_duration_since(sample.at) < quiet {
            return None;
        }
        self.last = None;
        Some(sample.direction)
    }
}

impl NowPlayingPage {
    /// 换歌、重新打开时回到歌词页。
    pub(super) fn reset_page(&mut self) {
        self.scroll_motion = ScrollMotion::default();
        self.wheel_watch = WheelWatch::default();
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    /// 只移动外层，评论内容保持原位一起退场。
    pub(super) fn return_to_song(&mut self, cx: &mut Context<Self>) {
        self.scroll_to_screen(px(0.), cx);
    }

    /// 播放栏的评论按钮在展开态走这里：不导航，只在本页上下两屏之间翻。
    ///
    /// 「看评论」和「滚到评论区」本来就是一回事；整页已经铺满窗口时再去换一个
    /// 评论页，只会把唱片页顶掉。停在评论区就回唱片页，其余情况（在唱片页或
    /// 翻页途中）一律往评论区推。
    pub(in crate::ui) fn toggle_comments_screen(&mut self, cx: &mut Context<Self>) {
        // 页高由上一次渲染量出：还没量过就没有可翻的目标。
        let Some(page_height) = self.page_height else {
            return;
        };
        let target = toggle_target(-f32::from(self.scroll.offset().y), page_height);
        self.scroll_to_screen(target, cx);
    }

    /// 程序化翻到某一屏：`0` 是唱片页，`-page_height` 是评论区。
    fn scroll_to_screen(&mut self, target: Pixels, cx: &mut Context<Self>) {
        self.wheel_watch = WheelWatch::default();
        if cx.reduce_motion() {
            self.scroll_motion = ScrollMotion::default();
            self.scroll.set_offset(point(px(0.), target));
        } else {
            let now = cx.background_executor().now();
            self.scroll_motion.command(&self.scroll, target, now);
        }
        cx.notify();
    }

    pub(super) fn settle_if_idle(
        &mut self,
        expanded: bool,
        page_height: f32,
        now: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !expanded || self.scroll_motion.is_active() {
            return;
        }
        let scrolled = -f32::from(self.scroll.offset().y);
        if song_page_at_rest(scrolled) || reached_comments(scrolled, page_height) {
            return;
        }
        let Some(direction) = self.wheel_watch.released(now) else {
            if self.wheel_watch.last.is_some() {
                window.request_animation_frame();
            }
            return;
        };
        let target = settle_target(scrolled, page_height, direction);
        if cx.reduce_motion() {
            self.scroll.set_offset(point(px(0.), target));
        } else {
            self.scroll_motion.settle(&self.scroll, target, now);
        }
        // cached 视图在 prepaint 中 render，此时 notify 不会安排下一帧。
        window.request_animation_frame();
    }
}

pub(super) fn song_page_at_rest(scrolled: f32) -> bool {
    scrolled <= SCROLL_END_TOLERANCE
}

pub(super) fn reached_comments(scrolled: f32, page_height: f32) -> bool {
    scrolled >= page_height - SCROLL_END_TOLERANCE
}

/// 播放栏评论按钮的翻屏目标：已经在评论区就回唱片页，否则去评论区。
/// 翻页途中（两个静止点之间）算「没在评论区」，按钮再点一下就是往评论区推。
pub(super) fn toggle_target(scrolled: f32, page_height: f32) -> Pixels {
    if reached_comments(scrolled, page_height) {
        px(0.)
    } else {
        px(-page_height)
    }
}

fn settle_target(scrolled: f32, page_height: f32, direction: f32) -> Pixels {
    let (push_down, push_up) = SETTLE_COMMIT;
    let commit = if direction > 0. {
        push_down
    } else if direction < 0. {
        push_up
    } else {
        0.5
    };
    if scrolled / page_height >= commit {
        px(-page_height)
    } else {
        px(0.)
    }
}

#[cfg(test)]
mod tests {
    use super::{SETTLE_QUIET_FINE, ScrollMotion, WheelWatch, settle_target, toggle_target};
    use gpui::{ScrollDelta, ScrollHandle, ScrollWheelEvent, TouchPhase, point, px};
    use std::time::{Duration, Instant};

    #[test]
    fn comment_button_flips_between_the_two_screens() {
        // 唱片页（含容差内的小抖动）→ 评论区。
        assert_eq!(toggle_target(0., 700.), px(-700.));
        assert_eq!(toggle_target(350., 700.), px(-700.));
        // 已经停在评论区 → 回唱片页。
        assert_eq!(toggle_target(700., 700.), px(0.));
        assert_eq!(toggle_target(699.5, 700.), px(0.));
    }

    #[test]
    fn settle_follows_wheel_direction_after_quiet_period() {
        let now = Instant::now();
        for (delta, target) in [(40., px(0.)), (-40., px(-700.))] {
            let mut watch = WheelWatch::default();
            watch.note(
                now,
                &ScrollWheelEvent {
                    delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                    touch_phase: TouchPhase::Ended,
                    ..Default::default()
                },
                px(20.),
            );
            assert!(watch.released(now).is_none());
            let direction = watch.released(now + SETTLE_QUIET_FINE).unwrap();
            assert_eq!(settle_target(350., 700., direction), target);
            assert!(watch.released(now + SETTLE_QUIET_FINE).is_none());
        }
    }

    #[test]
    fn settled_spring_lands_exactly_at_target() {
        let now = Instant::now();
        let handle = ScrollHandle::default();
        handle.set_offset(point(px(0.), px(-350.)));
        let mut motion = ScrollMotion::default();
        motion.settle(&handle, px(-700.), now);
        for frame in 1..=120 {
            if !motion.advance(&handle, now + Duration::from_millis(frame * 16)) {
                break;
            }
        }
        assert!(!motion.is_active());
        assert_eq!(handle.offset().y, px(-700.));
    }
}
