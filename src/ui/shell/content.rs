use std::{cell::Cell, collections::HashMap, rc::Rc};

use gpui::prelude::{FluentBuilder, StatefulInteractiveElement};
use gpui::*;
use gpui_kit::base::input::{Input, InputEvent, InputState};
use gpui_kit::base::{Button, ColorTokens, History, Scrollbar, ScrollbarMode, Theme};
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;

use super::{HEADER_HEIGHT, HEADER_TOP_INSET, hover_icon, mini_and_window_buttons};
use crate::models::{Playlist, Song};
use crate::playback::PlaybackController;
use crate::state::{
    account::AccountState, daily::DailySongsStore, library::MusicLibrary,
    recommend::RecommendStore, search_results::SearchResults,
};
use crate::ui::assets::thumbnail_url;
use crate::ui::comments::SongCommentsPage;
use crate::ui::components::{
    PAGE_SIDE_PADDING, ResizeDragPreview, SidePanel, SidePanelContent, SurfacePopover,
    search_clear_button, window_drag_area,
};
use crate::ui::cover_color::Backdrop;
use crate::ui::events::{ContentLayoutChanged, PlaylistCommentsTab};
use crate::ui::login;
use crate::ui::pages::{
    ContentPage, daily::DailySongsPage, playlist::PlaylistPage, recommend::RecommendPage,
    search::SearchPage,
};
use crate::ui::sidebar::{LoginRequested, SidebarChanged, SidebarPage};
use crate::ui::theme::PRESSED_ICON_ALPHA;

const MIN_SIDEBAR_WIDTH: Pixels = px(204.);
const MAX_SIDEBAR_WIDTH: Pixels = px(627.);
const SEARCH_BOX_WIDTH: Pixels = px(258.);
const SEARCH_BOX_MIN_WIDTH: Pixels = px(40.);

pub struct MainContent {
    pub(super) active_page: ContentPage,
    history: History<ContentPage>,
    sidebar_width: Pixels,
    sidebar: Entity<SidebarPage>,
    search_input: Entity<InputState>,
    user_profile: Entity<AccountState>,
    login_window: Option<AnyWindowHandle>,
    profile_menu_open: bool,
    account_id: u64,
    /// 占位页面长期持有；业务页面在下面单独注入依赖。
    pages: HashMap<ContentPage, AnyView>,
    recommend_page: Entity<RecommendPage>,
    /// 日推页长期持有：切走再回来不重复请求，日期数据由 store 按账号刷新。
    daily_page: Entity<DailySongsPage>,
    pub(super) playlist_page: Entity<PlaylistPage>,
    song_comments_page: Entity<SongCommentsPage>,
    search_page: Entity<SearchPage>,
    search_results: Entity<SearchResults>,
    library: Entity<MusicLibrary>,
    side_panel: Entity<SidePanel>,
    page_scroll: HashMap<ContentPage, ScrollHandle>,
    pub(super) playlist_backdrop: Backdrop,
    _sidebar_subscription: Subscription,
    _sidebar_login_subscription: Subscription,
    _user_profile_subscription: Subscription,
    _library_subscription: Subscription,
    _search_subscription: Subscription,
    _playlist_subscription: Subscription,
}

impl EventEmitter<ContentLayoutChanged> for MainContent {}

