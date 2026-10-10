//! 黑胶全屏页：外层滚动、背景、元信息与唱片协调。

mod comments;
mod lyrics;

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme, Transition, transition};
use gpui_kit::component::TitleBar;

use crate::playback::PlaybackController;
use crate::state::library::MusicLibrary;
use crate::ui::assets::track_cover_url;
use crate::ui::components::{
    ALBUM_REVEAL_DURATION, PLAYER_BAR_HEIGHT, RotationClock, Tonearm, Vinyl, format_duration,
    window_drag_area,
};
use crate::ui::cover_color::{
    Backdrop, CoverColor, CoverGradient, dark_colors, dark_gradient, gradient_layer,
};
use crate::ui::shell::{PAGE_HEADER_HEIGHT, WINDOW_HEADER_HEIGHT, hover_icon};
use comments::{CommentsView, ScrollBack};
use lyrics::LyricsView;

const SUMMARY_HEIGHT: f32 = 72.;
const SONG_CONTENT_MAX_WIDTH: f32 = 1400.;
const SONG_CONTENT_PADDING: f32 = 100.;
const SONG_CONTENT_TOP_PADDING: f32 = 24.;
const VINYL_MAX_SIDE: f32 = 540.;
const CONTENT_MAX_ASPECT: f32 = 1.6;
const SUMMARY_WIDTH_RATIO: f32 = 0.8;
const SCROLL_DURATION: Duration = Duration::from_millis(600);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SongTab {
    #[default]
    Lyrics,
    Encyclopedia,
    Similar,
}

pub(in crate::ui) struct AlbumLyrics {
    playback: Entity<PlaybackController>,
    library: Entity<MusicLibrary>,
    _library_subscription: Subscription,
    _playback_subscription: Subscription,
    _comments_subscription: Subscription,
    lyrics: Entity<LyricsView>,
    comments: Entity<CommentsView>,
    vinyl: Entity<Vinyl>,
    summary_vinyl: Entity<Vinyl>,
    opened: bool,
    scroll: ScrollHandle,
    cover_color: CoverColor,
    backdrop: Backdrop,
    song_id: Option<u64>,
    tonearm: Tonearm,
    tab: SongTab,
    song_scroll: ScrollTween,
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

impl AlbumLyrics {
    pub(in crate::ui) fn new(
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        backdrop: Backdrop,
        clock: Rc<Cell<RotationClock>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let vinyl = cx.new(|cx| Vinyl::new(playback.clone(), clock.clone(), "images/disc.png", cx));
        let summary_vinyl =
            cx.new(|cx| Vinyl::new(playback.clone(), clock, "images/miniVinyl.png", cx));
        let scroll = ScrollHandle::default();
        let lyrics = cx.new(|cx| LyricsView::new(playback.clone(), backdrop.clone(), cx));
        let comments = cx.new(|_| CommentsView::new(scroll.clone(), backdrop.clone()));
        let comments_subscription = cx.subscribe(&comments, |_, _, _: &ScrollBack, cx| cx.notify());
        let mut source = source_name(&library, &playback, cx);
        let library_playback = playback.clone();
        let library_subscription = cx.observe(&library, move |this, library, cx| {
            let next = source_name(&library, &library_playback, cx);
            if next != source {
                source = next;
                if this.opened {
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
            if this.opened {
                this.sync_song(cx);
            } else if changed && this.song_id.is_some() {
                // 隐藏后切歌只取消旧请求；下一次 open 再请求实际歌曲。
                this.song_id = None;
                this.lyrics.update(cx, |view, cx| view.set_song(None, cx));
                this.comments.update(cx, |view, cx| view.set_song(None, cx));
            }
            if this.opened && (changed || next != playback_state) {
                cx.notify();
            }
            playback_state = next;
        });
        Self {
            playback,
            library,
            _library_subscription: library_subscription,
            _playback_subscription: playback_subscription,
            _comments_subscription: comments_subscription,
            lyrics,
            comments,
            vinyl,
            summary_vinyl,
            opened: false,
            scroll,
            cover_color: CoverColor::default(),
            backdrop,
            song_id: None,
            tonearm: Tonearm::default(),
            tab: SongTab::default(),
            song_scroll: ScrollTween::default(),
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
        self.song_scroll = ScrollTween::default();
        self.comments_collapsed = false;
        self.scroll.set_offset(point(px(0.), px(0.)));
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
            self.sync_song(cx);
            cx.notify();
        }
    }

    fn frame_data(&mut self, window: &mut Window, cx: &mut Context<Self>) -> FrameData {
        let cover_url = self
            .playback
            .read(cx)
            .snapshot()
            .current_song
            .as_ref()
            .map(|song| track_cover_url(song.al.pic_url.as_deref(), 480));
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
                let now = cx.background_executor().now();
                this.song_scroll.aim(&this.scroll, None, now);
                this.song_scroll.aim(&this.scroll, Some(px(0.)), now);
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

    fn render_window_header(&self, frame: &FrameData, cx: &mut Context<Self>) -> AnyElement {
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
                    frame.colors,
                )
                .ml_0()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.opened = false;
                    this.lyrics
                        .update(cx, |view, cx| view.set_active(false, cx));
                    this.comments
                        .update(cx, |view, cx| view.configure(false, false, cx));
                    cx.notify();
                })),
            )
            .into_any_element()
    }
}

impl Render for AlbumLyrics {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = cx.background_executor().now();
        let scrolling = self.song_scroll.step(&self.scroll, window, now);
        if self.comments_collapsed && !scrolling {
            self.comments_collapsed = false;
            self.comments
                .update(cx, |view, cx| view.set_collapsed(false, cx));
        }
        let page_height = (f32::from(window.viewport_size().height) - PLAYER_BAR_HEIGHT).max(1.);
        if let Some(previous_height) = self.page_height {
            let offset = f32::from(self.scroll.offset().y);
            if previous_height != page_height {
                self.scroll
                    .set_offset(point(px(0.), px(offset + previous_height - page_height)));
            }
        }
        self.page_height = Some(page_height);
        let frame = self.frame_data(window, cx);
        // Backdrop 在 prepaint 写入实际插值色；其下一帧通知让缓存 fade 重新绘制。
        let backdrop_gradient = self.backdrop.gradient();
        self.lyrics.update(cx, |view, cx| {
            view.configure_backdrop(backdrop_gradient, cx)
        });
        self.lyrics.update(cx, |view, cx| {
            view.set_active(
                self.opened && self.tab == SongTab::Lyrics && !frame.show_comments,
                cx,
            )
        });
        self.comments.update(cx, |view, cx| {
            view.configure(self.opened, frame.show_comments, cx)
        });
        self.vinyl.read(cx).clear();
        self.summary_vinyl.read(cx).clear();
        if !self.opened && frame.reveal <= 0. {
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
                                            .h(px(frame.page_height))
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
                    .child(self.render_window_header(&frame, cx))
                    .child(page_header(&frame)),
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

fn page_header(frame: &FrameData) -> AnyElement {
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
        .when(!frame.show_comments, |header| {
            header.child(
                div()
                    .text_color(frame.colors.muted_foreground)
                    .text_size(px(13.))
                    .child("播放器模式"),
            )
        })
        .child(hover_icon(
            "album-header-mini-button",
            "迷你模式",
            "icons/menu_mini.svg",
            frame.colors,
        ))
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
