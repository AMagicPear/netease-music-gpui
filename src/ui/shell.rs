mod content;

pub use content::MainContent;

use gpui::prelude::StatefulInteractiveElement;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use super::assets::{CoverPrefetch, track_cover_url};
use super::comments::ViewSongComments;
use super::components::{Collect, PLAYER_BAR_HEIGHT, PlayNext, PlayerBar, icon_hover_color};
use super::cover_color::{Backdrop, CoverColor, CoverGradient, gradient_layer};
use super::events::{ContentLayoutChanged, Navigate, NavigationBus, PlaylistMetadataChanged};
use super::now_playing::{NowPlaying, NowPlayingPage};
use super::pages::ContentPage;
use super::theme::{IconSize, PRESSED_ICON_ALPHA};
use crate::models::{Playlist, Song};
use crate::playback::{PlaybackController, PlaybackEvent};
use crate::state::library::MusicLibrary;

/// macOS 顶部给系统红绿灯留白；其余平台窗口按钮与工具栏同行，只留少量顶部空隙。
pub(super) const HEADER_TOP_INSET: f32 = if cfg!(target_os = "macos") { 30. } else { 8. };
pub(super) const HEADER_HEIGHT: f32 = HEADER_TOP_INSET + 42.;

pub struct MainWindow {
    pub main_content: Entity<MainContent>,
    pub player_bar: Entity<PlayerBar>,
    now_playing_page: Entity<NowPlayingPage>,
    /// Action 沿焦点祖先链派发，启动时由根视图持有焦点。
    focus_handle: FocusHandle,
    _nav_subscription: Subscription,
    _playback_subscription: Subscription,
    _background_subscriptions: [Subscription; 2],
    cover_tint: CoverColor,
    cover_prefetch: CoverPrefetch,
    playback: Entity<PlaybackController>,
}

impl MainWindow {
    pub fn new(
        window: &mut Window,
        main_content: Entity<MainContent>,
        player_bar: Entity<PlayerBar>,
        now_playing: Entity<NowPlaying>,
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        cx: &mut Context<Self>,
    ) -> Self {
        let now_playing_page =
            cx.new(|cx| NowPlayingPage::new(playback.clone(), library, now_playing, cx));
        let focus_handle = cx.focus_handle();
        let nav_subscription = cx.subscribe_in(
            &NavigationBus::global(cx),
            window,
            |this, _, intent: &Navigate, window, cx| match intent {
                Navigate::Playlist { id, summary } => {
                    this.open_playlist_nav(*id, summary.clone(), cx)
                }
                Navigate::SongComments(song) => this.open_song_comments(song.clone(), window, cx),
                Navigate::DailySongs => {
                    this.main_content.update(cx, |content, cx| {
                        content.navigate(ContentPage::DailySongs, cx)
                    });
                }
            },
        );
        // 封面预取只跟随歌曲和预加载候选，不处理位置推进。
        let playback_subscription = cx.subscribe(&playback, |this, _, event, cx| {
            if matches!(
                event,
                PlaybackEvent::SnapshotChanged | PlaybackEvent::PreloadChanged
            ) {
                let playback = this.playback.clone();
                this.sync_cover_prefetch(&playback, cx);
            }
        });
        let playlist_page = main_content.read(cx).playlist_page.clone();
        let background_subscriptions = [
            cx.subscribe(&main_content, |_, _, _: &ContentLayoutChanged, cx| {
                cx.notify()
            }),
            cx.subscribe(&playlist_page, |_, _, _: &PlaylistMetadataChanged, cx| {
                cx.notify()
            }),
        ];
        let mut this = Self {
            main_content,
            player_bar,
            now_playing_page,
            focus_handle,
            _nav_subscription: nav_subscription,
            _playback_subscription: playback_subscription,
            _background_subscriptions: background_subscriptions,
            cover_tint: CoverColor::default(),
            cover_prefetch: CoverPrefetch::default(),
            playback: playback.clone(),
        };
        this.sync_cover_prefetch(&playback, cx);
        this
    }

    /// 启动时给原生菜单的 Action 一个派发起点；无焦点时无法到达根 div 的处理器。
    /// 默认不是 tab stop，不改变 Tab 导航顺序。
    pub fn focus_root(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }

    fn open_song_comments(&mut self, song: Song, window: &mut Window, cx: &mut Context<Self>) {
        if self.now_playing_page.read(cx).is_expanded(cx) {
            self.now_playing_page
                .update(cx, |page, cx| page.toggle_comments_screen(cx));
        } else {
            self.main_content.update(cx, |content, cx| {
                content.open_song_comments(song, window, cx)
            });
        }
    }

    fn open_playlist_nav(&mut self, id: u64, summary: Option<Playlist>, cx: &mut Context<Self>) {
        self.main_content.update(cx, |content, cx| {
            if let Some(summary) = summary {
                content.open_playlist_detail(id, Some(summary), cx);
            } else {
                content.navigate(ContentPage::Playlist(id), cx);
            }
        });
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
        let playback = self.playback.clone();
        div()
            .track_focus(&self.focus_handle)
            .id("main-window")
            .on_action(move |action: &PlayNext, _window, cx| {
                cx.update_entity(&playback, |controller, cx| {
                    controller.insert_next(action.0.clone(), cx);
                });
            })
            .on_action(cx.listener(|this, action: &ViewSongComments, window, cx| {
                this.open_song_comments(action.0.clone(), window, cx);
            }))
            .on_action(cx.listener(|_, action: &Collect, window, cx| {
                super::components::open_collect_window(window, vec![action.0.clone()], cx);
            }))
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
                            .update(cx, |content, cx| content.song_list_overlays(cx)),
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