impl MainContent {
    pub fn new(
        window: &mut Window,
        user_profile: Entity<AccountState>,
        library: Entity<MusicLibrary>,
        playback: Entity<PlaybackController>,
        side_panel: Entity<SidePanel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let sidebar = cx.new(|cx| SidebarPage::new(library.clone(), cx));
        let sidebar_subscription = cx.subscribe(&sidebar, |this, _, event: &SidebarChanged, cx| {
            this.navigate(event.0, cx);
        });
        let login_subscription = cx.subscribe_in(
            &sidebar,
            window,
            |this, _, _: &LoginRequested, window, cx| {
                this.login_window =
                    login::open(window, this.user_profile.clone(), this.login_window, cx);
            },
        );
        // 吸顶编辑框要按自己的屏幕位置从这份渐变里取色，所以两边共用同一个句柄。
        let playlist_backdrop = Backdrop::default();
        let playlist_page = cx.new(|cx| {
            PlaylistPage::new(
                window,
                library.clone(),
                playback.clone(),
                playlist_backdrop.clone(),
                cx,
            )
        });
        let playlist_subscription = cx
            .subscribe(&playlist_page, |_, _, _: &PlaylistCommentsTab, cx| {
                cx.notify()
            });
        let song_comments_page = cx.new(|cx| SongCommentsPage::new(window, cx));
        let recommended = cx.new(|cx| RecommendStore::new(user_profile.clone(), cx));
        let recommend_page = cx.new(|_| RecommendPage::new(recommended));
        let daily_store = cx.new(|cx| DailySongsStore::new(user_profile.clone(), cx));
        let daily_page =
            cx.new(|cx| DailySongsPage::new(daily_store, library.clone(), playback.clone(), cx));
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
        let search_results = cx.new(|_| SearchResults::default());
        let search_page = cx.new(|cx| {
            SearchPage::new(
                library.clone(),
                playback.clone(),
                search_results.clone(),
                cx,
            )
        });
        let search_subscription = cx.subscribe(&search_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_current_input(cx);
            }
        });

        let pages = ContentPage::all()
            .filter_map(|page| page.build(cx).map(|view| (page, view)))
            .collect();
        let mut history = History::new();
        history.push(ContentPage::Recommend);
        let user_profile_subscription = cx.observe(&user_profile, |this, account, cx| {
            let user_id = account.read(cx).profile.user_id;
            if this.account_id != user_id {
                this.account_id = user_id;
                this.playlist_page
                    .update(cx, |page, cx| page.open(None, None, cx));
                this.navigate(ContentPage::Recommend, cx);
            }
            cx.notify();
        });

        Self {
            active_page: ContentPage::Recommend,
            history,
            sidebar_width: MIN_SIDEBAR_WIDTH,
            sidebar,
            search_input,
            user_profile,
            login_window: None,
            profile_menu_open: false,
            account_id: 0,
            pages,
            recommend_page,
            daily_page,
            playlist_page,
            song_comments_page,
            search_page,
            search_results,
            library,
            side_panel,
            playlist_backdrop,
            page_scroll: ContentPage::all()
                .map(|page| (page, ScrollHandle::default()))
                .collect(),
            _sidebar_subscription: sidebar_subscription,
            _sidebar_login_subscription: login_subscription,
            _user_profile_subscription: user_profile_subscription,
            _library_subscription: library_subscription,
            _search_subscription: search_subscription,
            _playlist_subscription: playlist_subscription,
        }
    }

    fn submit_search(&mut self, keywords: String, cx: &mut Context<Self>) {
        self.search_results
            .update(cx, |results, cx| results.search(keywords, cx));
        self.navigate(ContentPage::Search, cx);
    }

    fn submit_current_input(&mut self, cx: &mut Context<Self>) {
        let keywords = self.search_input.read(cx).value().trim().to_owned();
        if !keywords.is_empty() {
            self.submit_search(keywords, cx);
        }
    }

    pub(super) fn navigate(&mut self, page: ContentPage, cx: &mut Context<Self>) {
        if self.active_page != page {
            self.history.push(page);
        }
        self.show_page(page, cx);
    }

    pub(super) fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.history.back() {
            self.show_page(page, cx);
        }
    }

    /// 返回历史页面时不再入栈。
    fn show_page(&mut self, page: ContentPage, cx: &mut Context<Self>) {
        let changed = self.active_page != page;
        self.active_page = page;
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_active_page(page, cx));
        self.page_scroll.entry(page).or_default();
        self.open_playlist(page, cx);
        if page != ContentPage::Search {
            self.search_page.update(cx, |page, _| page.reset_scroll());
        }
        if changed {
            cx.emit(ContentLayoutChanged);
        }
        cx.notify();
    }

    /// 先用摘要显示页头；导航补充喜欢状态，同 ID 的加载不会重复发请求。
    pub(super) fn open_playlist_detail(
        &mut self,
        id: u64,
        summary: Option<Playlist>,
        cx: &mut Context<Self>,
    ) {
        self.playlist_page
            .update(cx, |view, cx| view.open(Some(id), summary, cx));
        self.navigate(ContentPage::Playlist(id), cx);
    }

    pub(super) fn open_song_comments(
        &mut self,
        song: Song,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = song.id;
        self.song_comments_page
            .update(cx, |page, cx| page.open(song, window, cx));
        self.navigate(ContentPage::SongComments(id), cx);
    }

    fn open_playlist(&mut self, page: ContentPage, cx: &mut Context<Self>) {
        let (id, favorite) = match page {
            ContentPage::FavoriteMusic => (
                self.library
                    .read(cx)
                    .favorite_playlist()
                    .map(|playlist| playlist.id),
                true,
            ),
            ContentPage::Playlist(id) => (
                Some(id),
                self.library
                    .read(cx)
                    .favorite_playlist()
                    .is_some_and(|playlist| playlist.id == id),
            ),
            _ => return,
        };
        self.playlist_page.update(cx, |view, cx| {
            view.set_favorite_view(favorite);
            // 返回页面不再传摘要，避免覆盖已加载的完整详情。
            view.open(id, None, cx);
        });
    }
}

