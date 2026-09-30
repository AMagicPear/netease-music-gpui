use std::{cell::Cell, rc::Rc};

use gpui::prelude::{FluentBuilder, StatefulInteractiveElement};
use gpui::*;

use crate::theme::Theme;

const MIN_SIDEBAR_WIDTH: Pixels = px(204.);
const MAX_SIDEBAR_WIDTH: Pixels = px(627.);

#[derive(Clone, Copy, PartialEq, Eq)]
enum ContentPage {
    Discover,
    Playlists,
}

impl ContentPage {
    fn id(self) -> &'static str {
        match self {
            Self::Discover => "discover",
            Self::Playlists => "playlists",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Discover => "发现音乐",
            Self::Playlists => "我的歌单",
        }
    }
}

pub struct MainContent {
    theme: Theme,
    sidebar_width: Pixels,
    active_page: ContentPage,
    window_move_pending: bool,
}

impl MainContent {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            sidebar_width: MIN_SIDEBAR_WIDTH,
            active_page: ContentPage::Discover,
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

fn page_nav_item(
    page: ContentPage,
    active_page: ContentPage,
    theme: Theme,
    cx: &mut Context<MainContent>,
) -> impl IntoElement {
    div()
        .id(page.id())
        .w_full()
        .h(px(40.))
        .flex()
        .items_center()
        .px(px(12.))
        .rounded(px(4.))
        .text_size(px(14.))
        .text_color(if page == active_page {
            theme.primary
        } else {
            theme.black5
        })
        .when(page == active_page, |this| this.bg(rgba(0xfc3d491a)))
        .when(page != active_page, |this| {
            this.hover(|style| style.bg(theme.black10))
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.active_page = page;
            cx.notify();
        }))
        .child(page.title())
}

impl Render for MainContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
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
                    .flex_none()
                    .flex()
                    .flex_col()
                    .bg(rgba(0x28324808))
                    .child(page_header("left-page-header", cx))
                    .child(
                        div()
                            .id("left-page-content")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .p(px(12.))
                            .child(page_nav_item(
                                ContentPage::Discover,
                                self.active_page,
                                theme,
                                cx,
                            ))
                            .child(page_nav_item(
                                ContentPage::Playlists,
                                self.active_page,
                                theme,
                                cx,
                            )),
                    ),
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
                            .child(self.active_page.title()),
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
