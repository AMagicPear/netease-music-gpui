use gpui::prelude::{FluentBuilder, StatefulInteractiveElement};
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};
use gpui_kit::component::TitleBar;

use super::{ContentPage, LIBRARY_PAGES, MAIN_PAGES};
use crate::state::{library::MusicLibrary, playlist::Playlist};
use crate::theme::IconSize;

/// 选中项变化时发出的事件，`MainContent` 订阅它来重新渲染右侧内容。
pub(super) struct SidebarChanged;

pub(super) struct SidebarPage {
    active_page: ContentPage,
    library: Entity<MusicLibrary>,
    _library_subscription: Subscription,
}

impl EventEmitter<SidebarChanged> for SidebarPage {}

impl SidebarPage {
    pub(super) fn new(library: Entity<MusicLibrary>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&library, |_, _, cx| cx.notify());
        Self {
            active_page: ContentPage::Recommend,
            library,
            _library_subscription: subscription,
        }
    }

    pub(super) fn active_page(&self) -> ContentPage {
        self.active_page
    }
}

/// 顶部 logo 区域，同时充当窗口拖动手柄。
fn page_header(colors: ColorTokens) -> impl IntoElement {
    TitleBar::new()
        .h(px(72.))
        .w_full()
        .pl(px(0.))
        .border_b_0()
        .bg(rgba(0x00000000))
        .child(
            div().size_full().px(px(18.)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pt_10()
                    .left_1()
                    .child(
                        div()
                            .size(px(24.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(colors.primary)
                            .text_color(colors.primary_foreground)
                            .child(
                                svg()
                                    .path("icons/logo/logo.svg")
                                    .size(px(27.))
                                    .flex_none()
                                    .text_color(colors.primary_foreground),
                            ),
                    )
                    .child(
                        svg()
                            .path("icons/logo/logo_text.svg")
                            .w(px(101.))
                            .h(px(19.))
                            .text_color(colors.foreground),
                    ),
            ),
        )
}

/// 侧边栏每一行的公共样式：撑满宽度、固定高度、内部水平排列。
fn sidebar_row(id: impl Into<ElementId>, height: Pixels) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .h(height)
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .rounded(px(8.))
}

/// 导航和歌单分组标题。
fn section_heading(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    colors: ColorTokens,
) -> impl IntoElement {
    sidebar_row(id, px(36.))
        .text_size(px(12.))
        .text_color(colors.muted_foreground)
        .child(text.into())
}

/// 分组之间的分隔线。
fn divider(colors: ColorTokens) -> impl IntoElement {
    div().w_full().h(px(1.)).my(px(12.)).bg(colors.border)
}

/// 单个导航项：选中时用主题色高亮，未选中时悬停才显示背景。
fn page_nav_item(
    page: ContentPage,
    active_page: ContentPage,
    colors: ColorTokens,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    let active = page == active_page;

    Button::new(page.id())
        .w_full()
        .h(px(36.))
        .gap(px(8.))
        .p(px(8.))
        .rounded(px(8.))
        // Button 默认把内容居中，导航项需要图标 + 文字左对齐，所以显式覆盖。
        .justify_start()
        .text_size(px(14.))
        .text_color(colors.foreground)
        // 选中态交给 Button 的语义状态，而不是手工叠背景色。
        .selected(active)
        .styles(|styles| {
            styles.selected(|style| {
                style
                    .bg(colors.primary)
                    .text_color(colors.primary_foreground)
                    .font_weight(FontWeight::MEDIUM)
            })
        })
        .when(!active, |this| this.hover(|style| style.bg(colors.accent)))
        .child(
            svg()
                .path(page.icon())
                .size(IconSize::Small.pixels())
                .flex_none()
                .text_color(if active {
                    colors.primary_foreground
                } else {
                    colors.muted_foreground
                }),
        )
        .child(page.title())
        .when(page.has_notification(), |this| {
            this.child(
                div()
                    .size(px(5.))
                    .rounded_full()
                    .bg(colors.primary)
                    .flex_none(),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.active_page = page;
            cx.notify();
            cx.emit(SidebarChanged);
        }))
}

/// 按顺序渲染一组导航项。
fn navigation(
    pages: &[ContentPage],
    active_page: ContentPage,
    colors: ColorTokens,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    // 用循环而不是 `.children(pages.iter().map(...))`：闭包里借用 cx 会
    // 让返回的匿名类型带上 cx 的生命周期，无法从 FnMut 闭包中逃出去。
    let mut group = div().flex().flex_col().gap(px(4.));
    for page in pages {
        group = group.child(page_nav_item(*page, active_page, colors, cx));
    }
    group
}

fn main_navigation(
    active_page: ContentPage,
    colors: ColorTokens,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    navigation(&MAIN_PAGES, active_page, colors, cx)
}

fn library_navigation(
    active_page: ContentPage,
    colors: ColorTokens,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    let mut group = div().flex().flex_col().gap(px(4.)).child(section_heading(
        "library-heading",
        "我的",
        colors,
    ));
    for page in LIBRARY_PAGES {
        group = group.child(page_nav_item(page, active_page, colors, cx));
    }
    group
}

fn created_playlist(playlist: &Playlist, colors: ColorTokens) -> impl IntoElement {
    sidebar_row(("created-playlist", playlist.id), px(42.))
        .hover(|style| style.bg(colors.accent))
        .when_some(playlist.cover_img_url.clone(), |row, cover| {
            row.child(img(cover).size(px(32.)).rounded(px(4.)).flex_none())
        })
        .child(
            div()
                .flex_1()
                // 配合 flex_1 让长歌单名截断而不是把行撑宽
                .min_w(px(0.))
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(colors.secondary_foreground)
                .child(playlist.name.clone()),
        )
}

fn created_playlists(library: &MusicLibrary, colors: ColorTokens) -> Div {
    let playlists: Vec<_> = library
        .playlists
        .iter()
        .filter(|playlist| {
            playlist.creator.user_id == library.user_id && playlist.special_type != 5
        })
        .collect();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(section_heading(
            "created-playlists-heading",
            format!("创建的歌单 {}", playlists.len()),
            colors,
        ))
        .children(
            playlists
                .iter()
                .map(|playlist| created_playlist(playlist, colors)),
        )
}

impl Render for SidebarPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let playlists = created_playlists(self.library.read(cx), colors);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.foreground.alpha(0.03))
            .child(page_header(colors))
            // 头部固定，剩下的是唯一可滚动的区域；
            // min_h(0) 是 flex 子项能正确触发滚动的关键。
            .child(
                div()
                    .id("left-page-content")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .p(px(18.))
                            .child(main_navigation(self.active_page, colors, cx))
                            .child(divider(colors))
                            .child(library_navigation(self.active_page, colors, cx))
                            .child(divider(colors))
                            .child(playlists),
                    ),
            )
    }
}
