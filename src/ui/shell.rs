use std::{cell::Cell, collections::HashMap, rc::Rc};

use gpui::prelude::{FluentBuilder, StatefulInteractiveElement};
use gpui::*;

use super::assets::{CoverPrefetch, thumbnail_url, track_cover_url};
use super::components::{
    PLAYER_BAR_HEIGHT, PlayerBar, ResizeDragPreview, SurfacePopover, icon_hover_color,
    window_drag_area,
};
use super::cover_color::{Backdrop, CoverColor, CoverGradient, gradient_layer};
use super::now_playing::{NowPlaying, NowPlayingPage};
use super::pages::{ContentPage, playlist::PlaylistPage};
use super::sidebar::{LoginRequested, SidebarChanged, SidebarPage};
use super::theme::{IconSize, PRESSED_ICON_ALPHA};
use crate::playback::PlaybackController;
use crate::state::account::AccountState;
use crate::state::library::MusicLibrary;
use gpui_kit::base::input::{Input, InputState};
use gpui_kit::base::{Button, ColorTokens, Scrollbar, ScrollbarMode, Theme};
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;

const MIN_SIDEBAR_WIDTH: Pixels = px(204.);
const MAX_SIDEBAR_WIDTH: Pixels = px(627.);
/// 搜索框的理想宽度
const SEARCH_BOX_WIDTH: Pixels = px(258.);
/// 搜索框的压缩下限
const SEARCH_BOX_MIN_WIDTH: Pixels = px(40.);
/// macOS 顶部给系统红绿灯留白，其余平台的窗口按钮与工具栏同行。
pub(super) const HEADER_TOP_INSET: f32 = if cfg!(target_os = "macos") { 30. } else { 0. };
pub(super) const HEADER_HEIGHT: f32 = HEADER_TOP_INSET + 42.;

pub struct MainWindow {
    pub main_content: Entity<MainContent>,
    pub player_bar: Entity<PlayerBar>,
    now_playing_page: Entity<NowPlayingPage>,
    _playback_subscription: Subscription,
    _background_subscriptions: Vec<Subscription>,
    cover_tint: CoverColor,
    cover_prefetch: CoverPrefetch,
}

