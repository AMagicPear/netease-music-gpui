//! 黑胶（专辑歌词）全屏页。
//!
//! 文件按“常量 → 状态 → 数据加载 → 派生值 → 渲染分区 → 页面拼装 → 独立视图 → 测试”排列。
//! 每个渲染分区前都有 banner 注释；调整某块的尺寸、配色或文案时，
//! 先看 banner 定位，再在 [`AlbumLyrics::layout`] 里找对应的派生字段。
//!
//! 可复用的视觉件在 `crate::ui::components`：唱片+唱臂 [`vinyl_stage`]、
//! 评论行 [`comment_row`]，这里只保留本页特有的拼装。

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{
    Button, ColorTokens, Scrollbar, ScrollbarMode, Theme, Transition, transition,
};
use gpui_kit::component::TitleBar;

use crate::api::MusicApi;
use crate::models::{LyricLine, SongComment};
use crate::playback::PlaybackController;
use crate::ui::components::{
    ALBUM_REVEAL_DURATION, PLAYER_BAR_HEIGHT, PlayerBar, Tonearm, Vinyl, comment_row,
    format_duration, spinner, window_drag_area,
};
use crate::ui::cover_color::{
    Backdrop, CoverColor, CoverGradient, dark_colors, dark_gradient, gradient_layer,
};
use crate::ui::shell::{PAGE_HEADER_HEIGHT, WINDOW_HEADER_HEIGHT, hover_icon};

// ─────────────────────────────────────────────────────────────────────────
// 布局常量
// ─────────────────────────────────────────────────────────────────────────

/// 评论页顶部摘要胶囊所在条带的高度。
const SUMMARY_HEIGHT: f32 = 72.;
/// 内容区（左唱片 + 右信息）的最大宽度。
const SONG_CONTENT_MAX_WIDTH: f32 = 1400.;
/// 内容区左右内边距。
const SONG_CONTENT_PADDING: f32 = 100.;
/// 内容区上内边距。底部不留白——歌词要一直伸到进度条上。
const SONG_CONTENT_TOP_PADDING: f32 = 24.;
/// 唱片最大直径；再大就超出设计稿的视觉重心。
const VINYL_MAX_SIDE: f32 = 540.;
/// 内容区最大宽高比：1920×1080 下内容框约 1400×874。窗口又宽又矮时按高度收窄内容区。
const CONTENT_MAX_ASPECT: f32 = 1.6;
/// 摘要胶囊最大宽度占内容区的比例；随窗口宽度变化，长标题下不会横跨整页。
const SUMMARY_WIDTH_RATIO: f32 = 0.8;
/// 当前歌词行滚动到居中的时长，也是摘要胶囊回顶部的时长。
const SCROLL_DURATION: Duration = Duration::from_millis(450);
/// 歌词底部渐隐带的高度。
const LYRIC_FADE_HEIGHT: f32 = 96.;
/// 当前歌词行在歌词区里的竖直锚点：`0.` 是顶端，`0.5` 是正中。
const LYRIC_ANCHOR_RATIO: f32 = 0.33;
/// 歌词行距（每行底边留白）。
const LYRIC_LINE_GAP: f32 = 20.;
/// 歌词字号：正在播放的行大一号，其余收小。
const LYRIC_ACTIVE_TEXT: f32 = 22.;
const LYRIC_IDLE_TEXT: f32 = 18.;
/// 翻译字号，比正文小一档；行高单独收窄以贴近上面那行。
const LYRIC_TRANSLATION_TEXT: f32 = 16.;
const LYRIC_TRANSLATION_LINE_HEIGHT: f32 = 20.;

// ─────────────────────────────────────────────────────────────────────────
// 状态
// ─────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SongTab {
    #[default]
    Lyrics,
    Encyclopedia,
    Similar,
}

pub(in crate::ui) struct AlbumLyrics {
    player_bar: Entity<PlayerBar>,
    _player_subscription: Subscription,
    _playback_subscription: Subscription,
    vinyl: Entity<Vinyl>,
    summary_vinyl: Entity<Vinyl>,
    opened: bool,
    scroll: ScrollHandle,
    comment_scroll: ScrollHandle,
    playback: Entity<PlaybackController>,
    cover_color: CoverColor,
    backdrop: Backdrop,
    song_id: Option<u64>,
    generation: u64,
    lyrics: Vec<LyricLine>,
    lyric_scroll: ScrollHandle,
    active_lyric: Option<usize>,
    lyric_loading: bool,
    lyric_error: Option<String>,
    comments: Vec<SongComment>,
    comment_total: u64,
    comment_offset: usize,
    comments_more: bool,
    comments_loading: bool,
    comments_error: Option<String>,
    lyric_request: Option<tokio::task::AbortHandle>,
    comments_request: Option<tokio::task::AbortHandle>,
    tonearm: Tonearm,
    tab: SongTab,
    lyric_scroll_anim: ScrollTween,
    /// 外层滚动（唱片页 + 评论页）的补间，摘要胶囊回顶部用。
    song_scroll: ScrollTween,
    /// 回顶部途中临时收起评论列表，只留标题和计数。
    comments_collapsed: bool,
}

