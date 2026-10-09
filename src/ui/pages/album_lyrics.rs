use std::time::Duration;

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, Theme, Transition, transition};
use gpui_kit::component::Sizable;
use gpui_kit::component::TitleBar;
use gpui_kit::component::avatar::Avatar;

use crate::api::MusicApi;
use crate::models::{LyricLine, SongComment};
use crate::playback::PlaybackController;
use crate::ui::assets::thumbnail_url;
use crate::ui::components::{PlayerBar, window_drag_area};
use crate::ui::cover_color::{Backdrop, CoverColor, dark_colors, dark_gradient, gradient_layer};
use crate::ui::shell::{PAGE_HEADER_HEIGHT, WINDOW_HEADER_HEIGHT, hover_icon};
const SUMMARY_HEIGHT: f32 = 72.;
const SONG_CONTENT_MAX_WIDTH: f32 = 1400.;
const SONG_CONTENT_PADDING: f32 = 100.;
const SONG_CONTENT_VERTICAL_PADDING: f32 = 24.;

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
}

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
        }
    }

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

impl Render for AlbumLyrics {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.opened {
            self.sync_song(cx);
        }
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
        // 外层两屏等高；每屏的顶栏与正文共同占满播放器上方的空间。
        let page_height = (f32::from(window.viewport_size().height) - 86.).max(1.);
        let height = (page_height - WINDOW_HEADER_HEIGHT - PAGE_HEADER_HEIGHT).max(1.);
        let reveal = transition(
            "album-lyrics-reveal",
            if self.opened { 1_f32 } else { 0. },
            Transition::new(Duration::from_millis(500)).ease(ease_out_quint()),
            window,
            cx,
        );
        let summary_top = page_height + WINDOW_HEADER_HEIGHT + f32::from(self.scroll.offset().y);
        let show_comments = summary_top <= WINDOW_HEADER_HEIGHT;
        let content_width = (f32::from(window.viewport_size().width) - SONG_CONTENT_PADDING * 2.)
            .clamp(1., SONG_CONTENT_MAX_WIDTH);
        let content_height = (height - SONG_CONTENT_VERTICAL_PADDING * 2.).max(1.);
        // 内容宽 856 时直径 340，内容宽 1400 时直径 540；中间尺寸线性缩放。
        let side = (340. + (content_width - 856.) * 200. / (SONG_CONTENT_MAX_WIDTH - 856.))
            .min(content_width * 0.4)
            .min(content_height * 0.64)
            .floor()
            .max(1.);
        let vinyl = (reveal > 0. && summary_top > 0.).then(|| {
            self.player_bar.update(cx, |player, cx| {
                player
                    .vinyl(side, "images/disc.png", window, cx)
                    .debug_selector(|| "album-vinyl-image".into())
            })
        });
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
        let summary = div()
            .debug_selector(|| "album-song-summary".into())
            .w_full()
            .max_w(px(700.))
            .h(px(48.))
            .px_3()
            .flex()
            .items_center()
            .gap_3()
            .rounded_full()
            .border_1()
            .border_color(white().alpha(0.1))
            .bg(white().alpha(0.04))
            .occlude()
            .when_some(cover_url.clone(), |summary, url| {
                summary.child(
                    img(url)
                        .size(px(38.))
                        .rounded_full()
                        .object_fit(ObjectFit::Cover),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(17.))
                    .text_color(white())
                    .truncate()
                    .child(title.clone()),
            )
            .child(
                div()
                    .max_w(px(240.))
                    .text_size(px(13.))
                    .text_color(colors.muted_foreground)
                    .truncate()
                    .child(artists.clone()),
            )
            .child(
                Button::new("return-to-vinyl")
                    .aria_label("回到唱片")
                    .child(
                        svg()
                            .path("icons/unfold.svg")
                            .size(px(16.))
                            .text_color(white().alpha(0.6))
                            .with_transformation(Transformation::rotate(radians(
                                std::f32::consts::PI,
                            ))),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.scroll.set_offset(point(px(0.), px(0.)));
                        cx.notify();
                    })),
            );

        let lyric_content = if self.lyrics.is_empty() {
            div()
                .pt(px(80.))
                .text_color(colors.muted_foreground)
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
                .into_any_element()
        } else {
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
                    div()
                        .id(("song-lyric-line", index))
                        .w_full()
                        .pb(px(22.))
                        .text_size(px(19.))
                        .line_height(px(28.))
                        .text_color(if Some(index) == self.active_lyric {
                            white()
                        } else {
                            white().alpha(0.4)
                        })
                        .font_weight(if Some(index) == self.active_lyric {
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
        };

        let comment_content = div()
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
            .px(px(40.))
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
            .children(self.comments.iter().map(|comment| comment_row(comment)))
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
            });

        let song_info = div()
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
                    .child(title.clone()),
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
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(format!("专辑：{album}")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(format!("歌手：{artists}")),
                    )
                    .when_some(source, |metadata, source| {
                        metadata.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(format!("来源：{source}")),
                        )
                    }),
            )
            // 歌词、百科、推荐切换胶囊
            .child(
                div()
                    .mt(px(24.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_full()
                    .bg(white().alpha(0.06))
                    .w(px(180.))
                    .p_1()
                    .text_size(px(14.))
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .bg(white().alpha(0.12))
                            .text_color(white())
                            .child("歌词"),
                    )
                    .child(
                        Button::new("song-encyclopedia")
                            .disabled(true)
                            .text_color(colors.muted_foreground)
                            .child("百科"),
                    )
                    .child(
                        Button::new("song-similar")
                            .disabled(true)
                            .text_color(colors.muted_foreground)
                            .child("相似推荐"),
                    ),
            )
            .child(lyric_content);
        let playing = self.playback.read(cx).is_play_requested();
        let song_page = div()
            .h(px(height))
            .w_full()
            .px(px(SONG_CONTENT_PADDING))
            .py(px(SONG_CONTENT_VERTICAL_PADDING))
            .flex()
            .justify_center()
            .child(
                div()
                    .debug_selector(|| "album-song-content".into())
                    .size_full()
                    .max_w(px(SONG_CONTENT_MAX_WIDTH))
                    .flex()
                    .child(artwork(side, vinyl, playing, window, cx))
                    .child(song_info),
            );

        div()
            .absolute()
            .left_0()
            .top(relative(1. - reveal))
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .occlude()
            .child(
                gradient_layer("album-background-color", gradient, self.backdrop.clone())
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
                                    .h(px(page_height))
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
                                            .child(deferred(summary)),
                                    )
                                    .child(comment_content),
                            ),
                    )
                    .when(show_comments, |content| {
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
                                        .bg(hsla(0., 0., 0.35, 1.))
                                        .text_color(white())
                                        .child("发布评论"),
                                ),
                        )
                    }),
            )
            // 唱片能从透明顶栏下方透出；评论滚动区的边界位于胶囊下方，无需额外遮罩。
            .child(
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
                            colors,
                        )
                        .ml_0()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.opened = false;
                            cx.notify();
                        })),
                    ),
            )
            .child(
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
                    .when(!show_comments, |header| {
                        header.child(
                            div()
                                .text_color(colors.muted_foreground)
                                .text_size(px(13.))
                                .child("播放器模式"),
                        )
                    })
                    .child(hover_icon(
                        "album-header-mini-button",
                        "迷你模式",
                        "icons/menu_mini.svg",
                        colors,
                    )),
            )
    }
}

