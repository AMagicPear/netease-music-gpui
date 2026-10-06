use gpui::{
    Anchor, AnyElement, App, BoxShadow, ElementId, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, RenderOnce, StyleRefinement, Styled, Window, anchored,
    deferred, div, px, relative,
};
use gpui_kit::base::{StyledExt as _, Theme};

use super::LAYER_POPOVER;

/// 弹层离窗口边缘至少留这么多。
///
/// 这是自己实现弹层的主要理由：gpui-kit 的 Popup 把限位写死成 8px
/// （crates/base/src/popup.rs 的 WINDOW_MARGIN），没有对外开关。
const EDGE_MARGIN: Pixels = px(28.);
/// 弹层圆角。
const SURFACE_RADIUS: Pixels = px(10.);

/// 点击触发、点外面关闭的轻量弹层。
///
/// 定位交给 GPUI 的 `anchored()`：先按 anchor 把弹层钉在触发元素上，
/// 只有越界时才整体推回窗口内，限位就是 EDGE_MARGIN。
#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    /// 弹层与触发元素之间的间隙。
    offset: Pixels,
    trigger: Option<AnyElement>,
    /// 弹层内容。和 div 一样用 `.child()` 往里放；底色、圆角、投影由本组件套好，
    /// 传进来的元素只管内容和尺寸。
    children: Vec<AnyElement>,
}

impl Popover {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            anchor: Anchor::BottomCenter,
            offset: px(0.),
            trigger: None,
            children: Vec::new(),
        }
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn offset(mut self, offset: Pixels) -> Self {
        self.offset = offset;
        self
    }

    pub fn trigger(mut self, trigger: impl IntoElement + 'static) -> Self {
        self.trigger = Some(trigger.into_any_element());
        self
    }
}

/// 让调用方能像给 div 那样给弹层容器加样式（`flex_none()` 之类）。
impl Styled for Popover {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// 弹层内容走 `.child()`，用法和往 div 里塞孩子一样：
/// 外观不用调用方操心，尺寸和留白由传进来的元素自己定。
impl ParentElement for Popover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // 开合状态按元素 id 存在窗口里，宿主 view 不需要为它准备字段。
        let open = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let view_id = window.current_view();
        // 渲染时读到的值：下面靠它判断"状态有没有被别的监听先改过"。
        let is_open = *open.read(cx);

        let mut root = div()
            .id(self.id.clone())
            .relative()
            .refine_style(&self.style)
            // 触发：按下即切换。点在外面的关闭监听会在捕获阶段先把它关掉，
            // 这里比对渲染时的值，避免"刚关上又立刻打开"。
            .on_mouse_down(MouseButton::Left, {
                let open = open.clone();
                move |_: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if *open.read(cx) == is_open {
                        open.update(cx, |open, _| *open = !*open);
                        cx.notify(view_id);
                    }
                }
            });
        if let Some(trigger) = self.trigger {
            root = root.child(trigger);
        }

        if !is_open || self.children.is_empty() {
            return root.into_any_element();
        }
        let colors = Theme::global(cx).tokens.colors;

        root.child(
            // 零尺寸锚点：水平居中，落在触发元素上方 offset 处。
            div()
                .absolute()
                .top(-self.offset)
                .left(relative(0.5))
                .size(px(0.))
                .child(
                    deferred(
                        anchored()
                            .anchor(self.anchor)
                            // 越界才推回窗口内，限位可配——自己实现的意义就在这。
                            .snap_to_window_with_margin(EDGE_MARGIN)
                            .child(
                                div()
                                    .id("popover-content")
                                    // 从命中测试中遮住后方元素，hover 样式也不会穿透。
                                    .occlude()
                                    // 弹层的外观固定在这里，调用方给的元素只撑尺寸。
                                    .bg(colors.surface)
                                    .rounded(SURFACE_RADIUS)
                                    .shadow(vec![
                                        BoxShadow::new(
                                            px(0.),
                                            px(2.),
                                            colors.foreground.alpha(0.1),
                                        )
                                        .blur_radius(px(6.)),
                                    ])
                                    // 弹层最后画，冒泡阶段最先拿到事件：在这里掐断，
                                    // 点面板就不会穿透到下面的进度条或歌曲列表。
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        |_: &MouseDownEvent, _, cx| {
                                            cx.stop_propagation();
                                        },
                                    )
                                    // 点在任何不属于弹层的地方都关闭。
                                    .on_mouse_down_out({
                                        let open = open.clone();
                                        move |_: &MouseDownEvent, _, cx| {
                                            open.update(cx, |open, _| *open = false);
                                            cx.notify(view_id);
                                        }
                                    })
                                    .children(self.children),
                            ),
                    )
                    .with_priority(LAYER_POPOVER),
                ),
        )
        .into_any_element()
    }
}