/// 一帧的派生值：几何、配色、文案。集中算一次，各渲染分区只读它。
/// 想微调某块排版时，先在 [`AlbumLyrics::layout`] 里找到对应字段。
struct Layout {
    // 几何（单位 px）
    page_height: f32,
    height: f32,
    content_width: f32,
    side: f32,
    summary_top: f32,
    show_comments: bool,
    reveal: f32,
    // 配色
    colors: ColorTokens,
    gradient: CoverGradient,
    // 文案
    title: String,
    artists: String,
    album: String,
    source: Option<String>,
    duration: String,
}

/// 按时间插值的滚动补间；歌词居中和「摘要胶囊回顶部」共用。
///
/// `aim` 设定目标，目标一变就开一段补间从当前偏移平滑滚过去；`step` 每帧推进。
/// 补间跑完就停手——否则每帧写 `offset` 会把用户的手动滚动推回去。
#[derive(Default)]
struct ScrollTween {
    /// 当前目标偏移；`None` 表示没有目标，也就不动。
    target: Option<Pixels>,
    /// 正在播放的补间：起点偏移与开始时刻。
    playing: Option<(f32, Instant)>,
}

impl ScrollTween {
    /// 设定新目标；与上次相同则什么都不做。传 `None` 表示取消。
    fn aim(&mut self, handle: &ScrollHandle, target: Option<Pixels>) {
        if self.target == target {
            return;
        }
        self.target = target;
        self.playing = target.and_then(|to| {
            let from = f32::from(handle.offset().y);
            ((f32::from(to) - from).abs() > 0.5).then(|| (from, Instant::now()))
        });
    }