impl MainWindow {
    pub fn new(
        main_content: Entity<MainContent>,
        player_bar: Entity<PlayerBar>,
        now_playing: Entity<NowPlaying>,
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        cx: &mut Context<Self>,
    ) -> Self {
        // 依赖全由 main.rs 这个组合根显式传入：shell 不再从播放栏里"扒"参数。
        // 黑胶页直接拿到同一个 NowPlaying——开合状态、共享渐变底、黑胶时钟都是它；
        // 展开/收起的接线（谁触发、谁跟随）全在各自 view 内部，shell 不再当中介。
        let now_playing_page =
            cx.new(|cx| NowPlayingPage::new(playback.clone(), library, now_playing, cx));
        let playback_subscription = cx.observe(&playback, |this, playback, cx| {
            this.sync_cover_prefetch(&playback, cx);
        });
        let playlist_page = main_content.read(cx).playlist_page.clone();
        let background_subscriptions = vec![
            cx.observe(&main_content, |_, _, cx| cx.notify()),
            cx.observe(&playlist_page, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            main_content,
            player_bar,
            now_playing_page,
            _playback_subscription: playback_subscription,
            _background_subscriptions: background_subscriptions,
            cover_tint: CoverColor::default(),
            cover_prefetch: CoverPrefetch::default(),
        };
        this.sync_cover_prefetch(&playback, cx);
        this
    }

    fn sync_cover_prefetch(
        &mut self,
        playback: &Entity<PlaybackController>,
        cx: &mut Context<Self>,
    ) {
        let urls = {
            let controller = playback.read(cx);
            let next = controller.preloading_song();
            [controller.snapshot().current_song.as_ref(), next.as_ref()]
                .into_iter()
                .flatten()
                .flat_map(|song| {
                    [80, 480].map(|pixels| track_cover_url(song.al.pic_url.as_deref(), pixels))
                })
                .collect()
        };
        self.cover_prefetch.sync(urls, cx);
    }

    fn playlist_tint(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Hsla> {
        let content = self.main_content.read(cx);
        if !matches!(
            content.active_page,
            ContentPage::FavoriteMusic | ContentPage::Playlist(_)
        ) {
            return None;
        }
        let url = content.playlist_page.read(cx).cover_url(cx)?;
        let hue = self
            .cover_tint
            .load(&url, window, cx)
            .filter(|color| color.s >= 0.08)?
            .h;
        let dark = Theme::global(cx).tokens.colors.background.l < 0.5;
        // HSV(h, 1, 1, .1) / HSV(h, .55, 1, .4)，换成 GPUI 原生 HSL。
        Some(hsla(
            hue,
            1.,
            if dark { 0.725 } else { 0.5 },
            if dark { 0.4 } else { 0.1 },
        ))
    }
}

fn playlist_background(tint: Option<Hsla>, backdrop: Backdrop) -> impl IntoElement {
    let tint = tint.unwrap_or_else(transparent_black);
    let target = CoverGradient([
        tint.into(),
        hsla(tint.h, 1., if tint.l > 0.5 { 0.825 } else { 0.5 }, 0.).into(),
    ]);
    // 始终保留同一层，进入、切换和离开歌单页都能连续过渡。
    gradient_layer("playlist-background-color", target, backdrop)
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(px(480.))
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let tint = self.playlist_tint(window, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(playlist_background(
                        tint,
                        self.main_content.read(cx).playlist_backdrop.clone(),
                    ))
                    .child(
                        self.main_content.clone().cached(
                            StyleRefinement::default()
                                .w_full()
                                // 页头是 MainContent 自己内部的第一行，所以从窗口顶部量起。
                                .h((window.viewport_size().height - px(PLAYER_BAR_HEIGHT))
                                    .max(px(0.)))
                                .flex_shrink(0.),
                        ),
                    )
                    // 保留歌单 overlay 在专辑页滑入层下方，避免打断悬浮标题的退场动画。
                    .children(
                        self.main_content
                            .update(cx, |content, cx| content.playlist_overlays(cx)),
                    )
                    .child(
                        self.now_playing_page
                            .clone()
                            .cached(StyleRefinement::default().absolute().inset_0().size_full()),
                    ),
            )
            .child(
                self.player_bar.clone().cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(px(PLAYER_BAR_HEIGHT))
                        .flex_shrink(0.),
                ),
            )
            .child(self.player_bar.read(cx).vinyl_overlay())
            .children(self.now_playing_page.read(cx).vinyl_overlays())
    }
}

pub struct MainContent {
    active_page: ContentPage,
    sidebar_width: Pixels,
    sidebar: Entity<SidebarPage>,
    search_input: Entity<InputState>,
    user_profile: Entity<AccountState>,
    login_window: Option<AnyWindowHandle>,
    /// 账号弹层（头像 popover）是否展开；未登录时点击头像走登录窗。
    profile_menu_open: bool,
    account_id: u64,
    /// 每个导航项对应一个页面 View，一次创建后长期持有。
    pages: HashMap<ContentPage, AnyView>,
    playlist_page: Entity<PlaylistPage>,
    library: Entity<MusicLibrary>,
    page_scroll: HashMap<ContentPage, ScrollHandle>,
    playlist_backdrop: Backdrop,
    _sidebar_subscription: Subscription,
    _sidebar_login_subscription: Subscription,
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
        // 侧边栏「立即登录云音乐」点击 → 打开登录窗（与头像入口同一套流程）。
        let login_subscription = cx.subscribe_in(
            &sidebar,
            window,
            |this, _, _: &LoginRequested, window, cx| {
                this.login_window =
                    super::login::open(window, this.user_profile.clone(), this.login_window, cx);
            },
        );
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
        let user_profile_subscription = cx.observe(&user_profile, |this, account, cx| {
            let user_id = account.read(cx).profile.user_id;
            if this.account_id != user_id {
                this.account_id = user_id;
                this.playlist_page
                    .update(cx, |page, cx| page.open(None, cx));
                this.navigate(ContentPage::Recommend, cx);
            }
            cx.notify();
        });

