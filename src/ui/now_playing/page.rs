//! 黑胶全屏页：外层滚动、背景、元信息与唱片协调。

mod comments;
mod lyrics;

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use super::NowPlaying;
use crate::playback::PlaybackController;
use crate::state::library::MusicLibrary;
use crate::ui::assets::track_cover_url;
use crate::ui::components::{PLAYER_BAR_HEIGHT, Tonearm, Vinyl, format_duration, window_drag_area};
use crate::ui::cover_color::{
    Backdrop, CoverColor, CoverGradient, dark_colors, dark_gradient, gradient_layer,
};
use crate::ui::shell::{HEADER_HEIGHT, HEADER_TOP_INSET, hover_icon, mini_and_window_buttons};
use comments::{CommentsView, ListMode};
use lyrics::LyricsView;

const SUMMARY_HEIGHT: f32 = 72.;
const SONG_CONTENT_MAX_WIDTH: f32 = 1400.;
const SONG_CONTENT_PADDING: f32 = 100.;
const SONG_CONTENT_TOP_PADDING: f32 = 24.;
const VINYL_MAX_SIDE: f32 = 540.;
const CONTENT_MAX_ASPECT: f32 = 1.6;
const SUMMARY_WIDTH_RATIO: f32 = 0.8;
/// 判定外层「翻到评论区」的容差。滚动边界把 `scroll_max` 圆到两位小数、
/// 页高又可能是小数，差之毫厘不该让评论区在最后 1px 上反复开关。
const SCROLL_END_TOLERANCE: f32 = 1.;
/// 「回到唱片」把外层滑回歌词页的时长。和整页展开 / 收起同一档节拍。
const RETURN_DURATION: Duration = Duration::from_millis(600);
/// 补间的收敛容差，同时也是「偏移被谁动过」的判定门限。
const TWEEN_EPSILON: f32 = 0.5;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SongTab {
    #[default]
    Lyrics,
    Encyclopedia,
    Similar,
}

/// 外层滚动的补间，只服务「回到唱片」这一条程序化路径。
///
/// 其余位移全走原生滚动，补间到站就彻底放手：目标留着不清，下一次单靠
/// 「目标没变」会被挡掉，人就回不去了。跑动中一旦发现偏移被别的来源改过
/// （滚轮、原生惯性、窗口补位），立刻让位——手永远优先于动画。
#[derive(Default)]
struct ScrollTween {
    /// 目标偏移；`None` 表示不接管。
    target: Option<Pixels>,
    /// 起点与起始时刻。
    playing: Option<(f32, Instant)>,
    /// 上一帧写进去的值，用来认「谁动了偏移」。
    written: Option<f32>,
}

impl ScrollTween {
    /// 瞄准一个目标偏移；已经在目标上就什么都不做。
    fn aim(&mut self, handle: &ScrollHandle, target: Pixels, now: Instant) {
        if self.target == Some(target) {
            return;
        }
        let from = f32::from(handle.offset().y);
        if (f32::from(target) - from).abs() <= TWEEN_EPSILON {
            *self = Self::default();
            return;
        }
        self.target = Some(target);
        self.playing = Some((from, now));
        self.written = Some(from);
    }

    fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    /// 推进补间；还在飞就顺带要下一帧。
    fn step(&mut self, handle: &ScrollHandle, window: &mut Window, now: Instant) -> bool {
        let playing = self.advance(handle, now);
        if playing {
            window.request_animation_frame();
        }
        playing
    }

    fn advance(&mut self, handle: &ScrollHandle, now: Instant) -> bool {
        let (Some((from, started)), Some(target)) = (self.playing, self.target) else {
            return false;
        };
        // 上一帧写下的偏移被改过：有人接管了滚动，补间让位。
        if let Some(written) = self.written
            && (f32::from(handle.offset().y) - written).abs() > TWEEN_EPSILON
        {
            *self = Self::default();
            return false;
        }
        let progress = (now.saturating_duration_since(started).as_secs_f32()
            / RETURN_DURATION.as_secs_f32())
        .min(1.);
        let y = from + (f32::from(target) - from) * ease_out_quint()(progress);
        handle.set_offset(point(px(0.), px(y)));
        self.written = Some(y);
        if progress < 1. {
            return true;
        }
        *self = Self::default();
        false
    }
}

