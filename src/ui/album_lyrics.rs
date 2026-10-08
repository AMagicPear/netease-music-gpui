use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Theme, Transition, transition};
use gpui_kit::component::TitleBar;

use super::components::{PlayerBar, window_drag_area};
use super::shell::{PAGE_HEADER_HEIGHT, WINDOW_HEADER_HEIGHT, hover_icon};
const COMMENT_HEADER_HEIGHT: f32 = 52.;
const SNAP_DELAY: Duration = Duration::from_millis(160);
const SNAP_DURATION: Duration = Duration::from_millis(260);

pub(super) struct AlbumLyrics {
    player_bar: Entity<PlayerBar>,
    _player_subscription: Subscription,
    opened: bool,
    scroll: ScrollHandle,
    last_scroll: Option<Instant>,
    snap: Option<(Instant, f32, f32)>,
}

impl AlbumLyrics {
    pub(super) fn new(player_bar: Entity<PlayerBar>, cx: &mut Context<Self>) -> Self {
        let player_subscription = cx.observe(&player_bar, |_, _, cx| cx.notify());
        Self {
            player_bar,
            _player_subscription: player_subscription,
            opened: false,
            scroll: ScrollHandle::default(),
            last_scroll: None,
            snap: None,
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.opened
    }

    pub(super) fn open(&mut self, cx: &mut Context<Self>) {
        if !self.opened {
            self.scroll.set_offset(point(px(0.), px(0.)));
            self.last_scroll = None;
            self.snap = None;
            self.opened = true;
            cx.notify();
        }
    }

    fn settle_scroll(&mut self, height: f32, window: &mut Window, cx: &App) {
        let now = Instant::now();
        if let Some(last) = self.last_scroll {
            if now.duration_since(last) >= SNAP_DELAY {
                self.last_scroll = None;
                let offset = -f32::from(self.scroll.offset().y);
                if let Some(target) = snap_target(offset, height) {
                    self.snap = Some((now, offset / height, target / height));
                }
            } else {
                window.request_animation_frame();
            }
        }
        if let Some((started, from, to)) = self.snap {
            let progress = if cx.reduce_motion() {
                1.
            } else {
                (now.duration_since(started).as_secs_f32() / SNAP_DURATION.as_secs_f32()).min(1.)
            };
            let eased = 1. - (1. - progress).powi(3);
            self.scroll
                .set_offset(point(px(0.), px(-(from + (to - from) * eased) * height)));
            if progress == 1. {
                self.snap = None;
            } else {
                window.request_animation_frame();
            }
        }
    }
}

// 只吸附两个区域之间的过渡；进入评论正文后允许自由滚动。
fn snap_target(offset: f32, height: f32) -> Option<f32> {
    if offset > 0. && offset < height {
        Some(if offset > height / 2. { height } else { 0. })
    } else {
        None
    }
}

impl Render for AlbumLyrics {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        // 歌词首屏占满两层顶栏与播放器之间的空间。
        let height = (f32::from(window.viewport_size().height)
            - 86.
            - WINDOW_HEADER_HEIGHT
            - PAGE_HEADER_HEIGHT)
            .max(1.);
        self.settle_scroll(height, window, cx);
        let reveal = transition(
            "album-lyrics-reveal",
            if self.opened { 1_f32 } else { 0. },
            Transition::new(Duration::from_millis(500)).ease(ease_out_quint()),
            window,
            cx,
        );
        let comments_top = (height + f32::from(self.scroll.offset().y)).max(0.);
        let vinyl = (reveal > 0. && comments_top > 0.).then(|| {
            self.player_bar
                .update(cx, |player, cx| player.vinyl(314., window, cx))
        });

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
                TitleBar::new()
                    .on_close_window(|_, window, cx| crate::desktop::close_window(window, cx))
                    .h(px(WINDOW_HEADER_HEIGHT))
                    .w_full()
                    .border_b_0()
                    .bg(colors.background)
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
                            this.last_scroll = None;
                            this.snap = None;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                window_drag_area("album-lyrics-header")
                    .h(px(PAGE_HEADER_HEIGHT))
                    .w_full()
                    .flex_none()
                    .bg(colors.background)
                    .px(px(40.))
                    .flex()
                    .items_center()
                    .child(div().flex_1())
                    .child(hover_icon(
                        "album-header-mini-button",
                        "迷你模式",
                        "icons/menu_mini.svg",
                        colors,
                    )),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("album-lyrics-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                                this.snap = None;
                                this.last_scroll = Some(Instant::now());
                                cx.notify();
                            }))
                            .child(
                                div().h(px(height)).w_full().flex().justify_center().child(
                                    div()
                                        .size_full()
                                        .max_w(px(1120.))
                                        .flex()
                                        .px_8()
                                        .py_8()
                                        .child(
                                            div()
                                                .id("album-artwork-region")
                                                .w(relative(0.5))
                                                .h_full()
                                                .min_w_0()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(
                                                    div()
                                                        .size(px(340.))
                                                        .flex_none()
                                                        .rounded_full()
                                                        .bg(colors.muted)
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .children(vinyl),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .id("album-lyrics-region")
                                                .w(relative(0.5))
                                                .h_full()
                                                .min_w_0(),
                                        ),
                                ),
                            )
                            .child(div().h(px(COMMENT_HEADER_HEIGHT)).w_full())
                            // 留出正文的滚动空间，后续在这里填充评论列表。
                            .child(
                                div()
                                    .id("album-comments-region")
                                    .h(px(height * 2.))
                                    .w_full()
                                    .max_w(px(1120.))
                                    .mx_auto(),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(comments_top))
                            .left_0()
                            .w_full()
                            .h(px(COMMENT_HEADER_HEIGHT))
                            .bg(colors.background)
                            .block_mouse_except_scroll()
                            .border_b_1()
                            .border_color(colors.border)
                            .flex()
                            .justify_center()
                            .child(
                                div()
                                    .size_full()
                                    .max_w(px(1120.))
                                    .px_8()
                                    .flex()
                                    .items_center()
                                    .text_color(colors.foreground)
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("评论"),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::snap_target;

    #[test]
    fn transition_snaps_at_half_and_comments_scroll_freely() {
        for height in [240., 600., 1200.] {
            for (fraction, target) in [
                (0., None),
                (0.49, Some(0.)),
                (0.5, Some(0.)),
                (0.51, Some(height)),
                (1., None),
                (1.5, None),
            ] {
                assert_eq!(snap_target(height * fraction, height), target);
            }
        }
    }
}