fn search_box(
    input: Entity<InputState>,
    has_value: bool,
    on_submit: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    colors: ColorTokens,
) -> impl IntoElement {
    div()
        .occlude()
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
                .on_click(on_submit)
                .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA))),
        )
        .child(Input::new(&input))
        // 判断用原始内容（trim 过的空串对用户来说「看不见但删不掉」）；
        // 按钮 `flex_none`，框被压缩时先压输入区、按钮始终完整。
        .when(has_value, |box_| {
            box_.child(search_clear_button(
                input.clone(),
                "search-box-clear",
                colors,
            ))
        })
}

impl MainContent {
    fn profile_menu(&self, colors: ColorTokens, cx: &Context<Self>) -> impl IntoElement {
        let profile = self.user_profile.read(cx);
        let logged_in = profile.profile.user_id != 0;
        let was_open = self.profile_menu_open;
        let nickname = profile.profile.nickname.clone();
        let avatar_url = profile.profile.avatar_url.clone();
        let vip_badge = profile.vip.as_ref().and_then(|vip| {
            vip.badge_path(time::OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000)
        });

        let trigger = div()
            .id("header-profile-trigger")
            .flex_none()
            .flex()
            .items_center()
            .occlude()
            .cursor_pointer()
            .focusable()
            .role(Role::Button)
            .aria_label("账号菜单")
            // 守卫比较本次渲染的开合值：点在触发器上时，弹层的 down-out
            // 监听可能已经先把它关掉了，这次按下不能再翻回去（同音质弹层）。
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if logged_in {
                        if this.profile_menu_open == was_open {
                            this.profile_menu_open = !was_open;
                            cx.notify();
                        }
                    } else {
                        this.login_window =
                            login::open(window, this.user_profile.clone(), this.login_window, cx);
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                Avatar::new()
                    .with_size(px(28.))
                    .flex_none()
                    .src(if avatar_url.is_empty() {
                        "images/default_avatar.png".to_owned()
                    } else {
                        thumbnail_url(&avatar_url, 56)
                    }),
            )
            .child(
                div()
                    .group("header-profile-menu")
                    .ml(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(13.))
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
                            .text_color(colors.foreground.alpha(0.6))
                            .group_hover("header-profile-menu", |style| {
                                style.text_color(colors.foreground)
                            }),
                    ),
            );

        div().ml_auto().flex_none().child(
            SurfacePopover::new("header-profile-popover", logged_in && was_open, trigger)
                .anchor(Anchor::TopRight)
                .offset(px(8.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.profile_menu_open = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("header-profile-logout")
                        // 行声明成 group，图标才能跟着行的 hover 一起变色
                        //（gpui 的 svg() 不继承父级 text_color，必须显式上色）。
                        .group("header-profile-logout")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(12.))
                        .py(px(8.))
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.foreground.alpha(0.8))
                        .hover(|style| style.text_color(colors.primary))
                        .child(
                            svg()
                                .path("icons/logout.svg")
                                .size(px(20.))
                                .flex_none()
                                .text_color(colors.foreground.alpha(0.8))
                                .group_hover("header-profile-logout", |style| {
                                    style.text_color(colors.primary)
                                }),
                        )
                        .child("退出登录")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.profile_menu_open = false;
                            this.user_profile
                                .update(cx, |account, cx| account.logout(cx));
                            cx.notify();
                        })),
                ),
        )
    }

    /// 页头空白处拖动窗口，交互控件阻止拖动。
    fn page_header(
        &self,
        window: &Window,
        colors: ColorTokens,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        window_drag_area("page-header")
            .h(px(HEADER_HEIGHT))
            .pt(px(HEADER_TOP_INSET))
            .flex_none()
            .w_full()
            .px(px(40.))
            .flex()
            .items_center()
            .min_w(px(0.))
            .child(
                Button::new("back-button")
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .h_9()
                    .w_7()
                    .flex_none()
                    .border_1()
                    .border_color(colors.border)
                    .rounded_lg()
                    .hover(|style| style.bg(colors.accent))
                    // 没有可回退的页面时点击静默无效果，不做禁用外观。
                    .on_click(cx.listener(|this, _, _, cx| this.go_back(cx)))
                    .child(
                        svg()
                            .path("icons/backward.svg")
                            .size(px(11.))
                            .text_color(colors.secondary_foreground),
                    ),
            )
            .child(search_box(
                self.search_input.clone(),
                !self.search_input.read(cx).value().is_empty(),
                cx.listener(|this, _, _, cx| this.submit_current_input(cx)),
                colors,
            ))
            .child(self.profile_menu(colors, cx))
            .child(
                hover_icon("header-message-button", "消息", "icons/message.svg", colors)
                    // 按下时若面板开着，先拦下「面板外按下收起」，让 click 自己决定。
                    .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        if event.button == MouseButton::Left {
                            this.side_panel
                                .update(cx, |panel, _| panel.suppress_out_close());
                        }
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.side_panel
                            .update(cx, |panel, cx| panel.toggle(SidePanelContent::Messages, cx));
                    })),
            )
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
            .children(mini_and_window_buttons(
                "header-mini-button",
                window,
                colors,
            ))
    }
}

