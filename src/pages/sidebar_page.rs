use gpui::prelude::{FluentBuilder, StatefulInteractiveElement};
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme as BaseTheme};

use crate::components::{WindowDragState, window_drag_region};
use crate::theme::IconSize;

/// 主内容区可切换的页面，侧边栏里的每一个导航项都对应其中一个。
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

/// 顶部导航分组：推荐、精选、播客、漫游、关注。
const MAIN_PAGES: [ContentPage; 5] = [
    ContentPage::Recommend,
    ContentPage::Featured,
    ContentPage::Podcast,
    ContentPage::Roaming,
    ContentPage::Following,
];

/// 「我的音乐库」分组，展示在分隔线下方。
const LIBRARY_PAGES: [ContentPage; 4] = [
    ContentPage::FavoriteMusic,
    ContentPage::Recent,
    ContentPage::MyPodcast,
    ContentPage::MyCollection,
];

impl ContentPage {
    /// 同时作为元素 id，GPUI 靠它来匹配状态与事件。
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

/// 选中项变化时发出的事件，`MainContent` 订阅它来重新渲染右侧内容。
pub(super) struct SidebarChanged;

pub(super) struct SidebarPage {
    active_page: ContentPage,
    window_move_pending: bool,
    /// 假数据，格式是 `(歌单名, 封面路径)`。
    playlists: Vec<(&'static str, &'static str)>,
}

impl EventEmitter<SidebarChanged> for SidebarPage {}

impl WindowDragState for SidebarPage {
    fn window_move_pending_mut(&mut self) -> &mut bool {
        &mut self.window_move_pending
    }
}

impl SidebarPage {
    pub(super) fn new() -> Self {
        Self {
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

/// 顶部 logo 区域，同时充当窗口拖动手柄。
fn page_header(colors: ColorTokens, cx: &mut Context<SidebarPage>) -> impl IntoElement {
    window_drag_region("left-page-header", cx)
        .h(px(72.))
        .w_full()
        .px(px(18.))
        .child(
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

/// 分组标题，例如「我的」「创建的歌单 25」。
fn section_heading(
    id: impl Into<ElementId>,
    text: &'static str,
    colors: ColorTokens,
) -> impl IntoElement {
    sidebar_row(id, px(36.))
        .text_size(px(12.))
        .text_color(colors.muted_foreground)
        .child(text)
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

fn created_playlist(
    title: &'static str,
    cover: &'static str,
    colors: ColorTokens,
) -> impl IntoElement {
    sidebar_row(title, px(42.))
        .hover(|style| style.bg(colors.accent))
        .child(img(cover).size(px(32.)).rounded(px(4.)).flex_none())
        .child(
            div()
                .flex_1()
                // 配合 flex_1 让长歌单名截断而不是把行撑宽
                .min_w(px(0.))
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(colors.secondary_foreground)
                .child(title),
        )
}

fn created_playlists(
    playlists: &[(&'static str, &'static str)],
    colors: ColorTokens,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(section_heading(
            "created-playlists-heading",
            "创建的歌单 25",
            colors,
        ))
        .children(
            playlists
                .iter()
                .map(|(title, cover)| created_playlist(title, cover, colors)),
        )
}

impl Render for SidebarPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = BaseTheme::global(cx).tokens.colors;

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.foreground.alpha(0.03))
            .child(page_header(colors, cx))
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
                            .child(created_playlists(&self.playlists, colors)),
                    ),
            )
    }
}
