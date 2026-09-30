use gpui::prelude::{FluentBuilder, StatefulInteractiveElement};
use gpui::*;

use crate::theme::{IconSize, Theme};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ContentPage {
    Recommend,
    Featured,
    Podcast,
    Roaming,
    Following,
    FavoriteMusic,
    Recent,
    MyPodcast,
    MyCollection,
}

impl ContentPage {
    fn id(self) -> &'static str {
        match self {
            Self::Recommend => "recommend",
            Self::Featured => "featured",
            Self::Podcast => "podcast",
            Self::Roaming => "roaming",
            Self::Following => "following",
            Self::FavoriteMusic => "favorite-music",
            Self::Recent => "recent",
            Self::MyPodcast => "my-podcast",
            Self::MyCollection => "my-collection",
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Recommend => "推荐",
            Self::Featured => "精选",
            Self::Podcast => "播客",
            Self::Roaming => "漫游",
            Self::Following => "关注",
            Self::FavoriteMusic => "我喜欢的音乐",
            Self::Recent => "最近播放",
            Self::MyPodcast => "我的播客",
            Self::MyCollection => "我的收藏",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Recommend => "icons/sidebar_home.svg",
            Self::Featured => "icons/sidebar_featured.svg",
            Self::Podcast => "icons/sidebar_podcast.svg",
            Self::Roaming => "icons/sidebar_fm.svg",
            Self::Following => "icons/sidebar_community.svg",
            Self::FavoriteMusic => "icons/sidebar_like.svg",
            Self::Recent => "icons/sidebar_history.svg",
            Self::MyPodcast => "icons/sidebar_my_podcast.svg",
            Self::MyCollection => "icons/sidebar_favourite.svg",
        }
    }

    /// TODO: 如果有通知的话，会在右边显示个小红点
    fn has_notification(self) -> bool {
        matches!(self, Self::Following | Self::MyPodcast)
    }
}

pub(super) struct SidebarChanged;

pub(super) struct SidebarPage {
    theme: Theme,
    active_page: ContentPage,
    window_move_pending: bool,
}

impl EventEmitter<SidebarChanged> for SidebarPage {}

impl SidebarPage {
    pub(super) fn new(theme: Theme) -> Self {
        Self {
            theme,
            active_page: ContentPage::Recommend,
            window_move_pending: false,
        }
    }

    pub(super) fn active_page(&self) -> ContentPage {
        self.active_page
    }
}

fn page_header(theme: Theme, cx: &mut Context<SidebarPage>) -> impl IntoElement {
    div()
        .id("left-page-header")
        .h(px(72.))
        .w_full()
        .px(px(18.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .top_10()
                .left_1()
                .child(
                    div()
                        .size(px(24.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.primary)
                        .text_color(theme.white1)
                        .child(
                            svg()
                                .path("icons/logo/logo.svg")
                                .size(px(27.))
                                .flex_none()
                                .text_color(theme.white1),
                        ),
                )
                .child(
                    svg()
                        .path("icons/logo/logo_text.svg")
                        .w(px(101.))
                        .h(px(19.))
                        .text_color(theme.black1),
                ),
        )
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
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    let active = page == active_page;

    div()
        .id(page.id())
        .w_full()
        .h(px(36.))
        .flex()
        .items_center()
        .gap(px(8.))
        .p(px(8.))
        .rounded(px(8.))
        .text_size(px(14.))
        .text_color(if active { theme.white1 } else { theme.black1 })
        .when(active, |this| {
            this.bg(theme.primary).font_weight(FontWeight::MEDIUM)
        })
        .when(!active, |this| {
            this.hover(|style| style.bg(theme.sidebar_subtle))
        })
        .child(
            svg()
                .path(page.icon())
                .size(IconSize::Small.pixels())
                .flex_none()
                .text_color(if active { theme.white1 } else { theme.black5 }),
        )
        .child(page.title())
        .when(page.has_notification(), |this| {
            this.child(
                div()
                    .size(px(5.))
                    .rounded_full()
                    .bg(theme.primary)
                    .flex_none(),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.active_page = page;
            cx.notify();
            cx.emit(SidebarChanged);
        }))
}

fn main_navigation(
    active_page: ContentPage,
    theme: Theme,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(page_nav_item(
            ContentPage::Recommend,
            active_page,
            theme,
            cx,
        ))
        .child(page_nav_item(ContentPage::Featured, active_page, theme, cx))
        .child(page_nav_item(ContentPage::Podcast, active_page, theme, cx))
        .child(page_nav_item(ContentPage::Roaming, active_page, theme, cx))
        .child(page_nav_item(
            ContentPage::Following,
            active_page,
            theme,
            cx,
        ))
}

fn library_navigation(
    active_page: ContentPage,
    theme: Theme,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .h(px(36.))
                .flex()
                .items_center()
                .px(px(8.))
                .text_size(px(12.))
                .text_color(rgba(0x28324866))
                .child("我的"),
        )
        .child(page_nav_item(
            ContentPage::FavoriteMusic,
            active_page,
            theme,
            cx,
        ))
        .child(page_nav_item(ContentPage::Recent, active_page, theme, cx))
        .child(page_nav_item(
            ContentPage::MyPodcast,
            active_page,
            theme,
            cx,
        ))
        .child(page_nav_item(
            ContentPage::MyCollection,
            active_page,
            theme,
            cx,
        ))
}

impl Render for SidebarPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgba(0x28324808))
            .child(page_header(theme, cx))
            .child(
                div()
                    .id("left-page-content")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .p(px(18.))
                    .child(main_navigation(self.active_page, theme, cx))
                    .child(
                        div()
                            .w_full()
                            .h(px(1.))
                            .my(px(12.))
                            .bg(theme.sidebar_subtle),
                    )
                    .child(library_navigation(self.active_page, theme, cx))
                    .child(
                        div()
                            .w_full()
                            .h(px(1.))
                            .my(px(12.))
                            .bg(theme.sidebar_subtle),
                    ),
            )
    }
}