impl Render for MainContent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let active_page = self.active_page;
        let playlist_comments = matches!(
            active_page,
            ContentPage::FavoriteMusic | ContentPage::Playlist(_)
        ) && self.playlist_page.read(cx).comments_active(cx);
        let page_scroll = &self.page_scroll[&active_page];
        let page = match active_page {
            ContentPage::Recommend => Some(RecommendPage::content(
                self.recommend_page.clone(),
                window.viewport_size().width - self.sidebar_width - PAGE_SIDE_PADDING * 2.,
                window,
                cx,
            )),
            ContentPage::DailySongs => Some(self.daily_page.clone().into_any_element()),
            ContentPage::FavoriteMusic | ContentPage::Playlist(_) => Some(PlaylistPage::content(
                self.playlist_page.clone(),
                window.viewport_size().width - self.sidebar_width - px(80.),
                cx,
            )),
            ContentPage::SongComments(_) => {
                Some(self.song_comments_page.clone().into_any_element())
            }
            ContentPage::Search => Some(self.search_page.clone().into_any_element()),
            _ => self
                .pages
                .get(&active_page)
                .cloned()
                .map(IntoElement::into_any_element),
        };
        let drag_offset = Rc::new(Cell::new(px(0.)));
        // 搜索和评论页管理自己的滚动及裁剪留白，外层不再增加滚动容器。
        let full_bleed_comments =
            playlist_comments || matches!(active_page, ContentPage::SongComments(_));
        let page_scrolls = !matches!(active_page, ContentPage::Search) && !full_bleed_comments;

        div()
            // cached 会独立布局这棵树，根节点需要占满缓存容器。
            .size_full()
            .min_h(px(0.))
            .relative()
            .flex()
            .child(
                div().w(self.sidebar_width).h_full().flex_none().child(
                    self.sidebar
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .child(self.page_header(window, colors, cx))
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
                                    .when(!full_bleed_comments, |content| content.py(px(18.)))
                                    .when(page_scrolls, |content| {
                                        content
                                            .px(PAGE_SIDE_PADDING)
                                            .overflow_y_scroll()
                                            .track_scroll(page_scroll)
                                    })
                                    .children(page),
                            )
                            // 悬浮页头是页内内容，只是固定贴顶；和滚动条同属这一层，
                            // 排在滚动条之前，滚动条自然压在上面。
                            // （放外层不行：那边整棵树排在 main_content 之后，层级必然更高。）
                            .when(
                                !playlist_comments
                                    && matches!(
                                        active_page,
                                        ContentPage::FavoriteMusic | ContentPage::Playlist(_)
                                    ),
                                |content| {
                                    content.child(self.playlist_page.update(cx, |page, cx| {
                                        page.floating_header(self.playlist_backdrop.clone(), cx)
                                    }))
                                },
                            )
                            // 滚动条放在外层，避免它的边界被计入滚动内容高度。
                            .when(page_scrolls, |content| {
                                content.child(
                                    Scrollbar::vertical(page_scroll).mode(ScrollbarMode::Always),
                                )
                            }),
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
                            cx.emit(ContentLayoutChanged);
                            cx.notify();
                        },
                    )),
            )
    }
}

impl MainContent {
    /// 播放进度指示留在外层：它按页面内容区的裁剪框绘制（见 `PlayingIndicator`），
    /// 画不进滚动条那一条，没有遮挡问题，所以不必跟着悬浮页头搬进页面区。
    pub(super) fn playlist_overlays(&self, cx: &mut Context<Self>) -> Option<Div> {
        matches!(
            self.active_page,
            ContentPage::FavoriteMusic | ContentPage::Playlist(_)
        )
        .then(|| {
            div()
                .absolute()
                .left(self.sidebar_width)
                .right_0()
                .top(px(HEADER_HEIGHT))
                .bottom_0()
                .overflow_hidden()
                .child(self.playlist_page.read(cx).playing_overlay())
        })
    }
}