fn tonearm_angle(playing: bool, window: &mut Window, cx: &mut App) -> f32 {
    transition(
        "album-tonearm-angle",
        if playing { 0. } else { -33. },
        Transition::new(Duration::from_millis(350)).ease(ease_in_out),
        window,
        cx,
    )
}

fn artwork(
    side: f32,
    vinyl: Option<Div>,
    playing: bool,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let scale = side / 340.;
    let arm_side = 348. * scale;
    let angle = tonearm_angle(playing, window, cx).to_radians();
    div()
        .id("album-artwork-region")
        .debug_selector(|| "album-artwork-region".into())
        .w(relative(0.5))
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .justify_start()
        .child(
            div()
                .relative()
                .size(px(side))
                .flex_none()
                .child(
                    div()
                        .size_full()
                        .debug_selector(|| "album-vinyl-ring".into())
                        .rounded_full()
                        .border_1()
                        .border_color(white().alpha(0.08))
                        .bg(white().alpha(0.025))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(vinyl),
                )
                // SVG 画布以支点为中心；唱片与唱臂只共用一个缩放比例，无需旋转位置补偿。
                .child(
                    img("images/vinylHandle.svg")
                        .debug_selector(|| "album-tonearm".into())
                        .absolute()
                        .left(px(side * 0.5 - arm_side * 0.5))
                        .top(px(-80. * scale - arm_side * 0.5))
                        .size(px(arm_side))
                        .with_transformation(Transformation::rotate(radians(angle))),
                ),
        )
}

