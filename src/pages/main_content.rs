use std::{cell::Cell, rc::Rc};

use gpui::prelude::StatefulInteractiveElement;
use gpui::*;

use super::sidebar_page::{SidebarChanged, SidebarPage};
use crate::components::{WindowDragState, window_drag_region};
use crate::theme::IconSize;
use gpui_kit::base::input::{Input, InputState};
use gpui_kit::base::{Button, ColorTokens, Theme as BaseTheme};

const MIN_SIDEBAR_WIDTH: Pixels = px(204.);
const MAX_SIDEBAR_WIDTH: Pixels = px(627.);
const HEADER_HEIGHT: Pixels = px(72.);
const HEADER_TOP_PADDING: Pixels = px(34.);
const HEADER_SIDE_GUTTER: Pixels = px(40.);

pub struct MainContent {
    sidebar_width: Pixels,
    sidebar: Entity<SidebarPage>,
    search_input: Entity<InputState>,
    _sidebar_subscription: Subscription,
    window_move_pending: bool,
}

impl MainContent {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sidebar = cx.new(|_| SidebarPage::new());
        let sidebar_subscription = cx.subscribe(&sidebar, |_, _, _: &SidebarChanged, cx| {
            cx.notify();
        });
        let search_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("搜索音乐");
            state.set_editor_style(gpui_kit::base::input::InputEditorStyle {
                selection: BaseTheme::global(cx).tokens.colors.selection,
                ..Default::default()
            });
            state
        });

        Self {
            sidebar_width: MIN_SIDEBAR_WIDTH,
            sidebar,
            search_input,
            _sidebar_subscription: sidebar_subscription,
            window_move_pending: false,
        }
    }
}

impl WindowDragState for MainContent {
    fn window_move_pending_mut(&mut self) -> &mut bool {
        &mut self.window_move_pending
    }
}

/// 搜索框：左边放大镜图标，右边文本输入框。
fn search_box(input: Entity<InputState>, colors: ColorTokens) -> impl IntoElement {
    div()
        .w(px(258.))
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .border_1()
        .border_color(colors.border)
        .rounded_lg()
        .text_size(px(14.))
        .text_color(colors.foreground)
        .child(
            svg()
                .path("icons/search.svg")
                .size(px(20.))
                .flex_none()
                .text_color(colors.muted_foreground),
        )
        .child(Input::new(&input))
}

fn page_header(
    id: &'static str,
    colors: ColorTokens,
    search_input: Entity<InputState>,
    cx: &mut Context<MainContent>,
) -> impl IntoElement {
    div()
        .id(id)
        .h(HEADER_HEIGHT)
        .w_full()
        .flex_none()
        .relative()
        .pt(HEADER_TOP_PADDING)
        .child(
            window_drag_region("right-header-top-drag-region", cx)
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(HEADER_TOP_PADDING),
        )
        .child(
            div()
                .w_full()
                .h(HEADER_HEIGHT - HEADER_TOP_PADDING)
                .flex()
                .justify_between()
                .items_center()
                .child(
                    window_drag_region("right-header-left-drag-region", cx)
                        .w(HEADER_SIDE_GUTTER)
                        .h_full()
                        .flex_none(),
                )
                .child(
                    div()
                        .h_9()
                        .flex()
                        .gap_2()
                        // 返回按钮
                        .child(
                            Button::new("back-button")
                                .w_7()
                                .border_1()
                                .border_color(colors.border)
                                .rounded_lg()
                                .hover(|style| style.bg(colors.accent))
                                .child(
                                    svg()
                                        .path("icons/backward.svg")
                                        .size(px(11.))
                                        .text_color(colors.secondary_foreground),
                                ),
                        )
                        // 搜索框
                        .child(search_box(search_input, colors)),
                )
                .child(
                    window_drag_region("right-header-drag-region", cx)
                        .flex_1()
                        .h_full(),
                )
                // 右侧的头像名称和按钮
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .text_size(px(13.))
                                .text_color(colors.foreground.alpha(0.7))
                                .child(
                                    img("/Users/amagicpear/Pictures/Perry Origin Character/IMG_20240601_133150.jpeg")
                                        .size(px(28.))
                                        .rounded_full()
                                        .flex_none(),
                                )
                                .child("一只会魔法的梨")
                                .child(
                                    img("icons/vip-level.svg")
                                        .w(px(48.))
                                        .h(px(16.))
                                        .flex_none(),
                                )
                                .child(
                                    svg()
                                        .path("icons/unfold.svg")
                                        .size(px(20.))
                                        .flex_none()
                                        .text_color(colors.foreground.alpha(0.6)),
                                ),
                        )
                        .child(
                            svg()
                                .path("icons/message.svg")
                                .size(IconSize::Small.pixels())
                                .flex_none()
                                .text_color(colors.foreground.alpha(0.6)),
                        )
                        .child(
                            svg()
                                .path("icons/setting.svg")
                                .size(IconSize::Small.pixels())
                                .flex_none()
                                .text_color(colors.foreground.alpha(0.6)),
                        )
                        .child(
                            svg()
                                .path("icons/skin.svg")
                                .size(IconSize::Small.pixels())
                                .flex_none()
                                .text_color(colors.foreground.alpha(0.6)),
                        ),
                )
                .child(
                    window_drag_region("right-header-right-drag-region", cx)
                        .w(HEADER_SIDE_GUTTER)
                        .h_full()
                        .flex_none(),
                ),
        )
}

impl Render for MainContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = BaseTheme::global(cx).tokens.colors;
        let active_page = self.sidebar.read(cx).active_page();
        let drag_offset = Rc::new(Cell::new(px(0.)));

        div()
            .w_full()
            .flex_1()
            .min_h(px(0.))
            .relative()
            .flex()
            .child(
                div()
                    .w(self.sidebar_width)
                    .h_full()
                    .flex_none()
                    .child(self.sidebar.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .child(page_header(
                        "right-page-header",
                        colors,
                        self.search_input.clone(),
                        cx,
                    ))
                    .child(
                        div()
                            .id("right-page-content")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .p(px(24.))
                            .text_size(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.foreground)
                            .child(active_page.title()),
                    ),
            )
            .child(
                div()
                    .id("main-splitter")
                    .absolute()
                    .left(self.sidebar_width - px(4.))
                    .top(px(0.))
                    .h_full()
                    .w(px(8.))
                    .cursor_col_resize()
                    .occlude()
                    .on_drag(drag_offset.clone(), {
                        move |_, offset, _, cx| {
                            drag_offset.set(offset.x);
                            cx.new(|_| ResizeDragPreview)
                        }
                    })
                    .on_drag_move(cx.listener(
                        move |this, event: &DragMoveEvent<Rc<Cell<Pixels>>>, _, cx| {
                            this.sidebar_width = (event.event.position.x - event.drag(cx).get()
                                + px(4.))
                            .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                            cx.notify();
                        },
                    )),
            )
    }
}

struct ResizeDragPreview;

impl Render for ResizeDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.))
    }
}
