//! 黑胶（专辑歌词）全屏页。
//!
//! 文件按“常量 → 状态 → 数据加载 → 派生值 → 渲染分区 → 页面拼装 → 独立视图 → 测试”排列。
//! 每个渲染分区前都有 banner 注释；调整某块的尺寸、配色或文案时，
//! 先看 banner 定位，再在 [`AlbumLyrics::layout`] 里找对应的派生字段。
//!
//! 可复用的视觉件在 `crate::ui::components`：唱片+唱臂 [`vinyl_stage`]、
//! 评论行 [`comment_row`]，这里只保留本页特有的拼装。

use std::time::Duration;

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme, Transition, transition};
use gpui_kit::component::TitleBar;

use crate::api::MusicApi;
use crate::models::{LyricLine, SongComment};
use crate::playback::PlaybackController;
use crate::ui::components::{
    PLAYER_BAR_HEIGHT, PlayerBar, Tonearm, comment_row, format_duration, vinyl_stage,
    window_drag_area,
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
/// 内容区上下内边距。
const SONG_CONTENT_VERTICAL_PADDING: f32 = 24.;
/// 唱片最大直径；再大就超出设计稿的视觉重心。
const VINYL_MAX_SIDE: f32 = 540.;
/// 内容区最大宽高比：1920×1080 下内容框约 1400×874。窗口又宽又矮时按高度收窄内容区。
const CONTENT_MAX_ASPECT: f32 = 1.6;
/// 摘要胶囊最大宽度占内容区的比例；随窗口宽度变化，长标题下不会横跨整页。
const SUMMARY_WIDTH_RATIO: f32 = 0.8;
/// 全屏页展开/收起的时长。
const REVEAL_DURATION: Duration = Duration::from_millis(500);

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

// ─────────────────────────────────────────────────────────────────────────
// 数据加载与打开/收起
// ─────────────────────────────────────────────────────────────────────────

impl AlbumLyrics {
    pub(in crate::ui) fn new(player_bar: Entity<PlayerBar>, cx: &mut Context<Self>) -> Self {
        let player_subscription = cx.observe(&player_bar, |_, _, cx| cx.notify());
        let playback = player_bar.read(cx).playback();
        let backdrop = player_bar.read(cx).album_backdrop();
        Self {
            player_bar,
            _player_subscription: player_subscription,
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
                        for comment in page.comments {
                            if !this
                                .comments
                                .iter()
                                .any(|existing| existing.id == comment.id)
                            {
                                this.comments.push(comment);
                            }
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

    pub(in crate::ui) fn open(&mut self, cx: &mut Context<Self>) {
        if !self.opened {
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
            Transition::new(REVEAL_DURATION).ease(ease_out_quint()),
            window,
            cx,
        );
        let summary_top = page_height + WINDOW_HEADER_HEIGHT + f32::from(self.scroll.offset().y);
        let show_comments = summary_top <= WINDOW_HEADER_HEIGHT;

        // 内容区宽度同时受可用宽、设计上限和高度约束：又宽又矮的条带窗口按高度收窄，
        // 避免两栏被拉得过开。
        let content_height = (height - SONG_CONTENT_VERTICAL_PADDING * 2.).max(1.);
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
                this.scroll.set_offset(point(px(0.), px(0.)));
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
        div()
            .id("song-lyric-lines")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.lyric_scroll)
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .pt(px(72.))
            .pb(px(160.))
            .children(self.lyrics.iter().enumerate().map(|(index, line)| {
                let time = line.time;
                let active = Some(index) == self.active_lyric;
                div()
                    .id(("song-lyric-line", index))
                    .w_full()
                    .pb(px(22.))
                    .text_size(px(19.))
                    .line_height(px(28.))
                    .text_color(if active { white() } else { white().alpha(0.4) })
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
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
                                .text_size(px(15.))
                                .text_color(white().alpha(0.45))
                                .child(translation),
                        )
                    })
            }))
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
        div()
            .id("album-comments-region")
            .debug_selector(|| "album-comments-region".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .max_w(px(1120.))
            .mx_auto()
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
            .children(self.comments.iter().map(comment_row))
            .when(self.comments_loading, |comments| {
                comments.child(
                    div()
                        .py_5()
                        .text_color(colors.muted_foreground)
                        .child("评论加载中"),
                )
            })
            .when(
                !self.comments_loading && self.comments.is_empty() && self.comments_error.is_none(),
                |comments| {
                    comments.child(
                        div()
                            .py_8()
                            .text_color(colors.muted_foreground)
                            .child("还没有评论"),
                    )
                },
            )
            .when(self.comments_error.is_some(), |comments| {
                comments.child(
                    Button::new("retry-song-comments")
                        .child("评论加载失败，重试")
                        .on_click(cx.listener(|this, _, _, cx| this.load_comments(cx))),
                )
            })
            .when(self.comments_more && !self.comments_loading, |comments| {
                comments.child(
                    Button::new("more-song-comments")
                        .mt_5()
                        .child("加载更多评论")
                        .on_click(cx.listener(|this, _, _, cx| this.load_comments(cx))),
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
        if self.opened {
            self.sync_song(cx);
        }
        let layout = self.layout(window, cx);

        // 跟随播放进度高亮当前歌词行。
        let position = self.playback.read(cx).snapshot().position;
        let active = self
            .lyrics
            .partition_point(|line| line.time <= position)
            .checked_sub(1);
        if active != self.active_lyric {
            self.active_lyric = active;
            if let Some(active) = active {
                self.lyric_scroll.scroll_to_item(active);
            }
        }

        // 两张唱片要用当前封面纹理，唱臂要推进动画，都先算好再交给分区。
        let arm_angle = {
            let playback = self.playback.read(cx).snapshot();
            self.tonearm
                .angle(playback.is_playing, playback.revision, window, cx)
        };
        let vinyl = (layout.reveal > 0. && layout.summary_top > 0.).then(|| {
            self.player_bar.update(cx, |player, cx| {
                player
                    .vinyl(layout.side, "images/disc.png", window, cx)
                    .debug_selector(|| "album-vinyl-image".into())
            })
        });
        let summary_vinyl = self.player_bar.update(cx, |player, cx| {
            player
                .vinyl(45., "images/miniVinyl.png", window, cx)
                .debug_selector(|| "album-summary-vinyl".into())
        });

        // 先构建两块内容区，再自底向上拼装。
        let lyric_content = self.render_lyrics(&layout, cx);
        let tab_content = self.render_tab(&layout, lyric_content);
        let song_info = self.render_song_info(&layout, tab_content, cx);
        let comments = self.render_comments(&layout, cx);
        let summary = self.render_summary(&layout, summary_vinyl, cx);
        let song_page = build_song_page(&layout, song_info, vinyl, arm_angle);

        // 全屏页外壳：背景渐变 + 外层滚动（唱片页叠评论页）+ 两层顶栏。
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
            .child(page_header(&layout))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 独立视图
// ─────────────────────────────────────────────────────────────────────────

/// 唱片区：居中内容区，左唱片右信息，整体受内容宽/唱片边长约束。
fn build_song_page(
    layout: &Layout,
    song_info: AnyElement,
    vinyl: Option<Div>,
    arm_angle: f32,
) -> AnyElement {
    div()
        .h(px(layout.height))
        .w_full()
        .px(px(SONG_CONTENT_PADDING))
        .py(px(SONG_CONTENT_VERTICAL_PADDING))
        .flex()
        .justify_center()
        .child(
            div()
                .debug_selector(|| "album-song-content".into())
                .w(px(layout.content_width))
                .h_full()
                .flex()
                .child(artwork(layout.side, vinyl, arm_angle))
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
fn artwork(side: f32, disc: Option<Div>, arm_angle: f32) -> impl IntoElement {
    div()
        .id("album-artwork-region")
        .debug_selector(|| "album-artwork-region".into())
        .w(relative(0.5))
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .justify_start()
        .child(vinyl_stage(side, disc, arm_angle))
}

// ─────────────────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
