use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, Theme, Transition, transition};

use super::ScrollTween;
use crate::api::MusicApi;
use crate::models::LyricLine;
use crate::playback::PlaybackController;
use crate::ui::cover_color::{Backdrop, CoverGradient, dark_colors, dark_gradient};

const LYRIC_FADE_HEIGHT: f32 = 96.;
const LYRIC_TOP_PADDING: f32 = 20.;
const LYRIC_ANCHOR_RATIO: f32 = 0.4;
const LYRIC_LINE_GAP: f32 = 20.;
const LYRIC_LINE_HEIGHT: f32 = 28.;
const LYRIC_TEXT: f32 = 20.;
const LYRIC_ACTIVE_ALPHA: f32 = 1.;
const LYRIC_NEAR_ALPHA: f32 = 0.65;
const LYRIC_IDLE_ALPHA: f32 = 0.4;
const LYRIC_HIGHLIGHT_DURATION: Duration = Duration::from_millis(220);
const LYRIC_MANUAL_PAUSE: Duration = Duration::from_secs(3);
const LYRIC_TRANSLATION_TEXT: f32 = 17.;
const LYRIC_TRANSLATION_LINE_HEIGHT: f32 = 20.;
const LYRIC_TRANSLATION_ACTIVE_ALPHA: f32 = 0.45;
const LYRIC_TRANSLATION_IDLE_ALPHA: f32 = 0.25;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum LyricFollow {
    #[default]
    Auto,
    Manual(Instant),
}

pub(super) struct LyricsView {
    playback: Entity<PlaybackController>,
    _playback_subscription: Subscription,
    backdrop: Backdrop,
    song_id: Option<u64>,
    generation: u64,
    lyrics: Vec<LyricLine>,
    visible: Vec<usize>,
    active_line: Option<usize>,
    active: bool,
    loading: bool,
    error: Option<String>,
    request: Option<tokio::task::AbortHandle>,
    scroll: ScrollHandle,
    tween: ScrollTween,
    follow: LyricFollow,
    manual_task: Option<Task<()>>,
    layout_pending: bool,
    viewport_size: Size<Pixels>,
    backdrop_gradient: Option<CoverGradient>,
}

impl LyricsView {
    pub(super) fn new(
        playback: Entity<PlaybackController>,
        backdrop: Backdrop,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&playback, |this, playback, cx| {
            let active = this.line_at(playback.read(cx).snapshot().position);
            if this.active_line != active {
                this.active_line = active;
                if this.active {
                    cx.notify();
                }
            }
        });
        Self {
            playback,
            _playback_subscription: subscription,
            backdrop,
            song_id: None,
            generation: 0,
            lyrics: Vec::new(),
            visible: Vec::new(),
            active_line: None,
            active: false,
            loading: false,
            error: None,
            request: None,
            scroll: ScrollHandle::default(),
            tween: ScrollTween::default(),
            follow: LyricFollow::Auto,
            manual_task: None,
            layout_pending: false,
            viewport_size: Size::default(),
            backdrop_gradient: None,
        }
    }

    pub(super) fn set_song(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        if self.song_id == id {
            return;
        }
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.song_id = id;
        self.generation = self.generation.wrapping_add(1);
        self.manual_task = None;
        self.follow = LyricFollow::Auto;
        self.tween = ScrollTween::default();
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.lyrics.clear();
        self.visible.clear();
        self.active_line = None;
        self.loading = false;
        self.error = None;
        self.layout_pending = true;
        if id.is_some() {
            self.load(cx);
        }
        cx.notify();
    }

    pub(super) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        self.manual_task = None;
        self.follow = LyricFollow::Auto;
        self.tween
            .aim(&self.scroll, None, cx.background_executor().now());
        self.layout_pending = active;
        cx.notify();
    }

    pub(super) fn configure_backdrop(
        &mut self,
        gradient: Option<CoverGradient>,
        cx: &mut Context<Self>,
    ) {
        if self.backdrop_gradient != gradient {
            self.backdrop_gradient = gradient;
            cx.notify();
        }
    }

    fn line_at(&self, position: Duration) -> Option<usize> {
        self.lyrics
            .partition_point(|line| line.time <= position)
            .checked_sub(1)
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(id) = self.song_id else { return };
        self.loading = true;
        self.error = None;
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::song_lyrics(api.client.clone(), id));
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request.await.unwrap_or_else(|_| Err("歌词加载失败".into()));
            let _ = this.update(cx, |this, cx| {
                // abort 与完成可能相撞；代次检查拒绝已经排入主线程的旧结果。
                if this.song_id != Some(id) || this.generation != generation {
                    return;
                }
                this.loading = false;
                this.request = None;
                match result {
                    Ok(lyrics) => {
                        this.visible = visible_lyrics(&lyrics);
                        this.lyrics = lyrics;
                        this.active_line = this.line_at(this.playback.read(cx).snapshot().position);
                        this.layout_pending = true;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn manual_scroll(&mut self, cx: &mut Context<Self>) {
        if !self.active {
            return;
        }
        let executor = cx.background_executor().clone();
        let deadline = executor.now() + LYRIC_MANUAL_PAUSE;
        self.follow = LyricFollow::Manual(deadline);
        self.tween.aim(&self.scroll, None, executor.now());
        // 替换 Task 会取消上次滚轮的计时；暂停播放也能独立唤醒视图。
        let timer = executor.timer(LYRIC_MANUAL_PAUSE);
        let generation = self.generation;
        self.manual_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                if this.active
                    && this.generation == generation
                    && this.follow == LyricFollow::Manual(deadline)
                {
                    this.follow = LyricFollow::Auto;
                    this.manual_task = None;
                    cx.notify();
                }
            });
        }));
    }
}