    /// 推进补间并把偏移写回；返回是否还在动画中。
    fn step(&mut self, handle: &ScrollHandle, window: &mut Window) -> bool {
        let playing = self.advance(handle, Instant::now());
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
        let progress =
            (now.duration_since(started).as_secs_f32() / SCROLL_DURATION.as_secs_f32()).min(1.);
        let y = from + (f32::from(to) - from) * ease_out_quint()(progress);
        handle.set_offset(point(px(0.), px(y)));
        if progress < 1. {
            return true;
        }
        // 保留目标：同一句歌词期间的手动滚动不会被重新拉回。
        self.playing = None;
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 数据加载与打开/收起
// ─────────────────────────────────────────────────────────────────────────

impl AlbumLyrics {
    pub(in crate::ui) fn new(player_bar: Entity<PlayerBar>, cx: &mut Context<Self>) -> Self {
        let mut source = player_bar.read(cx).source_name(cx);
        let player_subscription = cx.observe(&player_bar, move |this, player, cx| {
            let next = player.read(cx).source_name(cx);
            if next != source {
                source = next;
                if this.opened {
                    cx.notify();
                }
            }
        });
        let playback = player_bar.read(cx).playback();
        let backdrop = player_bar.read(cx).album_backdrop();
        let clock = player_bar.read(cx).rotation_clock();
        let vinyl = cx.new(|cx| Vinyl::new(playback.clone(), clock.clone(), "images/disc.png", cx));
        let summary_vinyl =
            cx.new(|cx| Vinyl::new(playback.clone(), clock, "images/miniVinyl.png", cx));
        let mut playback_state = {
            let state = playback.read(cx).snapshot();
            (state.revision, state.is_playing)
        };
        let playback_subscription = cx.observe(&playback, move |this, playback, cx| {
            let state = playback.read(cx).snapshot();
            let next = (state.revision, state.is_playing);
            let active = this
                .lyrics
                .partition_point(|line| line.time <= state.position)
                .checked_sub(1);
            if this.opened && (next != playback_state || active != this.active_lyric) {
                cx.notify();
            }
            playback_state = next;
        });
        Self {
            player_bar,
            _player_subscription: player_subscription,
            _playback_subscription: playback_subscription,
            vinyl,
            summary_vinyl,
            opened: false,
            scroll: ScrollHandle::default(),
            comment_scroll: ScrollHandle::default(),
            playback,
            cover_color: CoverColor::default(),
            backdrop,
            song_id: None,
            generation: 0,
            lyrics: Vec::new(),
            lyric_scroll: ScrollHandle::default(),
            active_lyric: None,
            lyric_loading: false,
            lyric_error: None,
            comments: Vec::new(),
            comment_total: 0,
            comment_offset: 0,
            comments_more: false,
            comments_loading: false,
            comments_error: None,
            lyric_request: None,
            comments_request: None,
            tonearm: Tonearm::default(),
            tab: SongTab::default(),
            lyric_scroll_anim: ScrollTween::default(),
            song_scroll: ScrollTween::default(),
            comments_collapsed: false,
        }
    }

    /// 当前歌曲变化时清空并重新拉取歌词与评论；切歌会作废旧请求。
    fn sync_song(&mut self, cx: &mut Context<Self>) {
        let song_id = self
            .playback
            .read(cx)
            .snapshot()
            .current_song
            .as_ref()
            .map(|song| song.id);
        if self.song_id == song_id {
            return;
        }
        for request in [&mut self.lyric_request, &mut self.comments_request] {
            if let Some(request) = request.take() {
                request.abort();
            }
        }
        self.song_id = song_id;
        self.generation = self.generation.wrapping_add(1);
        self.lyrics.clear();
        self.comments.clear();
        self.comment_total = 0;
        self.comment_offset = 0;
        self.comments_more = false;
        self.comments_loading = false;
        self.lyric_loading = false;
        self.lyric_error = None;
        self.comments_error = None;
        self.active_lyric = None;
        self.lyric_scroll_anim = ScrollTween::default();
        self.song_scroll = ScrollTween::default();
        self.comments_collapsed = false;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.comment_scroll.set_offset(point(px(0.), px(0.)));
        self.lyric_scroll.set_offset(point(px(0.), px(0.)));
        if song_id.is_some() {
            self.load_lyrics(cx);
            self.load_comments(cx);
        }
    }

    fn load_lyrics(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.song_id else {
            return;
        };
        self.lyric_loading = true;
        let generation = self.generation;
        self.lyric_error = None;
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::song_lyrics(api.client.clone(), id));
        self.lyric_request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request.await.unwrap_or_else(|_| Err("歌词加载失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.song_id != Some(id) || this.generation != generation {
                    return;
                }
                this.lyric_loading = false;
                this.lyric_request = None;
                match result {
                    Ok(lyrics) => this.lyrics = lyrics,
                    Err(error) => this.lyric_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_comments(&mut self, cx: &mut Context<Self>) {
        if self.comments_loading {
            return;
        }
        let Some(id) = self.song_id else {
            return;
        };
        let offset = self.comment_offset;
        let generation = self.generation;
        self.comments_loading = true;
        self.comments_error = None;
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::song_comments(api.client.clone(), id, offset));
        self.comments_request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request.await.unwrap_or_else(|_| Err("评论加载失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.song_id != Some(id) || this.generation != generation {
                    return;
                }
                this.comments_loading = false;
                this.comments_request = None;
                match result {
                    Ok(page) => {
                        this.comment_total = page.total;
                        this.comments_more = page.more;
                        this.comment_offset = offset + 20;
                        let before = this.comments.len();
                        for comment in page.comments {
                            if !this
                                .comments
                                .iter()
                                .any(|existing| existing.id == comment.id)
                            {
                                this.comments.push(comment);
                            }
                        }
                        // 一页没带来任何新评论就别再拉了（否则自动加载会原地循环）。
                        if this.comments.len() == before {
                            this.comments_more = false;
                        }
                    }
                    Err(error) => this.comments_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(in crate::ui) fn is_open(&self) -> bool {
        self.opened
    }

    pub(in crate::ui) fn vinyl_overlays(&self) -> [Entity<Vinyl>; 2] {
        [self.vinyl.clone(), self.summary_vinyl.clone()]
    }

    pub(in crate::ui) fn open(&mut self, cx: &mut Context<Self>) {
        if !self.opened {
            self.song_scroll = ScrollTween::default();
            self.scroll.set_offset(point(px(0.), px(0.)));
            self.opened = true;
            cx.notify();
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 派生值
// ─────────────────────────────────────────────────────────────────────────

impl AlbumLyrics {
    /// 计算一帧的几何、配色与文案。各渲染分区只读它，
    /// 微调尺寸/取色时先从这里找对应字段。
    fn layout(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Layout {
        let cover_url = self.player_bar.read(cx).album_cover_url(cx);
        let color = if self.opened {
            cover_url
                .as_deref()
                .and_then(|url| self.cover_color.load(url, window, cx))
        } else {
            self.cover_color.current()
        };
        let gradient = dark_gradient(color);
        let colors = dark_colors(Theme::global(cx).tokens.colors, gradient.0[1].into());
        let song = self.playback.read(cx).snapshot().current_song.clone();

        // 外层两屏等高；每屏的顶栏与正文共同占满播放器上方的空间。
        let page_height = (f32::from(window.viewport_size().height) - PLAYER_BAR_HEIGHT).max(1.);
        let height = (page_height - WINDOW_HEADER_HEIGHT - PAGE_HEADER_HEIGHT).max(1.);
        let reveal = transition(
            "album-lyrics-reveal",
            if self.opened { 1_f32 } else { 0. },
            Transition::new(ALBUM_REVEAL_DURATION).ease(ease_out_quint()),
            window,
            cx,
        );
        let summary_top = page_height + WINDOW_HEADER_HEIGHT + f32::from(self.scroll.offset().y);
        let show_comments = summary_top <= WINDOW_HEADER_HEIGHT;

        // 内容区宽度同时受可用宽、设计上限和高度约束：又宽又矮的条带窗口按高度收窄，
        // 避免两栏被拉得过开。
        let content_height = (height - SONG_CONTENT_TOP_PADDING).max(1.);
        let content_width = (f32::from(window.viewport_size().width) - SONG_CONTENT_PADDING * 2.)
            .min(SONG_CONTENT_MAX_WIDTH)
            .min(content_height * CONTENT_MAX_ASPECT)
            .max(1.);
        // 唱片是正方形，取栏宽的 80%，再用设计上限封顶。
        // 内容宽已 ≤ 1.6×内容高，所以它天然不会超过内容高的 64%。
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
        let source = self.player_bar.read(cx).source_name(cx);
        let duration = song
            .as_ref()
            .map(|song| format_duration(song.duration()))
            .unwrap_or_default();

        Layout {
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
    /// 把当前歌词行滚到视口中间。
    fn sync_lyric_scroll(&mut self, active: Option<usize>, window: &mut Window) {
        let desired = active.and_then(|index| {
            let item = self.lyric_scroll.bounds_for_item(index)?;
            Some(anchored_offset(
                self.lyric_scroll.bounds(),
                item,
                self.lyric_scroll.max_offset().y,
            ))
        });
        self.lyric_scroll_anim.aim(&self.lyric_scroll, desired);
        self.lyric_scroll_anim.step(&self.lyric_scroll, window);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 渲染分区
// ─────────────────────────────────────────────────────────────────────────

impl AlbumLyrics {
    /// 评论页顶部的摘要胶囊：点击回到唱片，显示小唱片、标题、歌手。
    fn render_summary(
        &self,
        layout: &Layout,
        summary_vinyl: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new("album-song-summary")
            .aria_label("回到唱片")
            .debug_selector(|| "album-song-summary".into())
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                // 回顶部：先把评论列表收起来（只留「全部评论」和计数），再平滑滑上去，
                // 免得镜头一路扫过整个评论区。收起标记在补间结束时清掉。
                this.comments_collapsed = true;
                this.song_scroll.aim(&this.scroll, None);
                this.song_scroll.aim(&this.scroll, Some(px(0.)));
                cx.notify();
            }))
            .max_w(px(layout.content_width * SUMMARY_WIDTH_RATIO))
            .h(px(58.))
            // 外圆半径 29，唱片半径 22.5；扣除边框后的左内边距为 5.5。
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
                            .child(layout.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(layout.colors.muted_foreground)
                            .child("—"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "album-summary-artists".into())
                            .max_w(px(240.))
                            .text_size(px(13.))
                            .text_color(layout.colors.muted_foreground)
                            .truncate()
                            .child(layout.artists.clone()),
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

    /// 歌词分区：空态/错误重试，或可滚动的逐行歌词（当前行高亮、点击跳转）。
    fn render_lyrics(&self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        if self.lyrics.is_empty() {
            return div()
                .pt(px(80.))
                .text_color(layout.colors.muted_foreground)
                .child(if self.lyric_loading {
                    "歌词加载中"
                } else if self.lyric_error.is_some() {
                    "歌词加载失败"
                } else {
                    "暂无歌词"
                })
                .when(self.lyric_error.is_some(), |content| {
                    content.child(
                        Button::new("retry-song-lyrics")
                            .ml_2()
                            .child("重试")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load_lyrics(cx);
                                cx.notify();
                            })),
                    )
                })
                .into_any_element();
        }
        // 外面包一层 relative：滚动区铺满，底部渐隐固定在下方。
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("song-lyric-lines")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.lyric_scroll)
                    .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                        this.lyric_scroll_anim.playing = None;
                        cx.stop_propagation();
                    }))
                    .pt(px(72.))
                    .pb(px(160.))
                    .children(self.lyrics.iter().enumerate().map(|(index, line)| {
                        let time = line.time;
                        let active = Some(index) == self.active_lyric;
                        div()
                            .id(("song-lyric-line", index))
                            .w_full()
                            .pb(px(LYRIC_LINE_GAP))
                            .text_size(px(if active {
                                LYRIC_ACTIVE_TEXT
                            } else {
                                LYRIC_IDLE_TEXT
                            }))
                            .line_height(px(28.))
                            .text_color(if active { white() } else { white().alpha(0.4) })
                            .font_weight(if active {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.playback
                                    .update(cx, |playback, cx| playback.seek_to(time, cx));
                            }))
                            .child(line.text.clone())
                            .when_some(line.translation.clone(), |line, translation| {
                                line.child(
                                    div()
                                        .line_height(px(LYRIC_TRANSLATION_LINE_HEIGHT))
                                        .text_size(px(LYRIC_TRANSLATION_TEXT))
                                        .text_color(white().alpha(0.45))
                                        .child(translation),
                                )
                            })
                    })),
            )
            // 底部渐隐：歌词滚到底时淡入背景，不做硬切。
            .child(lyrics_fade(self.backdrop.clone()))
            .into_any_element()
    }

    /// 标签页内容：歌词 / 百科 / 相似推荐。
    fn render_tab(&self, layout: &Layout, lyric_content: AnyElement) -> AnyElement {
        match self.tab {
            SongTab::Lyrics => lyric_content,
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
                .text_color(layout.colors.muted_foreground)
                .children(
                    [
                        ("歌曲", layout.title.clone()),
                        ("歌手", layout.artists.clone()),
                        ("专辑", layout.album.clone()),
                        ("时长", layout.duration.clone()),
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
                .text_color(layout.colors.muted_foreground)
                .child("相似推荐暂不可用")
                .into_any_element(),
        }
    }

    /// 歌词 / 百科 / 相似推荐 切换胶囊。
    fn render_tab_bar(&self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        let colors = layout.colors;
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

    /// 右栏：歌曲标题、元信息、标签胶囊、标签内容。
    fn render_song_info(
        &self,
        layout: &Layout,
        tab_content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = layout.colors;
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
                    .text_color(white())
                    .line_clamp(2)
                    .child(layout.title.clone()),
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
                            .child(format!("专辑：{}", layout.album)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "album-metadata-artists".into())
                            .min_w_0()
                            .truncate()
                            .child(format!("歌手：{}", layout.artists)),
                    )
                    .when_some(layout.source.clone(), |metadata, source| {
                        metadata.child(
                            div()
                                .debug_selector(|| "album-metadata-source".into())
                                .min_w_0()
                                .truncate()
                                .child(format!("来源：{source}")),
                        )
                    }),
            )
            .child(self.render_tab_bar(layout, cx))
            .child(tab_content)
            .into_any_element()
    }

    /// 评论分区：标题 + 计数、评论列表、加载/空态/错误/加载更多。
    fn render_comments(&self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        let colors = layout.colors;
        let show_comments = layout.show_comments;

        // 评论列表：回顶部途中会临时整块收起，只留标题和计数。
        let mut list = div();
        if !self.comments_collapsed {
            list = list.children(self.comments.iter().map(comment_row)).when(
                self.comments_loading,
                |list| {
                    list.child(div().py_5().flex().justify_center().child(spinner(
                        "album-comments-loading",
                        20.,
                        colors.muted_foreground,
                    )))
                },
            );
            if self.comments_error.is_some() {
                list = list.child(
                    Button::new("retry-song-comments")
                        .child("评论加载失败，重试")
                        .on_click(cx.listener(|this, _, _, cx| this.load_comments(cx))),
                );
            } else if !self.comments_loading && self.comments.is_empty() {
                list = list.child(
                    div()
                        .py_8()
                        .text_color(colors.muted_foreground)
                        .child("还没有评论"),
                );
            }
        }

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .max_w(px(1120.))
            .mx_auto()
            .child(
                div()
                    .id("album-comments-region")
                    .debug_selector(|| "album-comments-region".into())
                    .size_full()
                    .overflow_hidden()
                    .when(show_comments, |comments| comments.overflow_y_scroll())
                    .track_scroll(&self.comment_scroll)
                    .on_scroll_wheel(cx.listener(move |this, _, _, cx| {
                        if show_comments {
                            // 原生滚动先更新内层；仅把越过评论顶部的剩余位移交回外层。
                            let remainder = this.comment_scroll.offset().y.max(px(0.));
                            if remainder > px(0.) {
                                this.comment_scroll.set_offset(point(px(0.), px(0.)));
                                this.scroll
                                    .set_offset(this.scroll.offset() + point(px(0.), remainder));
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }
                    }))
                    .px(px(80.))
                    .pt(px(24.))
                    .pb(px(100.))
                    .child(
                        div()
                            .debug_selector(|| "album-comments-heading".into())
                            .flex()
                            .items_center()
                            .gap_1()
                            .mb_4()
                            .text_color(white())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(18.))
                            .child("全部评论")
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .child(self.comment_total.to_string()),
                            ),
                    )
                    .child(list),
            )
            // 滚动条放在滚动容器外边，否则它的高度会被计入内容高度。
            // 黑胶页是暗底而主题是浅色，拇指单独调成白色半透明。
            .when(show_comments, |comments| {
                comments.child(
                    Scrollbar::vertical(&self.comment_scroll)
                        .mode(ScrollbarMode::Scrolling)
                        .styles(|styles| {
                            styles
                                .thumb(|thumb| thumb.bg(white().alpha(0.25)))
                                .thumb_hover(|thumb| thumb.bg(white().alpha(0.35)))
                        }),
                )
            })
            .into_any_element()
    }

    /// 顶部透明标题栏：收起按钮 + 关闭窗口。
    fn render_window_header(&self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        TitleBar::new()
            .on_close_window(|_, window, cx| crate::desktop::close_window(window, cx))
            .absolute()
            .top_0()
            .left_0()
            .h(px(WINDOW_HEADER_HEIGHT))
            .w_full()
            .border_b_0()
            .bg(transparent_black())
            .child(
                hover_icon(
                    "close-album-lyrics",
                    "收起专辑歌词页",
                    "icons/unfold.svg",
                    layout.colors,
                )
                .ml_0()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.opened = false;
                    cx.notify();
                })),
            )
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 页面拼装
// ─────────────────────────────────────────────────────────────────────────

impl Render for AlbumLyrics {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 先推进外层滚动补间，本帧的 summary_top 才能用上新偏移。
        let scrolling = self.song_scroll.step(&self.scroll, window);
        if self.comments_collapsed && !scrolling {
            self.comments_collapsed = false;
        }
        if self.opened {
            self.sync_song(cx);
        }
        let layout = self.layout(window, cx);
        self.vinyl.read(cx).clear();
        self.summary_vinyl.read(cx).clear();
        if !self.opened && layout.reveal <= 0. {
            self.lyric_scroll_anim.aim(&self.lyric_scroll, None);
            self.song_scroll.aim(&self.scroll, None);
            self.tonearm = Tonearm::default();
            return div().into_any_element();
        }

        // 滑到评论区底部附近就自动续拉下一页，不再放「加载更多」按钮。
        if can_load_more_comments(
            self.opened,
            layout.show_comments,
            self.comments_more,
            self.comments_loading,
            self.comments_error.is_some(),
        ) {
            let remaining = self.comment_scroll.max_offset().y + self.comment_scroll.offset().y;
            if remaining < px(120.) {
                self.load_comments(cx);
            }
        }

        // 跟随播放进度高亮当前歌词行。
        let position = self.playback.read(cx).snapshot().position;
        let active = self
            .lyrics
            .partition_point(|line| line.time <= position)
            .checked_sub(1);
        if active != self.active_lyric {
            self.active_lyric = active;
        }
        if self.tab == SongTab::Lyrics && !layout.show_comments {
            self.sync_lyric_scroll(active, window);
        } else {
            self.lyric_scroll_anim.aim(&self.lyric_scroll, None);
        }

        // 两张唱片要用当前封面纹理，唱臂要推进动画，都先算好再交给分区。
        let arm_angle = {
            let playback = self.playback.read(cx).snapshot();
            self.tonearm
                .angle(playback.is_playing, playback.revision, window, cx)
        };
        let vinyl = (layout.reveal > 0. && layout.summary_top > 0.).then(|| {
            self.vinyl
                .read(cx)
                .placeholder(layout.side, 1., Some(arm_angle))
                .debug_selector(|| "album-vinyl-image".into())
        });
        let summary_vinyl = if layout.summary_top < layout.page_height {
            self.summary_vinyl.read(cx).placeholder(45., 1., None)
        } else {
            div().size(px(45.)).flex_none()
        }
        .debug_selector(|| "album-summary-vinyl".into());

        // 先构建两块内容区，再自底向上拼装。
        let lyric_content = self.render_lyrics(&layout, cx);
        let tab_content = self.render_tab(&layout, lyric_content);
        let song_info = self.render_song_info(&layout, tab_content, cx);
        let comments = self.render_comments(&layout, cx);
        let summary = self.render_summary(&layout, summary_vinyl, cx);
        let song_page = build_song_page(&layout, song_info, vinyl);

        // 全屏页外壳：背景渐变 + 外层滚动（唱片页叠评论页）+ 两层顶栏。
        // 缓存视图作为独立布局根；滑动层放在其子节点，百分比 top 才有参照。
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(relative(1. - layout.reveal))
                    .size_full()
                    .flex()
                    .flex_col()
                    .bg(layout.colors.background)
                    .occlude()
                    .child(
                        gradient_layer(
                            "album-background-color",
                            layout.gradient,
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
                                    .size_full()
                                    .pt(px(WINDOW_HEADER_HEIGHT + PAGE_HEADER_HEIGHT))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .child(song_page)
                                    .child(
                                        div()
                                            .id("album-comments-page")
                                            .debug_selector(|| "album-comments-page".into())
                                            .w_full()
                                            .h(px(layout.page_height))
                                            .pt(px(WINDOW_HEADER_HEIGHT))
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
                            .when(layout.show_comments, |content| {
                                content.child(
                                    div()
                                        .absolute()
                                        .bottom(px(16.))
                                        .left(relative(0.5))
                                        .ml(px(-76.))
                                        .child(
                                            Button::new("publish-song-comment")
                                                .disabled(true)
                                                .w(px(152.))
                                                .h(px(40.))
                                                .rounded_full()
                                                .bg(hsla(0., 0., 0.35, 0.55))
                                                .border_1()
                                                .border_color(white().alpha(0.08))
                                                .text_color(white())
                                                .child("发布评论"),
                                        ),
                                )
                            }),
                    )
                    // 唱片能从透明顶栏下方透出；评论滚动区的边界位于胶囊下方，无需额外遮罩。
                    .child(self.render_window_header(&layout, cx))
                    .child(page_header(&layout)),
            )
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 独立视图
// ─────────────────────────────────────────────────────────────────────────

fn can_load_more_comments(
    opened: bool,
    visible: bool,
    more: bool,
    loading: bool,
    failed: bool,
) -> bool {
    opened && visible && more && !loading && !failed
}

/// 让某行中心对齐视口高度的 [`LYRIC_ANCHOR_RATIO`] 处所需的竖直偏移，
/// 再夹进可滚动范围 `[-max, 0]`。靠前的行没有负的滚动空间，居不了中就只能顶到开头。
fn anchored_offset(viewport: Bounds<Pixels>, item: Bounds<Pixels>, max: Pixels) -> Pixels {
    let anchor = f32::from(viewport.top()) + f32::from(viewport.size.height) * LYRIC_ANCHOR_RATIO;
    px((anchor - f32::from(item.center().y)).clamp(-f32::from(max), 0.))
}

/// 歌词底部的渐隐带：从透明渐变到当地底色，滚动时歌词不做硬切。
/// 底色由 [`Backdrop::paint_fade`] 按自身在屏幕上的位置现采，
/// 所以整页上下滚动时始终和身后的背景对齐。
/// 这一层没有监听器、不带 hitbox，鼠标事件和滚轮会直接穿透到下面的歌词。
fn lyrics_fade(backdrop: Backdrop) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| backdrop.paint_fade(bounds, window),
    )
    .absolute()
    .left_0()
    .bottom_0()
    .w_full()
    .h(px(LYRIC_FADE_HEIGHT))
}

/// 唱片区：居中内容区，左唱片右信息，整体受内容宽/唱片边长约束。
fn build_song_page(layout: &Layout, song_info: AnyElement, vinyl: Option<Div>) -> AnyElement {
    div()
        .h(px(layout.height))
        .w_full()
        .px(px(SONG_CONTENT_PADDING))
        .pt(px(SONG_CONTENT_TOP_PADDING))
        .flex()
        .justify_center()
        .child(
            div()
                .debug_selector(|| "album-song-content".into())
                .w(px(layout.content_width))
                .h_full()
                .flex()
                .child(artwork(vinyl))
                .child(song_info),
        )
        .into_any_element()
}

/// 标题栏下方的一行：右侧“播放器模式”提示与迷你模式按钮。
fn page_header(layout: &Layout) -> AnyElement {
    window_drag_area("album-lyrics-header")
        .absolute()
        .top(px(WINDOW_HEADER_HEIGHT))
        .left_0()
        .h(px(PAGE_HEADER_HEIGHT))
        .w_full()
        .bg(transparent_black())
        .px(px(40.))
        .flex()
        .items_center()
        .justify_end()
        .when(!layout.show_comments, |header| {
            header.child(
                div()
                    .text_color(layout.colors.muted_foreground)
                    .text_size(px(13.))
                    .child("播放器模式"),
            )
        })
        .child(hover_icon(
            "album-header-mini-button",
            "迷你模式",
            "icons/menu_mini.svg",
            layout.colors,
        ))
        .into_any_element()
}

/// 左半栏：唱片舞台（见 [`vinyl_stage`]）。
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

// ─────────────────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn failed_comment_page_waits_for_manual_retry() {
        use super::can_load_more_comments;
        assert!(can_load_more_comments(true, true, true, false, false));
        assert!(!can_load_more_comments(true, true, true, false, true));
        assert!(!can_load_more_comments(false, true, true, false, false));
        assert!(!can_load_more_comments(true, false, true, false, false));
        assert!(!can_load_more_comments(true, true, true, true, false));
        assert!(!can_load_more_comments(true, true, false, false, false));
    }

    #[test]
    fn completed_lyric_scroll_leaves_manual_offset_until_target_changes() {
        use super::{SCROLL_DURATION, ScrollTween};
        use gpui::{ScrollHandle, point, px};
        use std::time::Instant;
        let handle = ScrollHandle::default();
        let mut tween = ScrollTween::default();
        tween.aim(&handle, Some(px(-300.)));
        assert!(!tween.advance(&handle, tween.playing.unwrap().1 + SCROLL_DURATION));
        assert_eq!(handle.offset().y, px(-300.));
        handle.set_offset(point(px(0.), px(-380.)));
        tween.aim(&handle, Some(px(-300.)));
        assert!(!tween.advance(&handle, Instant::now()));
        assert_eq!(handle.offset().y, px(-380.));
        tween.aim(&handle, Some(px(-450.)));
        assert!(tween.advance(&handle, Instant::now()));
        tween.playing = None; // 手动滚轮中止正在运行的补间。
        handle.set_offset(point(px(0.), px(-500.)));
        tween.aim(&handle, Some(px(-450.)));
        assert!(!tween.advance(&handle, Instant::now()));
        assert_eq!(handle.offset().y, px(-500.));
        // 回顶部按钮显式重新发起，即使上一次目标也是零。
        tween.aim(&handle, None);
        tween.aim(&handle, Some(px(0.)));
        assert!(tween.advance(&handle, Instant::now()));
    }

    #[test]
    fn lyric_anchor_offset_hits_the_anchor_and_clamps_to_scroll_range() {
        use gpui::{Bounds, point, px, size};

        let viewport = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(400.)));
        let item =
            |top: f32, height: f32| Bounds::new(point(px(0.), px(top)), size(px(400.), px(height)));

