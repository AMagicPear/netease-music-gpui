//! 评论列表的单条评论：头像、昵称、正文、日期与点赞/回复图标。
//! 任何展示 [`SongComment`] 列表的页面都能复用。

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::SelectableText;
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;

use crate::models::SongComment;
use crate::ui::assets::thumbnail_url;
use crate::ui::theme::DOLPHIN_FAMILY;

pub fn comment_row(comment: &SongComment) -> impl IntoElement {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let current_year = now.year();
    let date = time::OffsetDateTime::from_unix_timestamp(comment.time / 1000)
        .ok()
        .map(|date| date.to_offset(now.offset()))
        .map(|date| {
            if date.year() == current_year {
                format!("{:02}-{:02}", u8::from(date.month()), date.day())
            } else {
                format!(
                    "{}-{:02}-{:02}",
                    date.year(),
                    u8::from(date.month()),
                    date.day()
                )
            }
        })
        .unwrap_or_default();
    div()
        .w_full()
        .flex()
        .gap(px(12.))
        .pt(px(19.))
        .pb(px(17.))
        .border_b_1()
        .border_color(white().alpha(0.08))
        .child(
            Avatar::new()
                .with_size(px(40.))
                .border_0()
                .src(thumbnail_url(&comment.avatar_url, 80)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .mt(px(-3.))
                        .text_color(rgb(0x5975B2))
                        .text_size(px(15.))
                        .child(comment.nickname.clone()),
                )
                .child(
                    div()
                        .text_color(white().alpha(0.9))
                        .text_size(px(15.))
                        .line_height(px(23.))
                        .mb_1()
                        .child(SelectableText::new(
                            ("comment-content", comment.id),
                            comment.content.clone(),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .text_size(px(12.))
                        .text_color(white().alpha(0.45))
                        .child(div().font_family(DOLPHIN_FAMILY).child(date))
                        .child(
                            div()
                                .ml_auto()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .font_family(DOLPHIN_FAMILY)
                                        .child(comment.liked_count.to_string()),
                                )
                                .child(
                                    svg()
                                        .path("icons/点赞.svg")
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
