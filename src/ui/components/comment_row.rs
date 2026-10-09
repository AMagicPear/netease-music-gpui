//! 评论列表的单条评论：头像、昵称、正文、日期与点赞/回复图标。
//! 任何展示 [`SongComment`] 列表的页面都能复用。

use gpui::prelude::*;
use gpui::*;
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;

use crate::models::SongComment;
use crate::ui::assets::thumbnail_url;

pub fn comment_row(comment: &SongComment) -> impl IntoElement {
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
