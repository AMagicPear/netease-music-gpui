use std::{cell::Cell, rc::Rc};

use gpui::prelude::StatefulInteractiveElement;
use gpui::*;

use super::sidebar_page::{SidebarChanged, SidebarPage};
use crate::theme::Theme;

const MIN_SIDEBAR_WIDTH: Pixels = px(204.);
const MAX_SIDEBAR_WIDTH: Pixels = px(627.);

pub struct MainContent {
    theme: Theme,
    sidebar_width: Pixels,
    sidebar: Entity<SidebarPage>,
    _sidebar_subscription: Subscription,
    window_move_pending: bool,
}

impl MainContent {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let sidebar = cx.new(|_| SidebarPage::new(theme));
        let sidebar_subscription = cx.subscribe(&sidebar, |_, _, _: &SidebarChanged, cx| {
            cx.notify();
        });

        Self {
            theme,
            sidebar_width: MIN_SIDEBAR_WIDTH,
            sidebar,
            _sidebar_subscription: sidebar_subscription,
            window_move_pending: false,
        }
    }
}

fn page_header(id: &'static str, cx: &mut Context<MainContent>) -> impl IntoElement {
    div()
        .id(id)
        .h(px(72.))
        .w_full()
        .flex_none()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.window_move_pending = true),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.window_move_pending = false),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.window_move_pending = false),
        )
        .on_mouse_down_out(cx.listener(|this, _, _, _| this.window_move_pending = false))
        .on_mouse_move(cx.listener(|this, _, window, _| {
            if this.window_move_pending {
                this.window_move_pending = false;
                window.start_window_move();
            }
        }))
}

impl Render for MainContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
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
                    .child(page_header("right-page-header", cx))
                    .child(
                        div()
                            .id("right-page-content")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .p(px(24.))
                            .text_size(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.black1)
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
