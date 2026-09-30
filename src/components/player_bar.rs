use gpui::*;

use super::progress_bar::ProgressBar;
use crate::state::playback::PlaybackState;
use crate::theme::Theme;

pub struct PlayerBar {
    theme: Theme,
    playback: Entity<PlaybackState>,
    progress_bar: Entity<ProgressBar>,
    _playback_subscription: Subscription,
}

impl PlayerBar {
    pub fn new(theme: Theme, playback: Entity<PlaybackState>, cx: &mut Context<Self>) -> Self {
        let progress_bar = cx.new(|cx| ProgressBar::new(theme, playback.clone(), cx));
        let playback_subscription = cx.observe(&playback, |_, _, cx| cx.notify());
        Self {
            theme,
            playback,
            progress_bar,
            _playback_subscription: playback_subscription,
        }
    }
}

impl Render for PlayerBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let (title, artist, is_playing) = {
            let playback = self.playback.read(cx);
            playback
                .current_song
                .as_ref()
                .map(|song| (song.title.clone(), song.artist.clone(), playback.is_playing))
                .unwrap_or_else(|| (String::new(), String::new(), playback.is_playing))
        };
        let play_gradient = linear_gradient(
            270.,
            linear_color_stop(theme.secondary1_2, 0.),
            linear_color_stop(theme.secondary1_1, 1.),
        );
        let play_pause_icon = if is_playing {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().w(px(4.)).h(px(16.)).rounded_sm().bg(theme.white1))
                .child(div().w(px(4.)).h(px(16.)).rounded_sm().bg(theme.white1))
                .into_any_element()
        } else {
            svg()
                .path("icons/play.svg")
                .size(px(20.))
                .text_color(theme.white1)
                .into_any_element()
        };

        div()
            .w_full()
            .h(px(86.))
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.player_bar_background)
            .border_t_1()
            .border_color(theme.black10)
            .relative()
            .child(self.progress_bar.clone())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(28.))
                    // 左侧：封面、歌曲信息和互动数据
                    .child(
                        div()
                            .w(px(325.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .child(
                                div()
                                    .size(px(54.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(theme.black1)
                                    .text_color(theme.white1)
                                    .text_size(px(23.))
                                    .child("♫"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .text_color(theme.black1)
                                            .text_size(px(15.))
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_color(theme.black5)
                                            .text_size(px(13.))
                                            .child(artist),
                                    ),
                            )
                            .child(
                                div()
                                    .ml(px(8.))
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .text_color(theme.secondary1_1)
                                    .text_size(px(11.))
                                    .child(
                                        svg()
                                            .path("icons/like.svg")
                                            .size(px(20.))
                                            .text_color(theme.secondary1_1),
                                    )
                                    .child("10w+"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .text_color(theme.black5)
                                    .text_size(px(11.))
                                    .child(
                                        svg()
                                            .path("icons/comment.svg")
                                            .size(px(20.))
                                            .text_color(theme.black5),
                                    )
                                    .child("999+"),
                            ),
                    )
                    // 中间：收藏与播放控制
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(27.))
                            .text_color(theme.black1)
                            .child(
                                svg()
                                    .path("icons/collect.svg")
                                    .size(px(21.))
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/pre.svg")
                                    .size(px(22.))
                                    .text_color(theme.black3),
                            )
                            .child(
                                div()
                                    .id("play-pause-button")
                                    .size(px(46.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(play_gradient)
                                    .text_color(theme.white1)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.playback.update(cx, |playback, cx| {
                                            playback.is_playing = !playback.is_playing;
                                            cx.notify();
                                        });
                                    }))
                                    .child(play_pause_icon),
                            )
                            .child(
                                svg()
                                    .path("icons/next.svg")
                                    .size(px(22.))
                                    .text_color(theme.black3),
                            )
                            .child(
                                svg()
                                    .path("icons/playlist.svg")
                                    .size(px(22.))
                                    .text_color(theme.black5),
                            ),
                    )
                    // 右侧：音质、设备、音量等工具
                    .child(
                        div()
                            .w(px(205.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(18.))
                            .text_color(theme.black5)
                            .child(
                                svg()
                                    .path("icons/sq.svg")
                                    .size(px(34.))
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/sidebar_add.svg")
                                    .size(px(22.))
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/xpoint.svg")
                                    .size(px(22.))
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/volume.svg")
                                    .size(px(22.))
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/morefunctions.svg")
                                    .size(px(22.))
                                    .text_color(theme.black5),
                            ),
                    ),
            )
    }
}