pub(in crate::ui) struct NowPlayingPage {
    playback: Entity<PlaybackController>,
    library: Entity<MusicLibrary>,
    _library_subscription: Subscription,
    _playback_subscription: Subscription,
    /// 展开/收起的单一状态源。本页不再自持 `opened`，开合同步也靠它广播。
    now_playing: Entity<NowPlaying>,
    _now_playing_subscription: Subscription,
    lyrics: Entity<LyricsView>,
    comments: Entity<CommentsView>,
    vinyl: Entity<Vinyl>,
    summary_vinyl: Entity<Vinyl>,
    scroll: ScrollHandle,
    /// 「回到唱片」的补间。只动外层，评论列表原地不动。
    scroll_tween: ScrollTween,
    cover_color: CoverColor,
    backdrop: Backdrop,
    song_id: Option<u64>,
    tonearm: Tonearm,
    tab: SongTab,
    page_height: Option<f32>,
}

struct FrameData {
    page_height: f32,
    height: f32,
    content_width: f32,
    side: f32,
    summary_top: f32,
    show_comments: bool,
    song_at_rest: bool,
    /// 评论列表这一帧听谁的，见 [`ListMode`]。
    list_mode: ListMode,
    reveal: f32,
    colors: ColorTokens,
    gradient: CoverGradient,
    title: String,
    artists: String,
    album: String,
    source: Option<String>,
    duration: String,
}

impl NowPlayingPage {
    pub(in crate::ui) fn new(
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        now_playing: Entity<NowPlaying>,
        cx: &mut Context<Self>,
    ) -> Self {
        // 共享渐变底与旋转时钟都由 NowPlaying 持有：本页绘制/转动时写、播放栏与进度条读，
        // 句柄共享同一份，迷你碟切到大碟时相位才连续。
        let backdrop = now_playing.read(cx).backdrop();
        let clock = now_playing.read(cx).rotation_clock();
        let vinyl = cx.new(|cx| Vinyl::new(playback.clone(), clock.clone(), "images/disc.png", cx));
        let summary_vinyl =
            cx.new(|cx| Vinyl::new(playback.clone(), clock, "images/miniVinyl.png", cx));
        let scroll = ScrollHandle::default();
        let lyrics = cx.new(|cx| LyricsView::new(playback.clone(), backdrop.clone(), cx));
        // 评论区自己吃不下滚轮位移时让事件冒泡，交给外层平台滚动。
        let comments = cx.new(|_| CommentsView::new(backdrop.clone()));
        let mut source = source_name(&library, &playback, cx);
        let library_playback = playback.clone();
        let library_subscription = cx.observe(&library, move |this, library, cx| {
            let next = source_name(&library, &library_playback, cx);
            if next != source {
                source = next;
                if this.is_expanded(cx) {
                    cx.notify();
                }
            }
        });
        let mut playback_state = {
            let state = playback.read(cx).snapshot();
            (
                state.revision,
                state.is_playing,
                playback.read(cx).playlist_id(),
            )
        };
        let playback_subscription = cx.observe(&playback, move |this, playback, cx| {
            let state = playback.read(cx).snapshot();
            let next = (
                state.revision,
                state.is_playing,
                playback.read(cx).playlist_id(),
            );
            let song_id = state.current_song.as_ref().map(|song| song.id);
            let changed = this.song_id != song_id;
            if this.is_expanded(cx) {
                this.sync_song(cx);
            } else if changed && this.song_id.is_some() {
                // 隐藏后切歌只取消旧请求；下一次 open 再请求实际歌曲。
                this.song_id = None;
                this.lyrics.update(cx, |view, cx| view.set_song(None, cx));
                this.comments.update(cx, |view, cx| view.set_song(None, cx));
            }
            if this.is_expanded(cx) && (changed || next != playback_state) {
                cx.notify();
            }
            playback_state = next;
        });
        // 展开/收起的状态迁移由 NowPlaying 广播；本页在这里跑对应的副作用，
        // 播放栏、进度条、音量则各自 observe 它来重绘。
        //
        // 这个 `expanded` 守卫是必需的，不能省：NowPlaying 还会用 `publish_backdrop`
        // 每帧广播背景渐变，那些通知同样会打到这里。少了守卫，`on_expand` 会在整个
        // 600ms 渐变期间被反复触发，`reset_page()` 每帧复位一次，页面看起来就是
        // "自己弹回顶部"。这里只认状态位的边沿，渐变通知一律放过。
        let mut expanded = now_playing.read(cx).is_expanded();
        let now_playing_subscription = cx.observe(&now_playing, move |this, now_playing, cx| {
            let next = now_playing.read(cx).is_expanded();
            if next == expanded {
                return;
            }
            expanded = next;
            if next {
                this.on_expand(cx);
            } else {
                this.on_collapse(cx);
            }
        });
        Self {
            playback,
            library,
            _library_subscription: library_subscription,
            _playback_subscription: playback_subscription,
            now_playing,
            _now_playing_subscription: now_playing_subscription,
            lyrics,
            comments,
            vinyl,
            summary_vinyl,
            scroll,
            scroll_tween: ScrollTween::default(),
            cover_color: CoverColor::default(),
            backdrop,
            song_id: None,
            tonearm: Tonearm::default(),
            tab: SongTab::default(),
            page_height: None,
        }
    }

