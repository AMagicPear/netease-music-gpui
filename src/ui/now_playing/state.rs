//! 「正在播放」界面的共享状态与节拍源。
//!
//! 黑胶全屏页、播放栏、进度条、音量共同响应同一次展开，还共享着封面渐变底与黑胶
//! 旋转时钟。整理前它们各自存一份布尔镜像、各自定义一条 `transition`，只靠「同一个
//! 常量 + 同一帧启动」勉强对齐；`PlayerBar.album_expanded` 还要靠 shell 回写。现在全部
//! 收进本实体：
//!
//! - [`NowPlaying::is_expanded`] 是唯一意图状态，谁都不再存副本；
//! - [`NowPlaying::backdrop`] 是共享的封面渐变底；
//! - [`NowPlaying::rotation_clock`] 是共享的黑胶旋转时钟（迷你碟与整页大碟同一份）；
//! - [`NowPlaying::reveal`] 是全应用唯一一处展开 / 收起动效定义（时长、曲线、通道 id）；
//! - 想改状态就 `update` 本实体调 `expand` / `collapse` / `toggle`，想跟随就 `observe`，
//!   事件回环与镜像 bool 一并消失。

use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Transition, transition};

use crate::ui::components::RotationClock;
use crate::ui::cover_color::{Backdrop, CoverGradient};

/// 展开 / 收起的时长。播放栏配色、封面滑出、进度条、整页滑入共用这一个值；
/// 改这里，整条链路一起变。音量不参与（见 `VolumeControl::render` 的说明）。
pub const REVEAL_DURATION: Duration = Duration::from_millis(500);

/// 展开 / 收起的动效通道 id。同一元素内必须始终是同一种值类型（这里是 `f32`）。
const REVEAL_TRANSITION_ID: &str = "now-playing-reveal";

/// 「正在播放」界面的共享状态。全应用一份：`main.rs` 创建，注入给播放栏与黑胶页。
pub struct NowPlaying {
    expanded: bool,
    backdrop: Backdrop,
    rotation_clock: Rc<Cell<RotationClock>>,
    /// 上次广播出去的渐变。见 [`Self::publish_backdrop`]。
    published_gradient: Option<CoverGradient>,
}

impl NowPlaying {
    pub fn new() -> Self {
        Self {
            expanded: false,
            backdrop: Backdrop::default(),
            rotation_clock: Rc::default(),
            published_gradient: None,
        }
    }

    pub(in crate::ui) fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// 共享的封面渐变底。是个 `Rc` 句柄，克隆出去的各处共享同一份状态：
    /// 黑胶页绘制时写、播放栏和进度条读。
    pub(in crate::ui) fn backdrop(&self) -> Backdrop {
        self.backdrop.clone()
    }

    /// 共享的黑胶旋转时钟。迷你碟与整页大碟读同一个，切换时碟面相位才连续。
    pub(in crate::ui) fn rotation_clock(&self) -> Rc<Cell<RotationClock>> {
        self.rotation_clock.clone()
    }

    pub(in crate::ui) fn expand(&mut self, cx: &mut Context<Self>) {
        self.set_expanded(true, cx);
    }

    pub(in crate::ui) fn collapse(&mut self, cx: &mut Context<Self>) {
        self.set_expanded(false, cx);
    }

    pub(in crate::ui) fn toggle(&mut self, cx: &mut Context<Self>) {
        self.set_expanded(!self.expanded, cx);
    }

    fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.expanded == expanded {
            return;
        }
        self.expanded = expanded;
        cx.notify();
    }

    /// 把共享底的最新插值广播给观察者；黑胶页在渲染时调用它。
    ///
    /// `published_gradient` 是这次整理里唯一新长出来的镜像，但它和当初删掉的
    /// `album_expanded` 不是一回事：没有任何路径能绕过本方法去写 `backdrop`，所以它
    /// 不会与真值漂移。`!=` 去重也不是可选的优化——背景渐变自带 600ms 过渡，若不去重，
    /// 播放栏 / 进度条每帧都会收到一次 notify 并重绘；去重后渐变一停通知就停。
    /// （音量也 observe 本实体，但它只认 `expanded` 的翻转，渐变广播会自己挡掉。）
    pub(in crate::ui) fn publish_backdrop(&mut self, cx: &mut Context<Self>) {
        let gradient = self.backdrop.gradient();
        if self.published_gradient != gradient {
            self.published_gradient = gradient;
            cx.notify();
        }
    }

    /// 展开 / 收起唯一的动效定义。各消费者在自己的渲染里调用它取本帧进度：
    /// id、时长、曲线都从这里出，任何地方都不该再自己 `Transition::new(...)`。
    ///
    /// 为什么是关联函数而不是 `&self` 方法：`Entity::read(&self, cx: &App) -> &T` 会把
    /// `&App` 借到返回值用完为止，而 `transition` 要 `&mut App`，两者冲突。所以调用方
    /// 必须先 `let expanded = now_playing.read(cx).is_expanded();` 取出 bool，再调这里。
    ///
    /// 另外：GPUI 的 `transition` 通道按元素路径做命名空间（`Window::use_keyed_state`），
    /// 同一个字符串在不同元素里是各自独立的通道。所以这里统一的是「参数来源」而非
    /// 「同一条通道」——正因参数同源，各处的进度永远一致。
    pub(in crate::ui) fn reveal(expanded: bool, window: &mut Window, cx: &mut App) -> f32 {
        transition(
            REVEAL_TRANSITION_ID,
            if expanded { 1_f32 } else { 0. },
            Transition::new(REVEAL_DURATION).ease(ease_out_quint()),
            window,
            cx,
        )
    }
}
