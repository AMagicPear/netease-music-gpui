//! 全局提示条：浮在窗口正中的悬浮胶囊。
//!
//! 它是全应用唯一的一条提示通道：任何组件只要说一句「心动模式开启中」，
//! 不管当时在哪个页面、哪个窗口层，都能被看见。做法是一个全局实体
//! （[`ToastHandle`]）持有当前文案，窗口层把它画在最上面。

use std::time::Duration;

use gpui::{
    App, Context, Entity, Hsla, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, div, px, rgb,
};
use gpui_kit::base::{Presence, Theme, Transition};

/// 胶囊高度。官方是 44px，一行字撑不满，就靠这个高度把它做成一个「块」。
const TOAST_HEIGHT: f32 = 44.;
/// 胶囊左右留白。文字短的提示也不会缩成一个圆点。
const TOAST_PADDING: f32 = 20.;
/// 停留时长：够读完一句话，又不至于挡住界面。
const TOAST_DURATION: Duration = Duration::from_secs(2);
/// 进出场时长。
const TOAST_MOTION: Duration = Duration::from_millis(180);
/// 出现时从上方多少像素落下来。
const TOAST_TRAVEL: f32 = 12.;

/// 胶囊底色。不透明的深灰蓝：压在浅色界面上也稳，不会像半透明那样透出底下的字。
fn toast_background() -> Hsla {
    rgb(0x2d2d38).into()
}

/// 当前提示。全局唯一，组件自己渲染它。
pub struct Toast {
    /// 文案。收起来之后仍然留着，退场动画要靠它把胶囊画完。
    message: Option<SharedString>,
    /// 是否正在显示。收起来时先把它置 false，文案等动画走完再丢。
    visible: bool,
    /// 每条提示一个号：后一条来了，前一条的定时器就作废。
    generation: u64,
}

/// 全局句柄。放进 [`gpui::Global`] 后，任意地方都能拿到这个实体。
pub struct ToastHandle(pub Entity<Toast>);

impl gpui::Global for ToastHandle {}

impl Toast {
    pub fn new() -> Self {
        Self {
            message: None,
            visible: false,
            generation: 0,
        }
    }

    /// 任意地方弹一条提示。
    pub fn show(cx: &mut App, message: impl Into<SharedString>) {
        Self::global(cx).update(cx, |toast, cx| toast.present(message, cx));
    }

    pub fn global(cx: &App) -> Entity<Toast> {
        cx.global::<ToastHandle>().0.clone()
    }

    fn present(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.message = Some(message.into());
        self.visible = true;
        self.generation += 1;
        let generation = self.generation;
        cx.notify();
        // 到点自己收掉；期间又来一条新提示，这条的定时器就作废。
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            let _ = this.update(cx, |toast, cx| {
                if toast.generation == generation {
                    // 只把「显示中」翻掉：文案要留着画完退场动画。
                    toast.visible = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for Toast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        // 进出场共用一条 Presence：退场期间仍返回 true，胶囊才能淡出去再卸载。
        let presence = Presence::new("toast", self.visible)
            .transition(Transition::new(TOAST_MOTION))
            .sample(window, cx);
        if !presence.should_render() {
            return div().into_any_element();
        }
        let Some(message) = self.message.clone() else {
            return div().into_any_element();
        };
        // 一个进度同时驱动透明度和位置：出现时从上方一点淡入正中，
        // 消失时原路退回——同一个式子正着反着都能用。
        let travel = TOAST_TRAVEL * (1. - presence.progress);
        let height = f32::from(window.viewport_size().height);
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    // 落在整窗正中，再按进度往上让一点。
                    .top(px((height - TOAST_HEIGHT) / 2. - travel))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .h(px(TOAST_HEIGHT))
                            .px(px(TOAST_PADDING))
                            .rounded_full()
                            .bg(toast_background())
                            .text_color(colors.primary_foreground)
                            .text_size(px(14.))
                            .flex()
                            .items_center()
                            .opacity(presence.progress)
                            .child(message),
                    ),
            )
            .into_any_element()
    }
}