    fn sync_song(&mut self, cx: &mut Context<Self>) {
        let id = self
            .playback
            .read(cx)
            .snapshot()
            .current_song
            .as_ref()
            .map(|song| song.id);
        if self.song_id == id {
            return;
        }
        self.song_id = id;
        self.lyrics.update(cx, |view, cx| view.set_song(id, cx));
        self.comments.update(cx, |view, cx| view.set_song(id, cx));
        self.reset_page();
    }

    /// 回到歌词页：换歌、重新打开这一页都从这里起步。
    fn reset_page(&mut self) {
        self.scroll_tween = ScrollTween::default();
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    /// 「回到唱片」：外层滑回歌词页。
    ///
    /// 只有外层偏移在动——评论列表保持自己的位置，整页连着评论区一起升上去，
    /// 中途不会有内容跳变。补间期间滚轮归谁见 [`ListMode::Held`]。
    fn return_to_song(&mut self, cx: &mut Context<Self>) {
        if cx.reduce_motion() {
            self.scroll.set_offset(point(px(0.), px(0.)));
        } else {
            let now = cx.background_executor().now();
            self.scroll_tween.aim(&self.scroll, px(0.), now);
        }
        cx.notify();
    }

    /// 这一页是否展开。只读单一状态源，页面自己不再存副本。
    fn is_expanded(&self, cx: &App) -> bool {
        self.now_playing.read(cx).is_expanded()
    }

    pub(in crate::ui) fn vinyl_overlays(&self) -> [Entity<Vinyl>; 2] {
        [self.vinyl.clone(), self.summary_vinyl.clone()]
    }

    /// 展开时的副作用：回到歌词页起点并同步当前歌曲。
    fn on_expand(&mut self, cx: &mut Context<Self>) {
        self.reset_page();
        self.sync_song(cx);
        cx.notify();
    }

    /// 收起时的副作用：停掉歌词跟随、复位评论区，和展开时对称。
    fn on_collapse(&mut self, cx: &mut Context<Self>) {
        self.scroll_tween = ScrollTween::default();
        self.lyrics
            .update(cx, |view, cx| view.set_active(false, cx));
        self.comments
            .update(cx, |view, cx| view.configure(ListMode::Parked, cx));
        cx.notify();
    }

    fn frame_data(&mut self, window: &mut Window, cx: &mut Context<Self>) -> FrameData {
        let expanded = self.now_playing.read(cx).is_expanded();
        let cover_url = self
            .playback
            .read(cx)
            .snapshot()
            .current_song
            .as_ref()
            .map(|song| track_cover_url(song.al.pic_url.as_deref(), 480));
        let color = if expanded {
            cover_url
                .as_deref()
                .and_then(|url| self.cover_color.load(url, window, cx))
        } else {
            self.cover_color.current()
        };
        let gradient = dark_gradient(color);
        let colors = dark_colors(Theme::global(cx).tokens.colors, gradient.0[1].into());
        let song = self.playback.read(cx).snapshot().current_song.clone();
        let page_height = (f32::from(window.viewport_size().height) - PLAYER_BAR_HEIGHT).max(1.);
        let height = (page_height - HEADER_HEIGHT).max(1.);
        // 展开/收起的节拍来自 NowPlaying：id、时长、曲线都只有它一处定义。
        let reveal = NowPlaying::reveal(expanded, window, cx);
        // 摘要条此刻在屏幕上的位置，唱片 / 摘要唱片的绘制用它做连续性判断。
        let summary_top = page_height + HEADER_TOP_INSET + f32::from(self.scroll.offset().y);
        // 「在不在评论区」从滚动位置推导：滚过半屏就算。原生滚动每一步都会
        // 通知本页重绘，这个值跟着滚动连续更新。
        let scrolled = -f32::from(self.scroll.offset().y);
        let show_comments = scrolled >= page_height * 0.5;
        // 两屏各有一个静止点，滚轮只归「已经停住」的那一屏，路上（两个静止点之间）
        // 一律归外层。判定都从偏移算：0 是歌词页，页高是评论区。
        //
        // 这不是洁癖——两屏的嵌套滚动都抢在指针前面注册监听，只要露出一点、
        // 指针又在它上面，就会把位移截走，外层这一屏从此停在中途，再也翻不完。
        let song_at_rest = scrolled <= SCROLL_END_TOLERANCE;
        // 评论列表这一帧听谁的。补间插在最前面：外层正在飞的那几百毫秒里，
        // 「到没到位」每帧都在翻，不能让它决定列表的去留——否则滚动条会闪一下，
        // 列表也会自己跳回顶部。
        let list_mode = if self.scroll_tween.is_playing() {
            ListMode::Held
        } else if expanded && reached_comments(scrolled, page_height) {
            ListMode::Owned
        } else {
            ListMode::Parked
        };
        let content_height = (height - SONG_CONTENT_TOP_PADDING).max(1.);
        let content_width = (f32::from(window.viewport_size().width) - SONG_CONTENT_PADDING * 2.)
            .min(SONG_CONTENT_MAX_WIDTH)
            .min(content_height * CONTENT_MAX_ASPECT)
            .max(1.);
        let side = (content_width * 0.4).min(VINYL_MAX_SIDE).floor().max(1.);
        let title = song
            .as_ref()
            .map(|song| song.name.clone())
            .unwrap_or_else(|| "暂无正在播放的歌曲".into());
        let artists = song
            .as_ref()
            .map(|song| {
                song.ar
                    .iter()
                    .filter_map(|artist| artist.name.as_deref())
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default();
        let album = song
            .as_ref()
            .and_then(|song| song.al.name.as_deref())
            .unwrap_or("未知专辑")
            .to_owned();
        let source = source_name(&self.library, &self.playback, cx);
        let duration = song
            .as_ref()
            .map(|song| format_duration(song.duration()))
            .unwrap_or_default();
        FrameData {
            page_height,
            height,
            content_width,
            side,
            summary_top,
            show_comments,
            song_at_rest,
            list_mode,
            reveal,
            colors,
            gradient,
            title,
            artists,
            album,
            source,
            duration,
        }
    }

    fn render_summary(
        &self,
        frame: &FrameData,
        summary_vinyl: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new("album-song-summary")
            .aria_label("回到唱片")
            .debug_selector(|| "album-song-summary".into())
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                // 「回到唱片」：外层滑回歌词页，评论列表原地不动。
                this.return_to_song(cx);
            }))
            .max_w(px(frame.content_width * SUMMARY_WIDTH_RATIO))
            .h(px(58.))
            .pl(px(5.5))
            .pr(px(16.))
            .flex()
            .items_center()
            .gap_3()
            .rounded_full()
            .border_1()
            .border_color(white().alpha(0.1))
            .bg(white().alpha(0.04))
            .block_mouse_except_scroll()
            .child(summary_vinyl)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .line_height(relative(1.5))
                    .child(
                        div()
                            .debug_selector(|| "album-summary-title".into())
                            .min_w_0()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white())
                            .truncate()
                            .child(frame.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(frame.colors.muted_foreground)
                            .child("—"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "album-summary-artists".into())
                            .max_w(px(240.))
                            .text_size(px(13.))
                            .text_color(frame.colors.muted_foreground)
                            .truncate()
                            .child(frame.artists.clone()),
                    ),
            )
            .child(
                svg()
                    .path("icons/unfold.svg")
                    .size(px(16.))
                    .flex_none()
                    .text_color(white().alpha(0.6))
                    .with_transformation(Transformation::rotate(radians(std::f32::consts::PI))),
            )
            .into_any_element()
    }

    fn render_tab(&self, frame: &FrameData) -> AnyElement {
        match self.tab {
            SongTab::Lyrics => div()
                .flex_1()
                .min_h_0()
                .relative()
                // cached 是独立布局根，尺寸同时写在缓存 refinement 与子视图根。
                .child(
                    self.lyrics
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                )
                .into_any_element(),
            SongTab::Encyclopedia => div()
                .id("song-encyclopedia-content")
                .debug_selector(|| "song-encyclopedia-content".into())
                .flex_1()
                .min_h_0()
                .pt(px(32.))
                .flex()
                .flex_col()
                .gap_4()
                .text_size(px(14.))
                .text_color(frame.colors.muted_foreground)
                .children(
                    [
                        ("歌曲", frame.title.clone()),
                        ("歌手", frame.artists.clone()),
                        ("专辑", frame.album.clone()),
                        ("时长", frame.duration.clone()),
                    ]
                    .into_iter()
                    .map(|(label, value)| div().child(format!("{label}：{value}"))),
                )
                .into_any_element(),
            SongTab::Similar => div()
                .id("song-similar-content")
                .debug_selector(|| "song-similar-content".into())
                .flex_1()
                .min_h_0()
                .pt(px(80.))
                .text_color(frame.colors.muted_foreground)
                .child("相似推荐暂不可用")
                .into_any_element(),
        }
    }

    fn render_tab_bar(&self, frame: &FrameData, cx: &mut Context<Self>) -> AnyElement {
        let colors = frame.colors;
        div()
            .id("album-song-tabs")
            .debug_selector(|| "album-song-tabs".into())
            .mt(px(24.))
            .self_start()
            .flex_none()
            .h(px(32.))
            .p(px(3.))
            .flex()
            .items_center()
            .rounded_full()
            .bg(white().alpha(0.06))
            .text_size(px(14.))
            .children(
                [
                    (SongTab::Lyrics, "song-lyrics", "歌词"),
                    (SongTab::Encyclopedia, "song-encyclopedia", "百科"),
                    (SongTab::Similar, "song-similar", "相似推荐"),
                ]
                .into_iter()
                .map(|(tab, id, label)| {
                    let selected = self.tab == tab;
                    Button::new(id)
                        .debug_selector(move || id.into())
                        .selected(selected)
                        .aria_label(label)
                        .h(px(26.))
                        .px(px(9.))
                        .rounded_full()
                        .border_1()
                        .border_color(transparent_black())
                        .cursor_pointer()
                        .text_color(if selected {
                            white()
                        } else {
                            colors.muted_foreground
                        })
                        .when(selected, |button| button.bg(white().alpha(0.12)))
                        .focus_visible(|style| style.border_color(white().alpha(0.5)))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tab = tab;
                            cx.notify();
                        }))
                }),
            )
            .into_any_element()
    }

    fn render_song_info(
        &self,
        frame: &FrameData,
        tab_content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = frame.colors;
        div()
            .id("album-lyrics-region")
            .debug_selector(|| "album-lyrics-region".into())
            .w(relative(0.5))
            .h_full()
            .min_w_0()
            .pl(px(24.))
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(26.))
                    .line_height(px(34.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white())
                    .line_clamp(2)
                    .child(frame.title.clone()),
            )
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .gap_4()
                    .text_size(px(14.))
                    .text_color(colors.muted_foreground)
                    .child(
                        div()
                            .debug_selector(|| "album-metadata-album".into())
                            .min_w_0()
                            .truncate()
                            .child(format!("专辑：{}", frame.album)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "album-metadata-artists".into())
                            .min_w_0()
                            .truncate()
                            .child(format!("歌手：{}", frame.artists)),
                    )
                    .when_some(frame.source.clone(), |metadata, source| {
                        metadata.child(
                            div()
                                .debug_selector(|| "album-metadata-source".into())
                                .min_w_0()
                                .truncate()
                                .child(format!("来源：{source}")),
                        )
                    }),
            )
            .child(self.render_tab_bar(frame, cx))
            .child(tab_content)
            .into_any_element()
    }

    /// 收起整页的按钮。
    ///
    /// 放在页头左端，和右端那组图标同一行——页头只有一块，所以它不需要再单占一行。
    fn collapse_button(&self, frame: &FrameData, cx: &mut Context<Self>) -> Button {
        hover_icon(
            "close-album-lyrics",
            "收起专辑歌词页",
            "icons/unfold.svg",
            frame.colors,
        )
        .ml_0()
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            this.now_playing
                .update(cx, |now_playing, cx| now_playing.collapse(cx));
        }))
    }

    /// 页头。整块自带顶部留白（`pt`），所以只有一块：左端收起，右端播放器模式 + 图标组。
    fn page_header(
        &self,
        frame: &FrameData,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        window_drag_area("album-lyrics-header")
            .absolute()
            .top_0()
            .left_0()
            .h(px(HEADER_HEIGHT))
            .pt(px(HEADER_TOP_INSET))
            .w_full()
            .bg(transparent_black())
            .px(px(40.))
            .flex()
            .items_center()
            .child(self.collapse_button(frame, cx))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .when(!frame.show_comments, |group| {
                        group.child(
                            div()
                                .text_color(frame.colors.muted_foreground)
                                .text_size(px(13.))
                                .child("播放器模式"),
                        )
                    })
                    .children(mini_and_window_buttons(
                        "album-header-mini-button",
                        window,
                        frame.colors,
                    )),
            )
            .into_any_element()
    }
}

