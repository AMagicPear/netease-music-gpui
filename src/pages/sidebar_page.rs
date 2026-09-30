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
    playlists: Vec<(&'static str, &'static str)>,
}

impl EventEmitter<SidebarChanged> for SidebarPage {}

impl SidebarPage {
    pub(super) fn new(theme: Theme) -> Self {
        Self {
            theme,
            active_page: ContentPage::Recommend,
            window_move_pending: false,
            playlists: vec![
                (
                    "would u wanna ride with me",
                    "/Users/amagicpear/Pictures/Perry Origin Character/ChatGPT Image 2026年9月29日 15_39_30.png",
                ),
                (
                    "回声之境 | Echoesphere",
                    "/Users/amagicpear/Pictures/Perry Origin Character/ChatGPT Image 2026年9月20日 22_25_55.png",
                ),
            ],
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

    sidebar_row(page.id(), px(36.))
        .p(px(8.))
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
                .text_color(theme.black5)
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

fn created_playlist(title: &'static str, cover: &'static str, theme: Theme) -> impl IntoElement {
    sidebar_row(title, px(42.))
        .hover(|style| style.bg(theme.sidebar_subtle))
        .child(img(cover).size(px(32.)).rounded(px(4.)).flex_none())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme.black3)
                .child(title),
        )
}

fn created_playlists(playlists: &[(&'static str, &'static str)], theme: Theme) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            sidebar_row("created-playlists-heading", px(36.))
                .text_size(px(12.))
                .text_color(theme.black5)
                .child("创建的歌单 25"),
        )
        .children(
            playlists
                .iter()
                .map(|(title, cover)| created_playlist(title, cover, theme)),
        )
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
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .p(px(18.))
                            .child(main_navigation(self.active_page, theme, cx))
                            .child(div().w_full().h(px(1.)).my(px(12.)).bg(theme.black10))
                            .child(library_navigation(self.active_page, theme, cx))
                            .child(div().w_full().h(px(1.)).my(px(12.)).bg(theme.black10))
                            .child(created_playlists(&self.playlists, theme)),
                    ),
            )
    }
}
