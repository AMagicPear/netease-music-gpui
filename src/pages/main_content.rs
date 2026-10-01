use std::{cell::Cell, collections::HashMap, rc::Rc};

use gpui::prelude::StatefulInteractiveElement;
use gpui::*;

use super::ContentPage;
use super::sidebar_page::{SidebarChanged, SidebarPage};
use crate::state::user::UserProfile;
use crate::theme::IconSize;
use gpui_kit::base::input::{Input, InputState};
use gpui_kit::base::{Button, ColorTokens, Theme};
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::{Sizable, TitleBar};

const MIN_SIDEBAR_WIDTH: Pixels = px(204.);
const MAX_SIDEBAR_WIDTH: Pixels = px(627.);
/// 搜索框的理想宽度
const SEARCH_BOX_WIDTH: Pixels = px(258.);
/// 搜索框的压缩下限
const SEARCH_BOX_MIN_WIDTH: Pixels = px(40.);

pub struct MainContent {
    sidebar_width: Pixels,
    sidebar: Entity<SidebarPage>,
    search_input: Entity<InputState>,
    user_profile: Entity<UserProfile>,
    /// 每个导航项对应一个页面 View，一次创建后长期持有。
    pages: HashMap<ContentPage, AnyView>,
    _sidebar_subscription: Subscription,
    _user_profile_subscription: Subscription,
}

impl MainContent {
    pub fn new(
        window: &mut Window,
        user_profile: Entity<UserProfile>,
        cx: &mut Context<Self>,
    ) -> Self {
        let sidebar = cx.new(|_| SidebarPage::new());
        let sidebar_subscription = cx.subscribe(&sidebar, |_, _, _: &SidebarChanged, cx| {
            cx.notify();
        });
        let search_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("搜索音乐");
            state.set_editor_style(gpui_kit::base::input::InputEditorStyle {
                selection: Theme::global(cx).tokens.colors.selection,
                ..Default::default()
            });
            state
        });

        // 页面在这里全部建好；切走再切回仍是同一个 View 实例。
        let pages = ContentPage::all()
            .map(|page| (page, page.build(user_profile.clone(), cx)))
            .collect();
        let user_profile_subscription = cx.observe(&user_profile, |_, _, cx| cx.notify());

        Self {
            sidebar_width: MIN_SIDEBAR_WIDTH,
            sidebar,
            search_input,
            user_profile,
            pages,
            _sidebar_subscription: sidebar_subscription,
            _user_profile_subscription: user_profile_subscription,
        }
    }
}

/// 搜索框：左边放大镜图标，右边文本输入框，宽度可被压缩
fn search_box(input: Entity<InputState>, colors: ColorTokens) -> impl IntoElement {
    div()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .h_9()
        .w(SEARCH_BOX_WIDTH)
        .min_w(SEARCH_BOX_MIN_WIDTH)
        .ml_2()
        .mr_2()
        .flex_shrink(1.)
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
                .text_color(colors.muted_foreground)
                .hover(|style| style.text_color(colors.foreground))
                .id("search-box-icon"),
        )
        .child(Input::new(&input))
}

fn hover_icon(id: &'static str, path: &'static str, colors: ColorTokens) -> impl IntoElement {
    svg()
        .path(path)
        .size(IconSize::Small.pixels())
        .ml(px(10.))
        .flex_none()
        .text_color(colors.foreground.alpha(0.6))
        .hover(|style| style.text_color(colors.foreground))
        .id(id)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn page_header(
    colors: ColorTokens,
    search_input: Entity<InputState>,
    user_name: String,
    avatar_path: String,
) -> impl IntoElement {
    TitleBar::new()
        .h(px(72.))
        .w_full()
        .pl(px(0.))
        .border_b_0()
        .bg(colors.background)
        .child(
            div()
                .size_full()
                .pt(px(30.))
                .px(px(40.))
                .flex()
                .items_center()
                .min_w(px(0.))
                .child(
                    Button::new("back-button")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .h_9()
                        .w_7()
                        .flex_none()
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
                .child(search_box(search_input, colors))
                .child(
                    div()
                        .id("header-avatar")
                        .ml_auto()
                        .flex_none()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Avatar::new()
                                .with_size(px(28.))
                                .flex_none()
                                .src(avatar_path),
                        ),
                )
                .child(
                    div()
                        .id("header-profile-menu")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .ml(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(13.))
                        .text_color(colors.foreground.alpha(0.7))
                        .child(user_name)
                        .child(img("icons/vip-level.svg").w(px(48.)).h(px(16.)).flex_none())
                        .child(
                            svg()
                                .path("icons/unfold.svg")
                                .size(px(20.))
                                .flex_none()
                                .text_color(colors.foreground.alpha(0.6)),
                        ),
                )
                .child(hover_icon(
                    "header-message-button",
                    "icons/message.svg",
                    colors,
                ))
                .child(hover_icon(
                    "header-setting-button",
                    "icons/setting.svg",
                    colors,
                ))
                .child(hover_icon("header-skin-button", "icons/skin.svg", colors)),
        )
}

impl Render for MainContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let user_profile = self.user_profile.read(cx);
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
                        colors,
                        self.search_input.clone(),
                        user_profile.name.clone(),
                        user_profile.avatar_path.clone(),
                    ))
                    // 右侧具体页面
                    .child(
                        div()
                            .id("right-page-content")
                            .flex_1()
                            .px(px(40.))
                            .py(px(18.))
                            .overflow_y_scroll()
                            .children(self.pages.get(&active_page).cloned()),
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