        Self {
            active_page: ContentPage::Recommend,
            sidebar_width: MIN_SIDEBAR_WIDTH,
            sidebar,
            search_input,
            user_profile,
            login_window: None,
            profile_menu_open: false,
            account_id: 0,
            pages,
            playlist_page,
            library,
            playlist_backdrop: Backdrop::default(),
            page_scroll: ContentPage::all()
                .map(|page| (page, ScrollHandle::default()))
                .collect(),
            _sidebar_subscription: sidebar_subscription,
            _sidebar_login_subscription: login_subscription,
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
        // 「是否我喜欢的音乐」在这里一次定死：显式导航算 true，
        // 传 id 进来的按 id 是否命中喜欢歌单判断。
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
            view.open(id, cx);
        });
    }
}

/// 搜索框：左边放大镜图标，右边文本输入框，宽度可被压缩
fn search_box(input: Entity<InputState>, colors: ColorTokens) -> impl IntoElement {
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
                // 同上：按下换用更浅的「按下色」，避免按住拖出时叠加 opacity 造成双重变淡
                .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA))),
        )
        .child(Input::new(&input))
}

pub(super) fn hover_icon(
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
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            svg()
                .path(path)
                .size(IconSize::Small.pixels())
                .flex_none()
                .text_color(colors.foreground.alpha(0.6))
                .hover(|style| {
                    style.text_color(icon_hover_color(colors.foreground.alpha(0.6), colors))
                })
                .id((id, 0usize))
                .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA))),
        )
}

/// 页头右端那三个窗口按钮：最小化 / 最大化或还原 / 关闭。
fn window_buttons(window: &Window, colors: ColorTokens) -> [Button; 3] {
    // 最大化后中间那个换成「向下还原」——官方客户端也是这么切的。
    let (maximize_icon, maximize_label) = if window.is_maximized() {
        ("icons/window/restore.svg", "向下还原")
    } else {
        ("icons/window/maximize.svg", "最大化")
    };
    [
        hover_icon(
            "header-minimize-button",
            "最小化",
            "icons/window/minimize.svg",
            colors,
        )
        .on_click(|_, window, _| window.minimize_window()),
        hover_icon(
            "header-maximize-button",
            maximize_label,
            maximize_icon,
            colors,
        )
        .on_click(|_, window, _| window.zoom_window()),
        hover_icon(
            "header-close-button",
            "关闭",
            "icons/window/close.svg",
            colors,
        )
        .on_click(|_, window, cx| crate::desktop::close_window(window, cx)),
    ]
}

/// 主界面和歌词页共用的窗口按钮组。
pub(super) fn mini_and_window_buttons(
    mini_id: &'static str,
    window: &Window,
    colors: ColorTokens,
) -> Vec<Button> {
    let mut buttons = vec![hover_icon(
        mini_id,
        "迷你模式",
        "icons/menu_mini.svg",
        colors,
    )];
    if !cfg!(target_os = "macos") {
        buttons.extend(window_buttons(window, colors));
    }
    buttons
}

