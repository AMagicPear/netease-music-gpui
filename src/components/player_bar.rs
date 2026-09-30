use gpui::*;

use super::progress_bar::ProgressBar;
use crate::theme::Theme;

#[derive(IntoElement)]
pub struct PlayerBar {
    pub theme: Theme,
    pub progress_bar: Entity<ProgressBar>,
}

impl RenderOnce for PlayerBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let play_gradient = linear_gradient(
            270.,
            linear_color_stop(theme.secondary1_2, 0.),
            linear_color_stop(theme.secondary1_1, 1.),
        );

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
            .child(self.progress_bar)
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
                                            .child("Run Away With Me"),
                                    )
                                    .child(
                                        div()
                                            .text_color(theme.black5)
                                            .text_size(px(13.))
                                            .child("Carly Rae Jepsen"),
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
                                    .size(px(46.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(play_gradient)
                                    .text_color(theme.white1)
                                    .child(
                                        svg()
                                            .path("icons/play.svg")
                                            .size(px(20.))
                                            .text_color(theme.white1),
                                    ),
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
