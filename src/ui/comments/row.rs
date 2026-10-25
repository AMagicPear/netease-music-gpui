//! 评论列表的单条评论：头像、昵称、正文、日期与点赞/回复图标。
//! 任何展示 [`SongComment`] 列表的页面都能复用。
//!
//! 颜色全部取自调用方传入的 [`ColorTokens`]：正在播放页在深色封面上用
//! `dark_colors`，歌单页用主题原色 —— 同一行组件在两处都不再需要特判。

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{ColorTokens, TextSelectionHandle};
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;

use crate::models::SongComment;
use crate::ui::assets::thumbnail_url;
use crate::ui::components::SelectableText;
use crate::ui::theme::{DOLPHIN_FAMILY, link_blue};

/// 正文的选中身份：`handle` 由列表（[`CommentsView`](super::super::CommentsView)）按评论 id
/// 持有，`order` 是它在整个评论文档里的阅读顺序（从 0 递增）。
///
/// 顺序必须显式给，
/// 而跨参与者选区靠这个值判断谁在前谁在后；全为 0 时跨行拖选和复制顺序都退化成
/// 窗口注册顺序，评论一多就会选串。
pub type CommentSelection = (TextSelectionHandle, u64);

pub fn comment_row(
    comment: &SongComment,
    colors: ColorTokens,
    selection: CommentSelection,
) -> impl IntoElement {
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
        .border_color(colors.border)
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
                        .text_color(link_blue())
                        .text_size(px(15.))
                        .child(comment.nickname.clone()),
                )
                .child(
                    div()
                        .text_color(colors.foreground.alpha(0.9))
                        .text_size(px(15.))
                        .line_height(px(23.))
                        .mb_1()
                        // 评论正文保留换行（官方客户端也是分段显示），所以用
                        // pre-wrap 而不是默认的折叠规则。
                        .whitespace_pre_wrap()
                        // 文本命中区本来就覆盖整个正文块，这里只把指针形状同步成 I-beam，
                        // 否则鼠标停在可选文字上仍是箭头，看不出「能选」。
                        .cursor(CursorStyle::IBeam)
                        .child(body(comment, selection)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .text_size(px(12.))
                        .text_color(colors.muted_foreground)
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
                                        .text_color(colors.muted_foreground),
                                )
                                .child(
                                    svg()
                                        .path("icons/comment.svg")
                                        .size(px(17.))
                                        .ml_3()
                                        .text_color(colors.muted_foreground),
                                ),
                        ),
                ),
        )
}

/// handle 由列表持有，保持跨帧选中身份和重绘订阅。
fn body(comment: &SongComment, selection: CommentSelection) -> SelectableText {
    let id = ("comment-content", comment.id);
    // 表情在文本层就换成 Unicode 字符：选中、复制、换行都由文字系统统一处理，
    // 不需要在文本流里插图片。handle 里的文本必须与这里一致，否则选区偏移会错位。
    let text = crate::ui::emoji::expand(&comment.content).into_owned();
    let (handle, order) = selection;
    SelectableText::new(id, handle, text, order)
}
