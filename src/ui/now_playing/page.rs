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
use comments::CommentsView;
use lyrics::LyricsView;

const SUMMARY_HEIGHT: f32 = 72.;
const SONG_CONTENT_MAX_WIDTH: f32 = 1400.;
const SONG_CONTENT_PADDING: f32 = 100.;
const SONG_CONTENT_TOP_PADDING: f32 = 24.;
const VINYL_MAX_SIDE: f32 = 540.;
const CONTENT_MAX_ASPECT: f32 = 1.6;
const SUMMARY_WIDTH_RATIO: f32 = 0.8;
/// 翻页补间的时长；两屏之间的自动滑动都走这一条。
const SCROLL_DURATION: Duration = Duration::from_millis(600);
/// 自动翻页的判定点：从本屏的静止位置往外滑过一屏的三分之一就翻页。
/// 两条线各属一侧——歌词页只有往下越线才翻，评论区只有往上越线才翻，
/// 合起来正好是歌词 / 唱片页的那两个三分之一点。
const PAGE_SNAP_TRIGGER: f32 = 1. / 3.;
/// 多久没有新的位移就算「滑完了」。滚轮是一串离散事件、没有抬起事件，
/// 只能靠「位移停了」来判断，用来把没滑够的偏移回弹到本屏静止点。
const SPRING_BACK_DELAY: Duration = Duration::from_millis(300);

/// 全页只有两屏。滚动位置不是自由量，而是「停在哪一屏」的投影。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    /// 歌词 / 唱片页。外层偏移为 0。
    Song,
    /// 评论区。外层偏移等于 `-页高`，把评论页顶对齐到视口顶。
    Comments,
}

impl Page {
    /// 这一屏静止时外层滚动容器的偏移。页高由窗口高度算出，所以窗口一变，
    /// 静止点跟着变——把偏移直接设成它即可，不需要任何补偿算术。
    fn rest_offset(self, page_height: f32) -> f32 {
        match self {
            Self::Song => 0.,
            Self::Comments => -page_height,
        }
    }
}

/// 最后一次滚动位移的方向，用于两个三分之一点之间的判定。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScrollDirection {
    /// 偏移变大：内容往下走。
    Up,
    /// 偏移变小：内容往上走。
    Down,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SongTab {
    #[default]
    Lyrics,
    Encyclopedia,
    Similar,
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
    cover_color: CoverColor,
    backdrop: Backdrop,
    song_id: Option<u64>,
    tonearm: Tonearm,
    tab: SongTab,
    /// 当前停在哪一屏。页面显示（头部文案、评论是否可滚）都是它的投影，
    /// 而不是从滚动偏移反推——两屏之间不存在"半屏"这种状态。
    page: Page,
    /// 外层滚动的补间：翻页和"回到唱片"都走它。
    ///
    /// 注意这一页**不接平台滚动**：`scroll` 只当一个位置变量用，滚轮由
    /// [`Self::wheel`] 自己推。位置完全归我们所有，动画才不会被打断。
    song_scroll: ScrollTween,
    /// 这次手势已经翻过页了。触控板抬手后还会继续发几十个惯性事件，
    /// 它们一律吞掉——否则会把动画拽回原位，还会顺着指针落到评论区列表上。
    snap_pending: bool,
    /// 「滑完了」看门狗。每个滚轮事件都换一个新的，
    /// 最后一个跑完时若还停在本屏静止点之外，就回弹。
    spring_back_task: Option<Task<()>>,
    comments_collapsed: bool,
    page_height: Option<f32>,
}

struct FrameData {
    page_height: f32,
    height: f32,
    content_width: f32,
    side: f32,
    summary_top: f32,
    show_comments: bool,
    reveal: f32,
    colors: ColorTokens,
    gradient: CoverGradient,
    title: String,
    artists: String,
    album: String,
    source: Option<String>,
    duration: String,
}

/// 补间完成后保留目标但不再写 offset，手动滚动不会被推回去。
#[derive(Default)]
struct ScrollTween {
    target: Option<Pixels>,
    playing: Option<(f32, Instant)>,
}

impl ScrollTween {
    fn aim(&mut self, handle: &ScrollHandle, target: Option<Pixels>, now: Instant) {
        if self.target == target {
            return;
        }
        self.target = target;
        self.playing = target.and_then(|to| {
            let from = f32::from(handle.offset().y);
            ((f32::from(to) - from).abs() > 0.5).then_some((from, now))
        });
    }

    /// 重新瞄准目标：目标没变也重放。
    ///
    /// 补间跑完后 `target` 会留着，手动滚过一点要归位时单靠 `aim` 会因
    /// 「目标相同」直接返回、补间永远不启动，所以先把目标清空再瞄。
    fn reaim(&mut self, handle: &ScrollHandle, target: Pixels, now: Instant) {
        self.target = None;
        self.aim(handle, Some(target), now);
    }

    fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    fn step(&mut self, handle: &ScrollHandle, window: &mut Window, now: Instant) -> bool {
        let playing = self.advance(handle, now);
        if playing {
            window.request_animation_frame();
        }
        playing
    }

