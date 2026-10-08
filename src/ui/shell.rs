use std::{cell::Cell, collections::HashMap, rc::Rc};

use gpui::prelude::{FluentBuilder, StatefulInteractiveElement};
use gpui::*;

use super::assets::thumbnail_url;
use super::components::{PlayerBar, ResizeDragPreview, window_drag_area};
use super::pages::{ContentPage, playlist::PlaylistPage};
use super::sidebar::{SidebarChanged, SidebarPage};
use super::theme::{IconSize, PRESSED_ICON_ALPHA};
use crate::playback::PlaybackController;
use crate::state::account::AccountState;
use crate::state::library::MusicLibrary;
use gpui_kit::base::input::{Input, InputState};
use gpui_kit::base::{Button, ColorTokens, Scrollbar, ScrollbarMode, Theme};
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::{Sizable, TitleBar};

const MIN_SIDEBAR_WIDTH: Pixels = px(204.);
const MAX_SIDEBAR_WIDTH: Pixels = px(627.);
/// 搜索框的理想宽度
const SEARCH_BOX_WIDTH: Pixels = px(258.);
/// 搜索框的压缩下限
const SEARCH_BOX_MIN_WIDTH: Pixels = px(40.);

pub struct MainWindow {
    pub main_content: Entity<MainContent>,
    pub player_bar: Entity<PlayerBar>,
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .child(
                TitleBar::new()
                    .on_close_window(|_, window, cx| crate::desktop::close_window(window, cx))
                    .h(px(30.))
                    .pl(px(0.))
                    .border_b_0()
                    .bg(colors.background)
                    .child(
                        div()
                            .w(self.main_content.read(cx).sidebar_width)
                            .h_full()
                            .flex_none()
                            .bg(colors.foreground.alpha(0.03)),
                    ),
            )
            .child(self.main_content.clone())
            .child(self.player_bar.clone())
    }
}

pub struct MainContent {
    active_page: ContentPage,
    sidebar_width: Pixels,
    sidebar: Entity<SidebarPage>,
    search_input: Entity<InputState>,
    user_profile: Entity<AccountState>,
    /// 每个导航项对应一个页面 View，一次创建后长期持有。
    pages: HashMap<ContentPage, AnyView>,
    playlist_page: Entity<PlaylistPage>,
    library: Entity<MusicLibrary>,
    page_scroll: HashMap<ContentPage, ScrollHandle>,
    _sidebar_subscription: Subscription,
    _user_profile_subscription: Subscription,
    _library_subscription: Subscription,
}

impl MainContent {
    pub fn new(
        window: &mut Window,
        user_profile: Entity<AccountState>,
        library: Entity<MusicLibrary>,
        playback: Entity<PlaybackController>,
        cx: &mut Context<Self>,
    ) -> Self {
        let sidebar = cx.new(|cx| SidebarPage::new(library.clone(), cx));
        let sidebar_subscription = cx.subscribe(&sidebar, |this, _, event: &SidebarChanged, cx| {
            this.navigate(event.0, cx);
        });
        let playlist_page = cx.new(|cx| PlaylistPage::new(library.clone(), playback, cx));
        let library_subscription = cx.observe(&library, |this, _, cx| {
            // 收藏歌单的 ID 要等账号索引返回后才能解析。
            this.open_playlist(this.active_page, cx);
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
            .filter_map(|page| page.build(cx).map(|view| (page, view)))
            .collect();
        let user_profile_subscription = cx.observe(&user_profile, |_, _, cx| cx.notify());

        Self {
            active_page: ContentPage::Recommend,
            sidebar_width: MIN_SIDEBAR_WIDTH,
            sidebar,
            search_input,
            user_profile,
            pages,
            playlist_page,
            library,
            page_scroll: ContentPage::all()
                .map(|page| (page, ScrollHandle::default()))
                .collect(),
            _sidebar_subscription: sidebar_subscription,
            _user_profile_subscription: user_profile_subscription,
            _library_subscription: library_subscription,
        }
    }

    /// 所有入口统一提交导航，侧栏负责显示选中项和发出请求。
    pub(super) fn navigate(&mut self, page: ContentPage, cx: &mut Context<Self>) {
        self.active_page = page;
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_active_page(page, cx));
        self.page_scroll.entry(page).or_default();
        self.open_playlist(page, cx);
        cx.notify();
    }

    fn open_playlist(&mut self, page: ContentPage, cx: &mut Context<Self>) {
        let id = match page {
            ContentPage::FavoriteMusic => self
                .library
                .read(cx)
                .favorite_playlist()
                .map(|playlist| playlist.id),
            ContentPage::Playlist(id) => Some(id),
            _ => return,
        };
        self.playlist_page.update(cx, |view, cx| view.open(id, cx));
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
                .id("search-box-icon")
                // 同上：按下换用更浅的「按下色」，避免按住拖出时叠加 opacity 造成双重变淡
                .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA))),
        )
        .child(Input::new(&input))
}

