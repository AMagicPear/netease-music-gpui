//! 侧面板：全局唯一的一个「停靠窗口右缘、通栏全高」的滑入外壳。
//!
//! 它和锚定触发器的 [`super::SurfacePopover`] 交互逻辑同构（点开、点外收起、
//! Presence 进出场），只是几何不同：锚定定位表达不了「贴窗口右缘、从头部到
//! 播放栏」的摆放，所以表面在这里自绘。外壳只管画；开合状态归本实体，内容是
//! 挂进来的子视图（播放列表 / 消息），点另一个入口时先收起、退场动画走完再
//! 展开新内容，观感上是两个不同的面板。

use std::time::Duration;

use gpui::*;
use gpui_kit::base::{Presence, Theme, Transition};

use super::message_panel::MessagePanel;
use super::play_queue::PlayQueue;
use super::{LAYER_POPOVER, PLAYER_BAR_HEIGHT, SURFACE_RADIUS, surface_shadow};
use crate::playback::PlaybackController;
use crate::state::account::AccountState;
use crate::state::library::MusicLibrary;
use crate::ui::shell::HEADER_HEIGHT;

/// 面板固定宽度（与官方播放列表面板一致）。
pub(in crate::ui) const PANEL_WIDTH: Pixels = px(386.);
/// 面板与上方头部、下方播放栏之间留出的间隔。
pub(in crate::ui) const PANEL_GAP: Pixels = px(12.);
/// 滑入/滑出时长。
const REVEAL_DURATION: Duration = Duration::from_millis(260);

/// 面板高度：窗口减去页头、播放栏和上下间隔。
pub(in crate::ui) fn panel_height(window: &Window) -> Pixels {
    (window.viewport_size().height
        - px(HEADER_HEIGHT)
        - px(PLAYER_BAR_HEIGHT)
        - PANEL_GAP * 2.)
    .max(px(0.))
}

/// 侧面板当前展示的内容。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SidePanelContent {
    PlayQueue,
    Messages,
}

pub struct SidePanel {
    content: SidePanelContent,
    open: bool,
    /// 收起过程中预约的下一页内容；退场动画走完后再展开它。None 表示没有
    /// 正在进行的切换（快速点击当前入口可取消预约，让面板弹回来）。
    pending_content: Option<SidePanelContent>,
    /// 触发图标的按下抑制（见 [`Self::suppress_out_close`]）。
    out_close_suppressed: bool,
    play_queue: Entity<PlayQueue>,
    messages: Entity<MessagePanel>,
}

impl SidePanel {
    pub fn new(
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        account: Entity<AccountState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let play_queue = cx.new(|cx| PlayQueue::new(playback, library, window, cx));
        let messages = cx.new(|cx| MessagePanel::new(account, cx));
        Self {
            content: SidePanelContent::PlayQueue,
            open: false,
            pending_content: None,
            out_close_suppressed: false,
            play_queue,
            messages,
        }
    }

    /// 触发图标的按下回调（capture 阶段）申请：这一次按下的 click 马上会从
    /// 松开时到达，请面板别急着在「面板外按下」里收起，开合/换内容交给 click。
    ///
    /// 面板的 `on_mouse_down_out` 注册在 capture 阶段且晚于图标（面板更晚绘制），
    /// 所以图标的申请一定先到。只在开着时生效，避免留下永远吃不掉的陈旧标志。
    pub fn suppress_out_close(&mut self) {
        if self.open {
            self.out_close_suppressed = true;
        }
    }

    /// click 语义的开合：关着就开；开着且同一内容就收；开着但换了入口就
    /// 收起后展开新内容（见 [`Self::switch_to`]）。
    pub fn toggle(&mut self, content: SidePanelContent, cx: &mut Context<Self>) {
        if self.pending_content == Some(content) {
            // 正在收起准备切到这一页，无需重复。
            return;
        }
        if self.open {
            if self.content == content {
                if self.pending_content.is_some() {
                    // 正在收起换页又点了当前页的入口：取消切换，面板弹回来。
                    self.pending_content = None;
                    self.set_open(true, cx);
                } else {
                    self.set_open(false, cx);
                }
            } else {
                self.switch_to(content, cx);
            }
        } else {
            self.pending_content = None;
            self.show_content(content, cx);
        }
    }

