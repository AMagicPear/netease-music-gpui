use gpui::*;

use super::progress_bar::ProgressBar;
use crate::state::playback::PlaybackState;
use crate::theme::{IconSize, Theme};

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

fn interaction_count(
    icon_path: &'static str,
    count: &'static str,
    color: Rgba,
    theme: Theme,
) -> impl IntoElement {
    div()
        .ml_0p5()
        .w(px(28.))
        .h(px(24.))
        .flex_none()
        .child(
            svg()
                .path(icon_path)
                .absolute()
                .left_0()
                .bottom_0()
                .size(IconSize::Large.pixels())
                .text_color(color),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(16.))
                .px(px(2.))
                .rounded_full()
                .bg(theme.player_bar_background)
                .text_color(color)
                .text_size(px(8.))
                .font_family("Trebuchet MS")
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(px(10.))
                .child(count),
        )
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
        let play_pause_icon = if is_playing {
            svg()
                .path("icons/pause.svg")
                .size(IconSize::Large.pixels())
                .text_color(theme.white1)
                .into_any_element()
        } else {
            svg()
                .path("icons/play.svg")
                .size(IconSize::Large.pixels())
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
            // 进度条覆盖渲染
            .child(deferred(self.progress_bar.clone()).with_priority(1))
            // 主控件栏
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .px(px(30.))
                    // 左侧：封面、歌曲信息和互动数据
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            // 旋转黑胶封面
                            .child(img("images/miniVinyl.png").size(px(60.)).flex_none())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .max_w(px(200.))
                                    .flex_shrink_1()
                                    .overflow_hidden()
                                    // 歌曲标题
                                    .child(
                                        div()
                                            .text_color(theme.black1)
                                            .text_size(px(16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(title),
                                    )
                                    // 歌手/制作人
                                    .child(
                                        div()
                                            .text_color(theme.black5)
                                            .text_size(px(13.))
                                            .truncate()
                                            .child(artist),
                                    ),
                            )
                            .child(interaction_count(
                                "icons/like.svg",
                                "10w+",
                                theme.primary,
                                theme,
                            ))
                            .child(interaction_count(
                                "icons/comment.svg",
                                "999+",
                                theme.black5,
                                theme,
                            )),
                    )
                    // 中间：收藏与播放控制
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(20.))
                            .text_color(theme.black1)
                            .child(
                                svg()
                                    .path("icons/播放顺序/顺序.svg")
                                    .size(IconSize::Large.pixels())
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/pre.svg")
                                    .size(IconSize::Large.pixels())
                                    .text_color(theme.black3),
                            )
                            .child(
                                div()
                                    .id("play-pause-button")
                                    .size(px(40.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(theme.primary)
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
                                    .size(IconSize::Large.pixels())
                                    .text_color(theme.black3),
                            )
                            .child(
                                svg()
                                    .path("icons/playlist.svg")
                                    .size(IconSize::Large.pixels())
                                    .text_color(theme.black5),
                            ),
                    )
                    // 右侧：收藏、音量等工具
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(18.))
                            .text_color(theme.black5)
                            .child(
                                svg()
                                    .path("icons/sq.svg")
                                    .size(IconSize::Middle.pixels())
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/collect.svg")
                                    .size(IconSize::Middle.pixels())
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/volume.svg")
                                    .size(IconSize::Middle.pixels())
                                    .text_color(theme.black5),
                            )
                            .child(
                                svg()
                                    .path("icons/xpoint.svg")
                                    .size(IconSize::Middle.pixels())
                                    .text_color(theme.black5),
                            ),
                    ),
            )
    }
}
