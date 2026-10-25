use super::components::window_drag_area;
use gpui::prelude::{FluentBuilder, StatefulInteractiveElement};
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use super::assets::thumbnail_url;
use super::events::{Navigate, NavigationBus};
use super::pages::{ContentPage, LIBRARY_PAGES, MAIN_PAGES};
use super::shell::{HEADER_HEIGHT, HEADER_TOP_INSET};
use super::theme::{IconSize, PRESSED_ICON_ALPHA};
use crate::models::{PlayMode, Playlist};
use crate::state::library::MusicLibrary;

/// 选中项变化时发出的事件，`MainContent` 订阅它来重新渲染右侧内容。
pub(super) struct SidebarChanged(pub ContentPage);

/// 未登录侧边栏的「立即登录云音乐」框被点击。
pub(super) struct LoginRequested;

impl EventEmitter<LoginRequested> for SidebarPage {}

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

    pub(super) fn set_active_page(&mut self, page: ContentPage, cx: &mut Context<Self>) {
        if self.active_page != page {
            self.active_page = page;
            cx.notify();
        }
    }

    fn navigate(&mut self, page: ContentPage, cx: &mut Context<Self>) {
        cx.emit(SidebarChanged(page));
    }
}

/// 顶部 logo 区域，同时充当窗口拖动手柄。
fn page_header(colors: ColorTokens) -> impl IntoElement {
    window_drag_area("sidebar-header")
        .h(px(HEADER_HEIGHT))
        .pt(px(HEADER_TOP_INSET))
        .flex_none()
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(22.))
        .pr(px(14.))
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

/// 「我喜欢的音乐」右侧的心动模式入口：一个纯图标按钮。
///
/// 它嵌在导航按钮内部，所以按下时先把鼠标事件拦下：GPUI 的点击是冒泡的，
/// 不拦的话外层导航会跟着一起触发，点了图标反而跳到「我喜欢的音乐」。
///
/// 配色跟着所在导航项走：这一行被选中时底色是品牌红，图标就得转白，
/// 否则红底上一个灰图标会糊在一起；未选中时保持灰，hover 才亮成品牌红。
fn heart_mode_entry(
    active: bool,
    colors: ColorTokens,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    let (idle, hover, pressed) = if active {
        (
            colors.primary_foreground.alpha(0.8),
            colors.primary_foreground,
            colors.primary_foreground.alpha(PRESSED_ICON_ALPHA),
        )
    } else {
        (
            colors.muted_foreground,
            colors.primary,
            colors.primary.alpha(PRESSED_ICON_ALPHA),
        )
    };
    svg()
        .path(PlayMode::Heart.icon_path())
        .size(px(18.))
        .flex_none()
        .text_color(idle)
        .hover(move |style| style.text_color(hover))
        // 纯 SVG 想当按钮用必须先有 id，之后才能挂 role / aria_label / on_click。
        .id("sidebar-heart-mode")
        .role(Role::Button)
        .aria_label("心动模式")
        .active(move |style| style.text_color(pressed))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|_, _, _, cx| {
            cx.stop_propagation();
            // 和首页那张心动卡片是同一件事，走同一条意图总线。
            NavigationBus::emit(cx, Navigate::HeartMode);
        }))
}

/// 单个导航项：选中时用主题色高亮，未选中时悬停才显示背景。
fn page_nav_item(
    page: ContentPage,
    active_page: ContentPage,
    colors: ColorTokens,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    let active = page == active_page;
    // 「我喜欢的音乐」多一个心动模式入口。
    let with_heart = page == ContentPage::FavoriteMusic;
    let heart = with_heart.then(|| heart_mode_entry(active, colors, cx));

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
        // 有心动入口的行，用一个撑开的空元素把图标顶到行尾。
        .when_some(heart, |this, heart| {
            this.child(div().flex_1().min_w(px(0.))).child(heart)
        })
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
            this.navigate(page, cx);
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

fn playlist_nav_item(
    playlist: &Playlist,
    active_page: ContentPage,
    colors: ColorTokens,
    cx: &mut Context<SidebarPage>,
) -> impl IntoElement {
    let page = ContentPage::Playlist(playlist.id);
    let active = active_page == page;
    Button::new(("playlist-nav", playlist.id))
        .w_full()
        .h(px(42.))
        .gap(px(8.))
        .px(px(8.))
        .rounded(px(8.))
        .justify_start()
        .accessibility_label(playlist.name.clone())
        .selected(active)
        .styles(|styles| styles.selected(|style| style.bg(colors.primary)))
        .when(!active, |row| row.hover(|style| style.bg(colors.accent)))
        .on_click(cx.listener(move |this, _, _, cx| this.navigate(page, cx)))
        .when_some(playlist.cover_img_url.clone(), |row, cover| {
            row.child(
                img(thumbnail_url(&cover, 64))
                    .size(px(32.))
                    .rounded(px(4.))
                    .object_fit(ObjectFit::Cover)
                    .flex_none(),
            )
        })
        .child(
            div()
                .flex_1()
                // 配合 flex_1 让长歌单名截断而不是把行撑宽
                .min_w(px(0.))
                .text_size(px(12.))
                .line_height(px(16.))
                .line_clamp(2)
                .text_color(if active {
                    colors.primary_foreground
                } else {
                    colors.secondary_foreground
                })
                .child(playlist.name.clone()),
        )
}

/// 未登录时替代「创建的歌单 / 收藏的歌单」区域的登录入口框：
/// 非常淡的圆角方框，左边文案右边箭头，点击拉起登录窗。
fn login_prompt(colors: ColorTokens, cx: &mut Context<SidebarPage>) -> Stateful<Div> {
    div()
        .id("sidebar-login-prompt")
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(8.))
        .px(px(12.))
        .py(px(10.))
        .bg(colors.foreground.alpha(0.04))
        .text_size(px(12.))
        .text_color(colors.secondary_foreground)
        .cursor_pointer()
        .on_click(cx.listener(|_, _, _, cx| cx.emit(LoginRequested)))
        .child("立即登录云音乐")
        .child(
            svg()
                .path("icons/chevron_right.svg")
                .size(px(14.))
                .text_color(colors.secondary_foreground),
        )
}

fn playlist_group(
    id: &'static str,
    title: &'static str,
    playlists: &[Playlist],
    active_page: ContentPage,
    colors: ColorTokens,
    cx: &mut Context<SidebarPage>,
) -> Div {
    let mut group = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(section_heading(
            id,
            format!("{title} {}", playlists.len()),
            colors,
        ));
    for playlist in playlists {
        group = group.child(playlist_nav_item(playlist, active_page, colors, cx));
    }
    group
}

impl Render for SidebarPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let library = self.library.read(cx);
        let (created, subscribed): (Vec<_>, Vec<_>) = library
            .playlists()
            .iter()
            .filter(|playlist| playlist.special_type != 5)
            .cloned()
            .partition(|playlist| playlist.creator.user_id == library.user_id);
        // 未登录（user_id == 0）时歌单区换成登录入口框；歌单列表本来就为空。
        let playlist_section = if library.user_id == 0 {
            login_prompt(colors, cx).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .child(playlist_group(
                    "created-playlists-heading",
                    "创建的歌单",
                    &created,
                    self.active_page,
                    colors,
                    cx,
                ))
                .child(divider(colors))
                .child(playlist_group(
                    "subscribed-playlists-heading",
                    "收藏的歌单",
                    &subscribed,
                    self.active_page,
                    colors,
                    cx,
                ))
                .into_any_element()
        };

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
                            .child(playlist_section),
                    ),
            )
    }
}