impl Render for LyricsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = dark_colors(
            Theme::global(cx).tokens.colors,
            self.backdrop
                .gradient()
                .unwrap_or_else(|| dark_gradient(None))
                .0[1]
                .into(),
        );
        let highlight = self
            .active_line
            .and_then(|index| self.visible.binary_search(&index).ok());
        let anchor = self
            .active_line
            .and_then(|index| lyric_slot(&self.visible, index));
        let interlude = self.active_line.is_some() && highlight.is_none();
        if self.active && !self.visible.is_empty() {
            if self.layout_pending {
                self.layout_pending = false;
                // 新行的 bounds 由本帧 prepaint 写入；下一帧再求滚动锚点。
                cx.on_next_frame(window, |_, _, cx| cx.notify());
            } else if self.follow == LyricFollow::Auto {
                let desired = anchor.and_then(|row| {
                    self.scroll.bounds_for_item(row).map(|item| {
                        anchored_offset(self.scroll.bounds(), item, self.scroll.max_offset().y)
                    })
                });
                let now = cx.background_executor().now();
                self.tween.aim(&self.scroll, desired, now);
                self.tween.step(&self.scroll, window, now);
            }
        }
        if self.visible.is_empty() {
            return div()
                .size_full()
                .pt(px(80.))
                .text_color(colors.muted_foreground)
                .child(if self.loading {
                    "歌词加载中"
                } else if self.error.is_some() {
                    "歌词加载失败"
                } else {
                    "暂无歌词"
                })
                .when(self.error.is_some(), |content| {
                    content.child(
                        Button::new("retry-song-lyrics")
                            .ml_2()
                            .child("重试")
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    )
                })
                .into_any_element();
        }
        let view = cx.weak_entity();
        div()
            .size_full()
            .relative()
            .on_children_prepainted(move |_, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    let size = this.scroll.bounds().size;
                    if this.viewport_size != size {
                        this.viewport_size = size;
                        if this.active {
                            // 调整窗口后要用新 bounds 重新求锚点，暂停时也能完成定位。
                            cx.on_next_frame(window, |_, _, cx| cx.notify());
                        }
                    }
                });
            })
            .child(
                div()
                    .id("song-lyric-lines")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                        if this.active {
                            this.manual_scroll(cx);
                            cx.stop_propagation();
                        }
                    }))
                    .pt(px(LYRIC_TOP_PADDING))
                    .pb(px(160.))
                    // children 在构建时立即收集，可直接在迭代器内采样 transition。
                    .children(self.visible.iter().enumerate().map(|(row, &index)| {
                        let line = &self.lyrics[index];
                        let time = line.time;
                        let weight = transition(
                            ElementId::Name(
                                format!("song-lyric-weight-{}-{row}", self.generation).into(),
                            ),
                            lyric_emphasis(row, highlight),
                            Transition::new(LYRIC_HIGHLIGHT_DURATION).ease(ease_out_quint()),
                            window,
                            cx,
                        );
                        let emphasis =
                            (weight - LYRIC_IDLE_ALPHA) / (LYRIC_ACTIVE_ALPHA - LYRIC_IDLE_ALPHA);
                        div()
                            .id(("song-lyric-line", row))
                            .w_full()
                            .text_size(px(LYRIC_TEXT))
                            .line_height(px(LYRIC_LINE_HEIGHT))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white().alpha(weight))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.playback
                                    .update(cx, |playback, cx| playback.seek_to(time, cx));
                            }))
                            .child(line.text.clone())
                            .when_some(line.translation.clone(), |line, translation| {
                                line.child(
                                    div()
                                        .text_size(px(LYRIC_TRANSLATION_TEXT))
                                        .line_height(px(LYRIC_TRANSLATION_LINE_HEIGHT))
                                        .text_color(white().alpha(
                                            LYRIC_TRANSLATION_IDLE_ALPHA
                                                + (LYRIC_TRANSLATION_ACTIVE_ALPHA
                                                    - LYRIC_TRANSLATION_IDLE_ALPHA)
                                                    * emphasis,
                                        ))
                                        .child(translation),
                                )
                            })
                            .child(
                                div()
                                    .h(px(LYRIC_LINE_GAP))
                                    .overflow_hidden()
                                    .text_size(px(LYRIC_TRANSLATION_TEXT))
                                    .line_height(px(LYRIC_TRANSLATION_LINE_HEIGHT))
                                    .text_color(white().alpha(LYRIC_TRANSLATION_ACTIVE_ALPHA))
                                    .when(interlude && Some(row) == anchor, |slot| {
                                        slot.child("···")
                                    }),
                            )
                    })),
            )
            .child(lyrics_top_fade(self.backdrop.clone(), self.scroll.clone()))
            .child(lyrics_fade(self.backdrop.clone()))
            .into_any_element()
    }
}