impl MainContent {
    /// 页头右端的头像与昵称；点击弹出账号菜单（官方同款 popover）。
    /// 未登录时点击仍是打开登录窗，已登录时开合弹层。
    fn profile_menu(&self, colors: ColorTokens, cx: &Context<Self>) -> impl IntoElement {
        let profile = self.user_profile.read(cx);
        let logged_in = profile.profile.user_id != 0;
        let was_open = self.profile_menu_open;
        let nickname = profile.profile.nickname.clone();
        let avatar_url = profile.profile.avatar_url.clone();
        let vip_badge = profile.vip.as_ref().and_then(|vip| {
            vip.badge_path(time::OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000)
        });
        // 弹层与触发器各自渲染一份（同一账号数据），克隆两份避免移动冲突。
        let popover_nickname = nickname.clone();
        let popover_avatar_url = avatar_url.clone();
        let popover_vip_badge = vip_badge.clone();

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
                        this.login_window = super::login::open(
                            window,
                            this.user_profile.clone(),
                            this.login_window,
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                // 未登录 / 无头像时用官方默认头像位图（avatarMiniBar，灰圆人形）；
                // 不走 gpui-kit 的内置占位图标：它继承 base 的 text_color（背景色），
                // 在浅灰圆上几乎不可见。有头像才走 CDN 缩略图。
                if avatar_url.is_empty() {
                    Avatar::new()
                        .with_size(px(28.))
                        .flex_none()
                        .src("images/default_avatar.png")
                        .into_any_element()
                } else {
                    Avatar::new()
                        .with_size(px(28.))
                        .flex_none()
                        .src(thumbnail_url(&avatar_url, 56))
                        .into_any_element()
                },
            )
            .child(
                div()
                    // 把整块（昵称 + VIP + 箭头）声明成一个 group，
                    // 子元素就能用 group_hover 感知「整块是否被悬浮」。
                    .group("header-profile-menu")
                    .ml(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(13.))
                    // 昵称保持原本设计的 0.7；hover 一并让昵称变深到 foreground。
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

        // ml_auto 是 header flex 行里的布局属性（wrapper 层），组件只管浮层本体。
        div().ml_auto().flex_none().child(
            SurfacePopover::new("header-profile-popover", logged_in && was_open, trigger)
                .anchor(Anchor::TopRight)
                .offset(px(8.))
                .w(px(240.))
                .py(px(6.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.profile_menu_open = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(12.))
                        .py(px(8.))
                        .child(
                            // 弹层里的头像展示更大一号，取 72px 缩略图。
                            if popover_avatar_url.is_empty() {
                                Avatar::new()
                                    .with_size(px(36.))
                                    .flex_none()
                                    .src("images/default_avatar.png")
                                    .into_any_element()
                            } else {
                                Avatar::new()
                                    .with_size(px(36.))
                                    .flex_none()
                                    .src(thumbnail_url(&popover_avatar_url, 72))
                                    .into_any_element()
                            },
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .line_clamp(1)
                                        .child(popover_nickname),
                                )
                                .when_some(popover_vip_badge, |col, badge| {
                                    col.child(img(badge).h(px(14.)).w_auto())
                                }),
                        ),
                )
                .child(div().h(px(1.)).bg(colors.border))
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
                        .text_size(px(13.))
                        .text_color(colors.foreground.alpha(0.8))
                        .hover(|style| style.bg(colors.muted).text_color(colors.foreground))
                        .child(
                            svg()
                                .path("icons/logout.svg")
                                .size(px(15.))
                                .flex_none()
                                .text_color(colors.foreground.alpha(0.8))
                                .group_hover("header-profile-logout", |style| {
                                    style.text_color(colors.foreground)
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
                    .child(
                        svg()
                            .path("icons/backward.svg")
                            .size(px(11.))
                            .text_color(colors.secondary_foreground),
                    ),
            )
            .child(search_box(self.search_input.clone(), colors))
            .child(self.profile_menu(colors, cx))
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
        let page_scroll = &self.page_scroll[&active_page];
        let page = match active_page {
            ContentPage::FavoriteMusic | ContentPage::Playlist(_) => Some(PlaylistPage::content(
                self.playlist_page.clone(),
                window.viewport_size().width - self.sidebar_width - px(80.),
                cx,
            )),
            _ => self
                .pages
                .get(&active_page)
                .cloned()
                .map(IntoElement::into_any_element),
        };
        let drag_offset = Rc::new(Cell::new(px(0.)));

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

impl MainContent {
    /// 动画和实时背景与缓存内容同级，避免它们每帧使侧栏和歌单缓存失效。
    fn playlist_overlays(&self, cx: &mut Context<Self>) -> Option<Div> {
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
                .child(self.playlist_page.update(cx, |page, cx| {
                    page.floating_header(self.playlist_backdrop.clone(), cx)
                }))
        })
    }
}