fn hover_icon(
    id: &'static str,
    label: &'static str,
    path: &'static str,
    colors: ColorTokens,
) -> Button {
    Button::new(id)
        .aria_label(label)
        .size(IconSize::Small.pixels())
        .ml(px(10.))
        .flex_none()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            svg()
                .path(path)
                .size(IconSize::Small.pixels())
                .flex_none()
                .text_color(colors.foreground.alpha(0.6))
                .hover(|style| style.text_color(colors.foreground))
                .id((id, 0usize))
                .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA))),
        )
}

fn page_header(
    colors: ColorTokens,
    search_input: Entity<InputState>,
    nickname: String,
    avatar_url: String,
    vip_badge: Option<String>,
) -> impl IntoElement {
    window_drag_area("page-header")
        .h(px(42.))
        .flex_none()
        .w_full()
        .bg(colors.background)
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
                .child(Avatar::new().with_size(px(28.)).flex_none().src(avatar_url)),
        )
        .child(
            div()
                .id("header-profile-menu")
                // 把整块（昵称 + VIP + 箭头）声明成一个 group，
                // 子元素就能用 group_hover 感知「整块是否被悬浮」，而不是各自单独判断。
                .group("header-profile-menu")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .ml(px(4.))
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(13.))
                // 昵称保持原本设计的 0.7；文字子元素会继承这个色，
                // 所以这里的 hover 一并让昵称变深到 foreground。
                .text_color(colors.foreground.alpha(0.7))
                .hover(|style| style.text_color(colors.foreground))
                .child(nickname)
                .when_some(vip_badge, |menu, badge| {
                    menu.child(img(badge).h(px(16.)).flex_none())
                })
                .child(
                    svg()
                        .path("icons/unfold.svg")
                        .size(px(20.))
                        .flex_none()
                        // svg 不继承文字色的 hover，改用 group_hover：
                        // 只要整块被悬浮，箭头也跟着变深到 foreground。
                        .text_color(colors.foreground.alpha(0.6))
                        .group_hover("header-profile-menu", |style| {
                            style.text_color(colors.foreground)
                        }),
                ),
        )
        .child(hover_icon(
            "header-message-button",
            "消息",
            "icons/message.svg",
            colors,
        ))
        .child(hover_icon(
            "header-setting-button",
            "设置",
            "icons/setting.svg",
            colors,
        ))
        .child(hover_icon(
            "header-skin-button",
            "皮肤",
            "icons/skin.svg",
            colors,
        ))
        .child(hover_icon(
            "header-mini-button",
            "迷你模式",
            "icons/menu_mini.svg",
            colors,
        ))
}

impl Render for MainContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let user_profile = self.user_profile.read(cx);
        let active_page = self.active_page;
        let page_scroll = &self.page_scroll[&active_page];
        let page = match active_page {
            ContentPage::FavoriteMusic | ContentPage::Playlist(_) => {
                Some(self.playlist_page.clone().into())
            }
            _ => self.pages.get(&active_page).cloned(),
        };
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
                        user_profile.profile.nickname.clone(),
                        thumbnail_url(&user_profile.profile.avatar_url, 56),
                        user_profile.vip.as_ref().and_then(|vip| {
                            vip.badge_path(
                                time::OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000,
                            )
                        }),
                    ))
                    // 右侧具体页面
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h(px(0.))
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id((
                                        ElementId::Name("right-page-content".into()),
                                        active_page.id(),
                                    ))
                                    .flex_1()
                                    .min_h(px(0.))
                                    .px(px(40.))
                                    .py(px(18.))
                                    .overflow_y_scroll()
                                    .track_scroll(page_scroll)
                                    .children(page),
                            )
                            .when(
                                matches!(
                                    active_page,
                                    ContentPage::FavoriteMusic | ContentPage::Playlist(_)
                                ),
                                |container| {
                                    container.child(
                                        self.playlist_page
                                            .update(cx, |page, cx| page.floating_header(cx)),
                                    )
                                },
                            )
                            // 滚动条放在外层，避免它的边界被计入滚动内容高度。
                            .child(Scrollbar::vertical(page_scroll).mode(ScrollbarMode::Always)),
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