fn lyric_emphasis(row: usize, highlight: Option<usize>) -> f32 {
    match highlight.map(|active| row.abs_diff(active)) {
        Some(0) => LYRIC_ACTIVE_ALPHA,
        Some(1) => LYRIC_NEAR_ALPHA,
        _ => LYRIC_IDLE_ALPHA,
    }
}

fn anchored_offset(viewport: Bounds<Pixels>, item: Bounds<Pixels>, max: Pixels) -> Pixels {
    let anchor = f32::from(viewport.top()) + f32::from(viewport.size.height) * LYRIC_ANCHOR_RATIO;
    px((anchor - f32::from(item.center().y)).clamp(-f32::from(max), 0.))
}

// Canvas 不插入 hitbox；背景在 paint 现采，外层滚动后仍对齐。
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

fn lyrics_top_fade(backdrop: Backdrop, scroll: ScrollHandle) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let strength = (-f32::from(scroll.offset().y) / LYRIC_FADE_HEIGHT).clamp(0., 1.);
            backdrop.paint_top_fade(bounds, strength, window);
        },
    )
    .absolute()
    .left_0()
    .top_0()
    .w_full()
    .h(px(LYRIC_FADE_HEIGHT))
}

fn is_blank(line: &LyricLine) -> bool {
    line.text.trim().is_empty()
        && line
            .translation
            .as_ref()
            .is_none_or(|text| text.trim().is_empty())
}

fn visible_lyrics(lyrics: &[LyricLine]) -> Vec<usize> {
    lyrics
        .iter()
        .enumerate()
        .filter(|(_, line)| !is_blank(line))
        .map(|(index, _)| index)
        .collect()
}

fn lyric_slot(visible: &[usize], active: usize) -> Option<usize> {
    visible
        .partition_point(|&index| index <= active)
        .checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::{is_blank, lyric_slot, visible_lyrics};
    use crate::models::LyricLine;
    use std::time::Duration;

    #[test]
    fn blank_lines_hold_no_row_but_keep_the_previous_one_anchored() {
        let line = |text: &str, translation: Option<&str>| LyricLine {
            time: Duration::ZERO,
            text: text.into(),
            translation: translation.map(str::to_owned),
        };
        let lyrics = vec![
            line("一", None),
            line("", None),
            line("二", None),
            line("  ", None),
            line("", Some("二译")),
        ];
        assert_eq!(visible_lyrics(&lyrics), vec![0, 2, 4]);
        assert!(!is_blank(&lyrics[4]) && is_blank(&lyrics[3]));
        for (index, slot) in [0, 0, 1, 1, 2].into_iter().enumerate() {
            assert_eq!(lyric_slot(&[0, 2, 4], index), Some(slot));
        }
    }
}