impl Render for NowPlayingPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 「回到唱片」的补间先走一步：它写的就是这一帧要画的偏移。
        let now = cx.background_executor().now();
        self.scroll_tween.step(&self.scroll, window, now);
        let page_height = (f32::from(window.viewport_size().height) - PLAYER_BAR_HEIGHT).max(1.);
        let previous_height = self.page_height.replace(page_height);
        if let Some(previous_height) = previous_height
            && previous_height != page_height
            // 补间在飞时不动偏移：目标就是歌词页静止点，窗口变了它照样对。
            && !self.scroll_tween.is_playing()
            && reached_comments(-f32::from(self.scroll.offset().y), previous_height)
        {
            // 窗口变高时原生滚动不会自动补位：原来停在评论页静止点，
            // 就直接挪到新的评论页静止点。
            self.scroll.set_offset(point(px(0.), px(-page_height)));
        }
        let frame = self.frame_data(window, cx);
        let expanded = self.now_playing.read(cx).is_expanded();
        // Backdrop 在 prepaint 写入实际插值色；其下一帧通知让缓存 fade 重新绘制。
        let backdrop_gradient = self.backdrop.gradient();
        // 把共享底的最新插值广播给播放栏/进度条：它们据此重新取色，配色才能跟着
        // 封面色渐变一帧帧走（渐变由 gradient_layer 在 prepaint 写入，这里晚一帧读到）。
        self.now_playing
            .update(cx, |now_playing, cx| now_playing.publish_backdrop(cx));
        self.lyrics.update(cx, |view, cx| {
            view.configure_backdrop(backdrop_gradient, cx)
        });
        self.lyrics.update(cx, |view, cx| {
            // 只有停在歌词页时歌词才吃滚轮（并恢复跟随播放）；翻页路上松手，
            // 否则指针恰好压在歌词上就把这一屏卡在半路了。
            view.set_active(
                expanded && self.tab == SongTab::Lyrics && frame.song_at_rest,
                cx,
            )
        });
        self.comments.update(cx, |view, cx| {
            // 评论列表这一帧听谁的，见 `ListMode`。
            view.configure(frame.list_mode, cx)
        });
        self.vinyl.read(cx).clear();
        self.summary_vinyl.read(cx).clear();
        if !expanded && frame.reveal <= 0. {
            self.tonearm = Tonearm::default();
            return div().into_any_element();
        }
        let arm_angle = {
            let playback = self.playback.read(cx).snapshot();
            self.tonearm
                .angle(playback.is_playing, playback.revision, window, cx)
        };
        let vinyl = (frame.reveal > 0. && frame.summary_top > 0.).then(|| {
            self.vinyl
                .read(cx)
                .placeholder(frame.side, 1., Some(arm_angle))
                .debug_selector(|| "album-vinyl-image".into())
        });
        let summary_vinyl = if frame.summary_top < frame.page_height {
            self.summary_vinyl.read(cx).placeholder(45., 1., None)
        } else {
            div().size(px(45.)).flex_none()
        }
        .debug_selector(|| "album-summary-vinyl".into());
        let tab_content = self.render_tab(&frame);
        let song_info = self.render_song_info(&frame, tab_content, cx);
        let comments = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .max_w(px(1120.))
            .mx_auto()
            .child(
                self.comments
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            );
        let summary = self.render_summary(&frame, summary_vinyl, cx);
        let song_page = build_song_page(&frame, song_info, vinyl);

        // 滑动层放在缓存根的子节点，百分比 top 才以全页为参照。
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(relative(1. - frame.reveal))
                    .size_full()
                    .flex()
                    .flex_col()
                    .bg(frame.colors.background)
                    .occlude()
                    .child(
                        gradient_layer(
                            "album-background-color",
                            frame.gradient,
                            self.backdrop.clone(),
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("now-playing-scroll")
                                    .debug_selector(|| "now-playing-scroll".into())
                                    .size_full()
                                    .pt(px(HEADER_HEIGHT))
                                    // 外层是普通平台滚动：两屏按文档流上下排布，滚到第二屏就是评论区。
                                    // 嵌套的歌词 / 评论列表自己决定什么时候拦位移，到边自然交回这里。
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .child(song_page)
                                    .child(
                                        div()
                                            .id("album-comments-page")
                                            .debug_selector(|| "album-comments-page".into())
                                            .w_full()
                                            .h(px(frame.page_height))
                                            .pt(px(HEADER_TOP_INSET))
                                            .flex()
                                            .flex_col()
                                            .overflow_hidden()
                                            .child(
                                                div()
                                                    .w_full()
                                                    .h(px(SUMMARY_HEIGHT))
                                                    .flex_none()
                                                    .px(px(40.))
                                                    .flex()
                                                    .justify_center()
                                                    .child(summary),
                                            )
                                            .child(comments),
                                    ),
                            )
                            .when(frame.show_comments, |content| {
                                content.child(
                                    div()
                                        .absolute()
                                        .bottom(px(16.))
                                        .left(relative(0.5))
                                        .ml(px(-76.))
                                        .child(
                                            div()
                                                .relative()
                                                .w(px(152.))
                                                .h(px(40.))
                                                .child(
                                                    canvas(
                                                        |_, _, _| (),
                                                        |bounds, _, window, _| {
                                                            window.paint_backdrop_blur(
                                                                bounds,
                                                                px(12.),
                                                                px(20.).into(),
                                                            );
                                                        },
                                                    )
                                                    .absolute()
                                                    .inset_0(),
                                                )
                                                .child(
                                                    Button::new("publish-song-comment")
                                                        .disabled(true)
                                                        .w(px(152.))
                                                        .h(px(40.))
                                                        .rounded_full()
                                                        .bg(linear_gradient(
                                                            180.,
                                                            linear_color_stop(
                                                                white().alpha(0.28),
                                                                0.,
                                                            ),
                                                            linear_color_stop(
                                                                white().alpha(0.14),
                                                                1.,
                                                            ),
                                                        ))
                                                        .border_1()
                                                        .border_color(white().alpha(0.10))
                                                        .text_color(white())
                                                        .child("发布评论"),
                                                ),
                                        ),
                                )
                            }),
                    )
                    .child(self.page_header(&frame, window, cx)),
            )
            .into_any_element()
    }
}