fn comment_row(comment: &SongComment) -> impl IntoElement {
    let date = time::OffsetDateTime::from_unix_timestamp(comment.time / 1000)
        .ok()
        .map(|date| {
            format!(
                "{}-{:02}-{:02}",
                date.year(),
                u8::from(date.month()),
                date.day()
            )
        })
        .unwrap_or_default();
    div()
        .w_full()
        .flex()
        .gap(px(14.))
        .py(px(18.))
        .border_b_1()
        .border_color(white().alpha(0.07))
        .child(
            Avatar::new()
                .with_size(px(40.))
                .src(thumbnail_url(&comment.avatar_url, 80)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_color(rgb(0x89a6d4))
                        .text_size(px(14.))
                        .child(comment.nickname.clone()),
                )
                .child(
                    div()
                        .mt_2()
                        .text_color(white().alpha(0.9))
                        .text_size(px(15.))
                        .line_height(px(23.))
                        .child(comment.content.clone()),
                )
                .child(
                    div()
                        .mt_3()
                        .flex()
                        .items_center()
                        .text_size(px(12.))
                        .text_color(white().alpha(0.45))
                        .child(date)
                        .child(
                            div()
                                .ml_auto()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(comment.liked_count.to_string())
                                .child(
                                    svg()
                                        .path("icons/like_outline.svg")
                                        .size(px(17.))
                                        .text_color(white().alpha(0.5)),
                                )
                                .child(
                                    svg()
                                        .path("icons/comment.svg")
                                        .size(px(17.))
                                        .ml_3()
                                        .text_color(white().alpha(0.5)),
                                ),
                        ),
                ),
        )
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn tonearm_animates_and_retargets_from_the_displayed_angle(cx: &mut gpui::TestAppContext) {
        use super::tonearm_angle;
        use gpui::prelude::*;
        use gpui::{Context, IntoElement, Render, Window, div, px, size};
        use std::time::Duration;
        struct Host {
            playing: bool,
            angle: f32,
        }
        impl Render for Host {
            fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.angle = tonearm_angle(self.playing, window, cx);
                div().size_full()
            }
        }
        let window = cx.open_window(size(px(100.), px(100.)), |_, _| Host {
            playing: true,
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
        cx.executor().advance_clock(Duration::from_millis(350));
        assert_eq!(draw(cx), 0.);
        window
            .update(cx, |host, _, _| host.playing = false)
            .unwrap();
        assert_eq!(draw(cx), 0.);
        cx.executor().advance_clock(Duration::from_millis(350));
        assert_eq!(draw(cx), -33.);
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn expanded_layout_has_two_columns_and_scrolls_to_comments(cx: &mut gpui::TestAppContext) {
        use super::*;
        use crate::state::library::MusicLibrary;
        struct Host {
            page: Entity<AlbumLyrics>,
            player: Entity<PlayerBar>,
        }
        impl Render for Host {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child(self.page.clone()),
                    )
                    .child(self.player.clone())
            }
        }
        cx.update(gpui_kit::init);
        let playback = cx.new(|_| PlaybackController::default());
        let library = cx.new(|_| MusicLibrary::default());
        let window = cx.open_window(size(px(1280.), px(800.)), |window, cx| {
            let player = cx.new(|cx| PlayerBar::new(playback, library, window, cx));
            player.update(cx, |player, cx| player.set_album_expanded(true, cx));
            let page = cx.new(|cx| {
                let mut page = AlbumLyrics::new(player.clone(), cx);
                page.opened = true;
                page.lyrics.push(LyricLine {
                    time: Duration::ZERO,
                    text: "测试歌词".into(),
                    translation: None,
                });
                page.comments = (0..10)
                    .map(|id| SongComment {
                        id,
                        nickname: "测试用户".into(),
                        avatar_url: String::new(),
                        content: "测试评论".into(),
                        time: 0,
                        liked_count: 0,
                    })
                    .collect();
                page
            });
            Host { page, player }
        });
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        };
        draw(cx);
        draw(cx);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let artwork = visual.debug_bounds("album-artwork-region").unwrap();
        let lyrics = visual.debug_bounds("album-lyrics-region").unwrap();
        let comments = visual.debug_bounds("album-comments-region").unwrap();
        assert!(artwork.right() <= lyrics.left());
        assert!(artwork.size.width > px(300.) && lyrics.size.width > px(300.));
        assert!(comments.top() >= artwork.bottom());
        let ring = visual.debug_bounds("album-vinyl-ring").unwrap();
        assert_eq!(ring, visual.debug_bounds("album-vinyl-image").unwrap());
        assert_eq!(
            visual.debug_bounds("album-song-summary").unwrap().top(),
            px(744.)
        );
        let scroll_to = |offset: f32, cx: &mut TestAppContext| {
            window
                .update(cx, |host, _, cx| {
                    host.page.update(cx, |page, cx| {
                        page.scroll.set_offset(point(px(0.), px(-offset)));
                        cx.notify();
                    });
                })
                .unwrap();
            draw(cx);
            draw(cx);
        };
        // 内容真实地绘制到顶栏背后，裁剪边界没有停在两层顶栏下面。
        scroll_to(f32::from(ring.top()) - 10., cx);
        cx.update_window(window.into(), |_, window, _| {
            let quads = window.painted_quads();
            let scale = window.scale_factor();
            let ring = quads
                .iter()
                .find(|quad| {
                    (quad.bounds.origin.y.0 - 10. * scale).abs() < 0.01
                        && (quad.bounds.size.width.0 - f32::from(ring.size.width) * scale).abs()
                            <= 2.
                })
                .unwrap();
            assert_eq!(ring.content_mask.bounds.origin.y.0, 0.);
        })
        .unwrap();
        // 评论页恰好一屏，外层到第二屏末端后，胶囊自然停在 TitleBar 下沿。
        for (offset, top) in [
            (50., 694.),
            (200., 544.),
            (500., 244.),
            (714., 30.),
            (850., 30.),
        ] {
            scroll_to(offset, cx);
            assert_eq!(
                visual.debug_bounds("album-song-summary").unwrap().top(),
                px(top)
            );
        }
        scroll_to(714., cx);
        cx.update_window(window.into(), |_, window, _| {
            let gradients: Vec<_> = window
                .painted_quads()
                .into_iter()
                .filter(|quad| quad.background.as_solid().is_none())
                .collect();
            assert_eq!(gradients.len(), 1, "两屏共用背景，无需复制渐变遮住评论");
        })
        .unwrap();
        assert_eq!(
            visual.debug_bounds("album-comments-region").unwrap().top(),
            px(WINDOW_HEADER_HEIGHT + SUMMARY_HEIGHT)
        );
        assert_eq!(
            visual
                .debug_bounds("album-comments-page")
                .unwrap()
                .size
                .height,
            px(714.)
        );
        let heading_top = visual.debug_bounds("album-comments-heading").unwrap().top();
        let wheel = |delta, visual: &mut VisualTestContext, cx: &mut TestAppContext| {
            visual.simulate_event(ScrollWheelEvent {
                position: point(px(640.), px(250.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                touch_phase: TouchPhase::Moved,
                ..Default::default()
            });
            draw(cx);
            draw(cx);
        };
        wheel(-120., &mut visual, cx);
        assert_eq!(
            visual.debug_bounds("album-comments-heading").unwrap().top(),
            heading_top - px(120.)
        );
        assert_eq!(
            visual.debug_bounds("album-song-summary").unwrap().top(),
            px(30.)
        );
        window
            .update(cx, |host, _, cx| {
                let page = host.page.read(cx);
                assert_eq!(page.scroll.offset().y, px(-714.));
                assert_eq!(page.comment_scroll.offset().y, px(-120.));
            })
            .unwrap();
        // 向上滚动先消费评论正文，再把越过顶部的剩余距离交给外层。
        wheel(40., &mut visual, cx);
        window
            .update(cx, |host, _, cx| {
                let page = host.page.read(cx);
                assert_eq!(page.scroll.offset().y, px(-714.));
                assert_eq!(page.comment_scroll.offset().y, px(-80.));
            })
            .unwrap();
        wheel(100., &mut visual, cx);
        window
            .update(cx, |host, _, cx| {
                let page = host.page.read(cx);
                assert_eq!(page.scroll.offset().y, px(-694.));
                assert_eq!(page.comment_scroll.offset().y, px(0.));
            })
            .unwrap();
        // 空评论与长列表都不改变外层两屏的长度。
        window
            .update(cx, |host, _, cx| {
                host.page.update(cx, |page, cx| {
                    let mut comment = page.comments[0].clone();
                    for id in 10..50 {
                        comment.id = id;
                        page.comments.push(comment.clone());
                    }
                    cx.notify();
                });
            })
            .unwrap();
        draw(cx);
        window
            .update(cx, |host, _, cx| {
                assert_eq!(host.page.read(cx).scroll.max_offset().y, px(714.))
            })
            .unwrap();
        scroll_to(0., cx);
        let mut diameters = Vec::new();
        for (width, height, expected_width) in [
            (1056., 736., 856.),
            (1920., 1080., 1400.),
            (2400., 1080., 1400.),
            (1056., 1000., 856.),
        ] {
            visual.simulate_resize(size(px(width), px(height)));
            draw(cx);
            draw(cx);
            let content = visual.debug_bounds("album-song-content").unwrap();
            let artwork = visual.debug_bounds("album-artwork-region").unwrap();
            let lyrics = visual.debug_bounds("album-lyrics-region").unwrap();
            let ring = visual.debug_bounds("album-vinyl-ring").unwrap();
            let arm = visual.debug_bounds("album-tonearm").unwrap();
            assert!(
                (f32::from(content.left()) - (width - f32::from(content.right()))).abs() < 0.01
            );
            assert!(content.size.width <= px(SONG_CONTENT_MAX_WIDTH));
            assert_eq!(content.size.width, px(expected_width));
            assert_eq!(artwork.right(), lyrics.left());
            assert_eq!(artwork.size.width, lyrics.size.width);
            assert_eq!(ring.left(), content.left());
            assert!(
                (f32::from(ring.center().y - content.center().y)).abs() < 0.01,
                "window={width}x{height}, content={content:?}, artwork={artwork:?}, ring={ring:?}"
            );
            assert_eq!(ring, visual.debug_bounds("album-vinyl-image").unwrap());
            let scale = f32::from(ring.size.width) / 340.;
            assert!((f32::from(arm.size.width) - 348. * scale).abs() <= 0.5);
            assert_eq!(arm.size.width, arm.size.height);
            // SVG 支点就是画布中心，任意播放角度都不会改变支点坐标。
            let pivot = arm.center();
            assert!((f32::from(pivot.x - ring.center().x)).abs() <= 0.5);
            assert!((f32::from(pivot.y - ring.top()) + 80. * scale).abs() <= 0.5);
            assert!(pivot.y > content.top() && ring.bottom() < content.bottom());
            window
                .update(cx, |host, _, cx| {
                    assert_eq!(host.page.read(cx).scroll.max_offset().y, px(height - 86.));
                })
                .unwrap();
            diameters.push(ring.size.width);
        }
        assert!((f32::from(diameters[0]) - 340.).abs() <= 3.);
        assert_eq!(diameters[1], px(540.));
        assert!(diameters[1] > diameters[0]);
        assert_eq!(diameters[1], diameters[2]);
        assert!(diameters[3] < diameters[1]);
        window
            .update(cx, |host, _, cx| {
                host.page.update(cx, |page, cx| {
                    page.comments.clear();
                    cx.notify();
                });
            })
            .unwrap();
        draw(cx);
        window
            .update(cx, |host, _, cx| {
                assert_eq!(host.page.read(cx).scroll.max_offset().y, px(914.))
            })
            .unwrap();
    }
}
