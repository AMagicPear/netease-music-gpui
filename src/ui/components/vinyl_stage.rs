//! 唱片舞台：正方形圆环 + 唱片 + 唱臂。唱片本体由 [`PlayerBar`](super::PlayerBar) 生成后传入。
//! 唱臂角度由 [`Tonearm`] 状态机按播放状态与切歌事件推进。

use std::time::Duration;

use gpui::*;
use gpui_kit::base::{MotionStatus, Transition, transition_with_status};

/// 唱臂素材按 340 基准绘制：画布 348，抬起前偏移 80。全部按唱片直径等比换算。
const TONEARM_SCALE: f32 = 348. / 340.;
const TONEARM_LIFT: f32 = 80. / 340.;

/// 唱臂起落状态机。
///
/// 切歌必须先完整抬起再落下，恢复播放不能打断正在进行的抬起；
/// 因此新目标只在当前动作结束后才被接受。
#[derive(Default)]
pub struct Tonearm {
    revision: Option<u64>,
    lowered: bool,
    lift_pending: bool,
}

impl Tonearm {
    pub fn angle(
        &mut self,
        playing: bool,
        revision: u64,
        window: &mut Window,
        cx: &mut App,
    ) -> f32 {
        match self.revision.replace(revision) {
            None => self.lowered = playing,
            Some(previous) if previous != revision => self.lift_pending = true,
            _ => {}
        }
        let sample = |lowered, window: &mut Window, cx: &mut App| {
            transition_with_status(
                "album-tonearm-angle",
                if lowered { 0. } else { -33. },
                Transition::new(Duration::from_millis(350)).ease(ease_in_out),
                window,
                cx,
            )
        };
        let motion = sample(self.lowered, window, cx);
        if matches!(motion.status, MotionStatus::Idle | MotionStatus::Finished) {
            // 当前动作完整结束后才接受新目标；切歌必须先到达抬起位置。
            if !self.lowered {
                self.lift_pending = false;
            }
            let lowered = playing && !self.lift_pending;
            if self.lowered != lowered {
                self.lowered = lowered;
                return sample(lowered, window, cx).value;
            }
        }
        motion.value
    }
}

/// 唱片 + 唱臂的正方形舞台，`side` 为整个舞台的边长。
/// 唱臂位置写成容器边长的百分比，不再先算 scale/arm_side 再乘像素。
pub fn vinyl_stage(side: f32, disc: Option<Div>, arm_angle: f32) -> impl IntoElement {
    let angle = arm_angle.to_radians();
    div()
        .relative()
        .size(px(side))
        .flex_none()
        .child(
            div()
                .size_full()
                .debug_selector(|| "album-vinyl-ring".into())
                .rounded_full()
                .border_1()
                .border_color(white().alpha(0.08))
                .bg(white().alpha(0.025))
                .flex()
                .items_center()
                .justify_center()
                .children(disc),
        )
        .child(
            img("images/vinylHandle.svg")
                .debug_selector(|| "album-tonearm".into())
                .absolute()
                .left(relative(0.5 - TONEARM_SCALE / 2.))
                .top(relative(-TONEARM_LIFT - TONEARM_SCALE / 2.))
                .w(relative(TONEARM_SCALE))
                .h(relative(TONEARM_SCALE))
                .with_transformation(Transformation::rotate(radians(angle))),
        )
}