/// 外层这一屏翻完了没有。
///
/// 两屏的页高都等于窗口可视高度，内容总高正好是两倍页高，所以外层的滚动尽头
/// 就是「评论区顶对齐到视口顶」，也就是评论区整屏到位的那一刻。整页只有这里
/// 把滚轮交给评论列表，[`ListMode::Owned`] 和窗口变高时的补位都读它。
fn reached_comments(scrolled: f32, page_height: f32) -> bool {
    scrolled >= page_height - SCROLL_END_TOLERANCE
}

fn source_name(
    library: &Entity<MusicLibrary>,
    playback: &Entity<PlaybackController>,
    cx: &App,
) -> Option<String> {
    let id = playback.read(cx).playlist_id()?;
    library
        .read(cx)
        .playlists
        .iter()
        .find(|playlist| playlist.id == id)
        .map(|playlist| playlist.name.clone())
}

fn build_song_page(frame: &FrameData, song_info: AnyElement, vinyl: Option<Div>) -> AnyElement {
    div()
        .h(px(frame.height))
        .w_full()
        .px(px(SONG_CONTENT_PADDING))
        .pt(px(SONG_CONTENT_TOP_PADDING))
        .flex()
        .justify_center()
        .child(
            div()
                .debug_selector(|| "album-song-content".into())
                .w(px(frame.content_width))
                .h_full()
                .flex()
                .child(artwork(vinyl))
                .child(song_info),
        )
        .into_any_element()
}

fn artwork(disc: Option<Div>) -> impl IntoElement {
    div()
        .id("album-artwork-region")
        .debug_selector(|| "album-artwork-region".into())
        .w(relative(0.5))
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .justify_start()
        .children(disc)
}