    fn advance(&mut self, handle: &ScrollHandle, now: Instant) -> bool {
        let (Some((from, started)), Some(to)) = (self.playing, self.target) else {
            self.playing = None;
            return false;
        };
        let progress = (now.saturating_duration_since(started).as_secs_f32()
            / SCROLL_DURATION.as_secs_f32())
        .min(1.);
        let y = from + (f32::from(to) - from) * ease_out_quint()(progress);
        handle.set_offset(point(px(0.), px(y)));
        if progress < 1. {
            return true;
        }
        self.playing = None;
        false
    }
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
        // 评论区不再往外层交位移：它自己吃不下的时候让事件冒泡上来，
        // 由这一页决定是翻页还是吞掉。
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
            cover_color: CoverColor::default(),
            backdrop,
            song_id: None,
            tonearm: Tonearm::default(),
            tab: SongTab::default(),
            page: Page::Song,
            song_scroll: ScrollTween::default(),
            snap_pending: false,
            spring_back_task: None,
            comments_collapsed: false,
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
        self.comments_collapsed = false;
        self.reset_page();
    }

    /// 回到歌词页的静止状态。换歌、重新打开这一页都从这里起步，
    /// 顺带取消还没跑完的翻页补间和看门狗。
    fn reset_page(&mut self) {
        self.page = Page::Song;
        self.song_scroll = ScrollTween::default();
        self.snap_pending = false;
        self.spring_back_task = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
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
        self.lyrics
            .update(cx, |view, cx| view.set_active(false, cx));
        self.comments
            .update(cx, |view, cx| view.configure(false, false, cx));
        cx.notify();
    }

    /// 落到某一屏：记下状态，并把外层滚动补间到这一屏的静止点。
    ///
    /// 已经在静止点时补间长度为 0，什么都不会动；半路停下（偏移在两个静止点
    /// 之间）时这里就是「归位」的那一步。
    fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        let Some(page_height) = self.page_height else {
            return;
        };
        // 翻页会改变头部文案、评论区是否可滚、歌词是否跟随播放。
        // 另外补间要靠下一帧的 `step` 推进，所以即使页别没变（半路停下归位）
        // 也得重绘一次，否则动画起不来。
        self.page = page;
        cx.notify();
        let now = cx.background_executor().now();
        self.song_scroll
            .reaim(&self.scroll, px(page.rest_offset(page_height)), now);
    }

    /// 滚轮：整页不接平台滚动，位置完全由这里推。
    ///
    /// 滚多少、什么时候翻页、什么时候一律吞掉，全在这里决定。正因为位置
    /// 归我们所有，翻页动画才不会被原生滚动的惯性插一脚，触控板抬手后的
    /// 惯性也不会顺着指针落到评论区列表上。
    fn wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        // 事件由我们自己消化，不再往外传。
        cx.stop_propagation();
        let Some(page_height) = self.page_height else {
            return;
        };
        // 这次手势已经翻过页：剩下的惯性全吞掉，不给任何列表碰到。
        if self.snap_pending {
            return;
        }
        let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
        // 方向沿用平台约定：往下滚 delta 为负，`offset` 也是往下为负。
        let direction = if delta < 0. {
            ScrollDirection::Down
        } else {
            ScrollDirection::Up
        };
        // 新的滚动输入打断进行中的补间（比如回弹），位置从当前接着推。
        self.song_scroll = ScrollTween::default();
        let from = f32::from(self.scroll.offset().y);
        let to = (from + delta).clamp(-page_height, 0.);
        if to != from {
            self.scroll.set_offset(point(px(0.), px(to)));
            // 位置变了就得重绘：这一页没有平台滚动替我们通知视图了。
            cx.notify();
        }
        if self.crosses_line(direction) {
            // 碰线：立刻开始翻页，这次手势剩下的惯性交给动画。
            let target = match self.page {
                Page::Song => Page::Comments,
                Page::Comments => Page::Song,
            };
            self.snap_pending = true;
            self.set_page(target, cx);
            return;
        }
        self.restart_spring_back(cx);
    }

    /// 越线判定：歌词页只有往下越 1/3 才翻，评论区只有往上越 2/3 才翻。
    ///
    /// 带上方向，往本屏静止点那一侧回去（也就是没滑够就停住）不算碰线。
    fn crosses_line(&self, direction: ScrollDirection) -> bool {
        let Some(page_height) = self.page_height else {
            return false;
        };
        let scrolled = -f32::from(self.scroll.offset().y);
        let trigger = page_height * PAGE_SNAP_TRIGGER;
        match (self.page, direction) {
            (Page::Song, ScrollDirection::Down) => scrolled >= trigger,
            (Page::Comments, ScrollDirection::Up) => scrolled <= page_height - trigger,
            _ => false,
        }
    }

    /// 重置「滑完了」看门狗：每个滚轮事件都换一个新的计时器，
    /// 只有最后一个能跑到底，它跑完时就是这次手势停下的时刻。
    fn restart_spring_back(&mut self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        let timer = executor.timer(SPRING_BACK_DELAY);
        // 替换 `spring_back_task` 会丢掉上一个 Task，也就是取消上一次计时。
        self.spring_back_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                if this.snap_pending {
                    // 翻页动画还没跑完，惯性可能还在发：接着等。
                    if this.song_scroll.is_playing() {
                        this.restart_spring_back(cx);
                        return;
                    }
                    // 手势结束，翻页手势到此为止。
                    this.snap_pending = false;
                }
                this.spring_back(cx);
            });
        }));
    }

    /// 滑完还停在本屏静止点之外（也就是没滑够三分之一）就回弹。
    ///
    /// 越线的翻页在 [`Self::wheel`] 里已经即时处理过了，这里只管归位；
    /// 翻页动画正在跑时不能插手，否则会把滑到一半的页拉回来。
    fn spring_back(&mut self, cx: &mut Context<Self>) {
        if self.song_scroll.is_playing() {
            return;
        }
        self.set_page(self.page, cx);
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
        // 「在不在评论区」看的是停在哪一屏，而不是「外层滚到底了没有」：
        // 偏移是两个静止点之间的连续量，动画走到一半时它说明不了当前属于哪一屏。
        let show_comments = self.page == Page::Comments;
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
                this.comments_collapsed = true;
                this.comments
                    .update(cx, |view, cx| view.set_collapsed(true, cx));
                // 「回到唱片」就是落到歌词页，剩下的路交给翻页补间。
                this.set_page(Page::Song, cx);
                cx.notify();
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
        let now = cx.background_executor().now();
        let scrolling = self.song_scroll.step(&self.scroll, window, now);
        if self.comments_collapsed && !scrolling {
            self.comments_collapsed = false;
            self.comments
                .update(cx, |view, cx| view.set_collapsed(false, cx));
        }
        let page_height = (f32::from(window.viewport_size().height) - PLAYER_BAR_HEIGHT).max(1.);
        let previous_height = self.page_height.replace(page_height);
        if let Some(previous_height) = previous_height
            && previous_height != page_height
        {
            // 两屏的页高都等于窗口可视高度，窗口一变，两个静止点一起平移。
            // 停在一屏的静止点上就直接挪到新的静止点——这正是原来那串补偿算术
            // 想做的事，只是现在静止点是当前屏算出来的，不必判断"在歌词区还是
            // 评论区"，两屏自然都对。
            if (f32::from(self.scroll.offset().y) - self.page.rest_offset(previous_height)).abs()
                <= 1.
            {
                self.song_scroll = ScrollTween::default();
                self.scroll
                    .set_offset(point(px(0.), px(self.page.rest_offset(page_height))));
            } else if self.song_scroll.is_playing() {
                // 翻页走到一半：补间目标还是按旧页高算的，按新页高重新瞄准，
                // 剩下的路接着走完。
                let now = cx.background_executor().now();
                self.song_scroll
                    .reaim(&self.scroll, px(self.page.rest_offset(page_height)), now);
            }
            // 手势进行中：什么都不做，等看门狗按新页高判定去处。
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
            view.set_active(
                expanded && self.tab == SongTab::Lyrics && !frame.show_comments,
                cx,
            )
        });
        self.comments.update(cx, |view, cx| {
            // 翻页手势没结束时评论区不可滚：这段时间的滚轮是我们自己的，
            // 让列表去响应的话，抬手后的惯性就会把评论内容滚下去一大截。
            view.configure(expanded, frame.show_comments && !self.snap_pending, cx)
        });
        self.vinyl.read(cx).clear();
        self.summary_vinyl.read(cx).clear();
        if !expanded && frame.reveal <= 0. {
            self.song_scroll.aim(&self.scroll, None, now);
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
                                    .id("album-lyrics-scroll")
                                    .debug_selector(|| "album-lyrics-scroll".into())
                                    .size_full()
                                    .pt(px(HEADER_HEIGHT))
                                    // 整页不接平台滚动：位置由 `Self::wheel` 自己推。
                                    // 位置归我们所有，翻页动画才不会被原生滚动的惯性
                                    // 插一脚，抬手后的惯性也不会顺着指针漏给评论区列表。
                                    .overflow_hidden()
                                    .on_scroll_wheel(cx.listener(
                                        |this, event: &ScrollWheelEvent, window, cx| {
                                            this.wheel(event, window, cx);
                                        },
                                    ))
                                    // "滚动"只是这两层各挪一份相同的位移：
                                    // 平台滚动是往下的负偏移，这里直接当相对位移用。
                                    .child(
                                        div()
                                            .w_full()
                                            .relative()
                                            .top(self.scroll.offset().y)
                                            .child(song_page),
                                    )
                                    .child(
                                        div()
                                            .id("album-comments-page")
                                            .debug_selector(|| "album-comments-page".into())
                                            .w_full()
                                            .relative()
                                            .top(self.scroll.offset().y)
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