    /// 换内容：先收起，等退场动画走完再展开新内容，观感是两个不同的面板。
    fn switch_to(&mut self, content: SidePanelContent, cx: &mut Context<Self>) {
        self.pending_content = Some(content);
        self.set_open(false, cx);
        cx.spawn(async move |this, cx| {
            // 略长于退场时长，确保进度归零后再反向展开，避免中途反向出现回弹。
            cx.background_executor()
                .timer(REVEAL_DURATION + Duration::from_millis(40))
                .await;
            let _ = this.update(cx, |panel, cx| {
                if let Some(target) = panel.pending_content.take() {
                    panel.show_content(target, cx);
                }
            });
        })
        .detach();
    }

    fn show_content(&mut self, content: SidePanelContent, cx: &mut Context<Self>) {
        self.content = content;
        self.open = true;
        self.set_content_visible(true, cx);
        cx.notify();
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if !open {
            // 收起后内容不可见，让它们停掉「可见才做」的刷新。
            self.set_content_visible(false, cx);
        }
        cx.notify();
    }

    fn set_content_visible(&mut self, visible: bool, cx: &mut App) {
        // 只有当前展示的那页可见；换内容时旧页要停下来（订阅刷新、强制拉新）。
        self.play_queue.update(cx, |queue, cx| {
            queue.set_visible(visible && self.content == SidePanelContent::PlayQueue, cx)
        });
        self.messages.update(cx, |panel, cx| {
            panel.set_visible(visible && self.content == SidePanelContent::Messages, cx)
        });
    }
}

impl Render for SidePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 进出场都靠这一个采样：收起后仍继续渲染到进度归零，才滑得出去。
        let presence = Presence::new("side-panel", self.open)
            .transition(Transition::new(REVEAL_DURATION))
            .sample(window, cx);
        if !presence.should_render() {
            return div().into_any_element();
        }
        let width = PANEL_WIDTH.min(window.viewport_size().width);
        let offset = -(1. - presence.progress) * f32::from(width);
        let colors = Theme::global(cx).tokens.colors;
        let content = match self.content {
            SidePanelContent::PlayQueue => self.play_queue.clone().into_any_element(),
            SidePanelContent::Messages => self.messages.clone().into_any_element(),
        };

        // 与其它浮层一样延后绘制，避免被页面和进度条的 deferred 元素盖住。
        deferred(
            div()
                .id("side-panel")
                .absolute()
                // 右缘对齐窗口；被滑出时整体挪到窗口右侧之外，由窗口边界裁掉。
                .right(px(offset))
                .bottom(px(PLAYER_BAR_HEIGHT) + PANEL_GAP)
                .w(width)
                .h(panel_height(window))
                .flex()
                .flex_col()
                .text_color(colors.foreground)
                .bg(colors.surface)
                // 贴着窗口右缘，只需要把左侧两角磨圆；圆角量取自音质浮窗。
                .rounded_l(SURFACE_RADIUS)
                .shadow(vec![surface_shadow(colors)])
                // 遮住后方命中区：面板上的悬停和点击都不穿透到底下的页面。
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // 面板外的任何按下都收起——除非触发图标刚申请过抑制（点在图标上，
                // click 随后从松开时到达，由图标决定开合或原地换内容）。
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.out_close_suppressed {
                        this.out_close_suppressed = false;
                        return;
                    }
                    // 收起途中点在外面视为「只想收起来」：连切换预约也一起取消。
                    this.pending_content = None;
                    this.set_open(false, cx);
                }))
                .child(content),
        )
        .with_priority(LAYER_POPOVER)
        .into_any_element()
    }
}
