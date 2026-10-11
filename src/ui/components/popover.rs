//! 浮层组件：锚定触发器的 popover 与右侧抽屉共用的表面外观。
//!
//! [`SurfacePopover`] 把音质弹窗、账号弹层同款的「表面 + 动画 + 交互」收敛成
//! 一个组件（对齐 gpui-kit 组件的实现风格：`RenderOnce` + `Styled` 精化 +
//! `ParentElement` 装内容）；开合状态与关闭回调归调用方（view）持有，组件只管画。
//! 播放队列抽屉是 absolute 定位的自渲染面板，形态不同，只共用这里的圆角与投影。

use std::{rc::Rc, time::Duration};

use gpui::{
    Anchor, AnyElement, App, BoxShadow, ElementId, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Pixels, RenderOnce, StyleRefinement, Styled,
    Window, div, ease_out_quint, px,
};
use gpui_kit::base::{Popup, Presence, StyledExt as _, Theme, Transition};
use smallvec::SmallVec;

/// 浮层统一圆角。播放队列抽屉复用它（只取左侧两角），所有浮层的圆角量始终一致。
pub(in crate::ui) const SURFACE_RADIUS: Pixels = px(10.);

/// 浮层统一投影。集中在这里，改一次所有浮层一起生效。
pub(in crate::ui) fn surface_shadow(colors: crate::ui::components::ColorTokens) -> BoxShadow {
    BoxShadow::new(px(0.), px(2.), colors.foreground.alpha(0.1)).blur_radius(px(6.))
}

/// 锚定浮层统一的进出场时长与浮起距离（官方浮层的轻快手感）。
const SURFACE_MOTION: Duration = Duration::from_millis(150);
const SURFACE_TRAVEL: Pixels = px(6.);

/// 锚定触发器的浮层：统一的表面外观（surface 底色、圆角、投影）、
/// 进出场动画（下方浮起 + 淡入）以及「点外面收起、点里面不冒泡」的交互。
///
/// 开合状态（`open`）和关闭回调归调用方持有——关闭回调用
/// [`Self::on_mouse_down_out`] 传入（通常就是把 `open` 置回 `false`）。
/// 表面底色、文字色、尺寸等微调走 [`Styled`] 精化，最终覆盖在表面容器上
/// （深色面板就是用 `.bg()` 精化换掉的默认浅底）。
#[derive(IntoElement)]
pub(in crate::ui) struct SurfacePopover {
    id: ElementId,
    open: bool,
    trigger: AnyElement,
    anchor: Anchor,
    offset: Pixels,
    on_mouse_down_out: Option<Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>>,
    children: SmallVec<[AnyElement; 1]>,
    style: StyleRefinement,
}

impl SurfacePopover {
    pub fn new(id: impl Into<ElementId>, open: bool, trigger: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            open,
            trigger: trigger.into_any_element(),
            anchor: Anchor::BottomCenter,
            offset: px(12.),
            on_mouse_down_out: None,
            children: SmallVec::new(),
            style: StyleRefinement::default(),
        }
    }

    /// 弹层贴着触发器的哪一边弹出。
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// 与触发器锚点方向的间距。
    pub fn offset(mut self, offset: Pixels) -> Self {
        self.offset = offset;
        self
    }

    /// 点弹层外面时回调（捕获阶段触发，点在弹层内不会到这）。
    pub fn on_mouse_down_out(
        mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_mouse_down_out = Some(Rc::new(listener));
        self
    }
}

impl ParentElement for SurfacePopover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SurfacePopover {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for SurfacePopover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        // 进出场共用一条 Presence：`should_render()` 在退场期间仍为真，
        // 弹层会继续挂着直到动画放完才卸载——退场动画能存在全靠这一步。
        let presence = Presence::new((self.id.clone(), "motion"), self.open)
            .transition(Transition::new(SURFACE_MOTION).ease(ease_out_quint()))
            .sample(window, cx);
        let popup = Popup::new(self.id.clone(), self.trigger)
            .anchor(self.anchor)
            .offset(self.offset)
            .flex_none();
        if !presence.should_render() {
            return popup.into_any_element();
        }

        // 浮现方向跟锚点走：弹层在触发器下方（Top*）从锚点向下出现，
        // 在上方（Bottom*）则向上浮起；退场反向收回。
        let travel = match self.anchor {
            Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => -SURFACE_TRAVEL,
            _ => SURFACE_TRAVEL,
        };

        let mut surface = div()
            .id((self.id, "surface"))
            .bg(colors.surface)
            .rounded(SURFACE_RADIUS)
            // 阴影永远是墨色的：深色面板上不该出现白色光晕。
            .shadow(vec![surface_shadow(colors)])
            .text_color(colors.foreground)
            .opacity(presence.progress)
            .top(travel * (1. - presence.progress))
            .refine_style(&self.style)
            .children(self.children);
        if let Some(on_out) = self.on_mouse_down_out {
            surface = surface.on_mouse_down_out(move |event, window, cx| on_out(event, window, cx));
        }
        // 弹层是命中区最上层的延迟绘制后代；不吞掉这次按下，里面的点击会
        // 继续冒到页面根上（如播放栏根的"选音质顺手收起歌词页"）。
        surface = surface.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        popup.content(surface).into_any_element()
    }
}