        // 锚点在视口 40% 处（y=160）；行中心 1030 对上去 → 需要 -870。
        assert_eq!(
            super::anchored_offset(viewport, item(1010., 40.), px(900.)),
            px(-870.)
        );
        // 超出可滚动范围时夹到底，不会滑过头。
        assert_eq!(
            super::anchored_offset(viewport, item(1010., 40.), px(500.)),
            px(-500.)
        );
        // 靠前的行没有负滚动空间，只能顶到开头。
        assert_eq!(
            super::anchored_offset(viewport, item(72., 28.), px(900.)),
            px(0.)
        );
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn tonearm_finishes_each_motion_and_lifts_even_when_song_loading_is_fast(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::Tonearm;
        use gpui::prelude::*;
        use gpui::{Context, IntoElement, Render, Window, div, px, size};
        use std::time::Duration;
        struct Host {
            playing: bool,
            revision: u64,
            tonearm: Tonearm,
            angle: f32,
        }
        impl Render for Host {
            fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.angle = self.tonearm.angle(self.playing, self.revision, window, cx);
                div().size_full()
            }
        }
        let window = cx.open_window(size(px(100.), px(100.)), |_, _| Host {
            playing: true,
            revision: 1,
            tonearm: Tonearm::default(),
            angle: 0.,
        });
        let draw = |cx: &mut gpui::TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            window.update(cx, |host, _, _| host.angle).unwrap()
        };
        assert_eq!(draw(cx), 0.);
        window
            .update(cx, |host, _, _| host.playing = false)
            .unwrap();
        assert_eq!(draw(cx), 0.);
        cx.executor().advance_clock(Duration::from_millis(175));
        let halfway = draw(cx);
        assert!(halfway > -33. && halfway < 0.);
        window.update(cx, |host, _, _| host.playing = true).unwrap();
        assert!((draw(cx) - halfway).abs() < 0.001);
        cx.executor().advance_clock(Duration::from_millis(175));
        assert_eq!(draw(cx), -33., "恢复播放不能打断抬起动作");
        cx.executor().advance_clock(Duration::from_millis(350));
        assert_eq!(draw(cx), 0.);
        // 即使加载的停止状态未被渲染，revision 变化也必须完整抬起再落下。
        window.update(cx, |host, _, _| host.revision += 1).unwrap();
        assert_eq!(draw(cx), 0.);
        cx.executor().advance_clock(Duration::from_millis(350));
        assert_eq!(draw(cx), -33.);
        cx.executor().advance_clock(Duration::from_millis(175));
        let lowering = draw(cx);
        assert!(lowering > -33. && lowering < 0.);
        window
            .update(cx, |host, _, _| host.playing = false)
            .unwrap();
        assert!((draw(cx) - lowering).abs() < 0.001);
        cx.executor().advance_clock(Duration::from_millis(175));
        assert_eq!(draw(cx), 0., "暂停不能打断落下动作");
        cx.executor().advance_clock(Duration::from_millis(350));
        assert_eq!(draw(cx), -33.);
        window.update(cx, |host, _, _| host.revision += 1).unwrap();
        assert_eq!(draw(cx), -33., "未播放的新歌曲应保持抬起");
    }
}
