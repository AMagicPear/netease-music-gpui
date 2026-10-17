//! 歌曲评论页：播放栏、歌曲行或右键菜单的评论图标进来。
//!
//! 结构和歌单评论页一致——页头条目 + 虚拟评论列表 + 吸顶编辑框（CommentComposer），
//! 只有页头内容换成歌曲信息；热评跟着歌曲接口返回，列表顶部会多出「精选评论」小节。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::Theme;
use gpui_kit::base::input::{InputEvent, TextareaState};

use crate::models::Song;
use crate::ui::assets::thumbnail_url;
use crate::ui::components::{
    CommentComposer, CommentResource, CommentsView, ListMode, clamp_to_limit, new_input,
};

/// 歌单 / 歌曲评论共享一个 View；歌曲数据直接存在页面上（只有一首，不用独立 Entity）。
pub(crate) struct SongCommentsPage {
    song: Option<Song>,
    comments: Entity<CommentsView>,
    comment_input: Entity<TextareaState>,
    _comment_input_subscription: Subscription,
    composer: CommentComposer,
    /// 列表滚动 / 翻页时覆盖层要跟着重新定位；gpui 不会自动通知父 view，显式接一条。
    _comments_subscription: Subscription,
}

impl SongCommentsPage {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let comments = cx.new(|_| CommentsView::new(None));
        // 头部条目渲染时才回头取当前歌曲； weak 引用避免页面和列表互相持有。
        let page = cx.weak_entity();
        comments.update(cx, |comments, cx| {
            comments.set_header(
                move |window, cx| {
                    page.update(cx, |page, cx| page.render_header(window, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                },
                cx,
            );
        });
        let comment_input = new_input(window, cx);
        let comment_input_subscription = cx.subscribe_in(
            &comment_input,
            window,
            |_, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    input.update(cx, |input, cx| clamp_to_limit(input, window, cx));
                }
                cx.notify();
            },
        );
        let composer = CommentComposer::new(comment_input.clone(), comments.clone(), None);
        let comments_subscription = cx.observe(&comments, |_, _, cx| cx.notify());
        Self {
            song: None,
            comments,
            comment_input,
            _comment_input_subscription: comment_input_subscription,
            composer,
            _comments_subscription: comments_subscription,
        }
    }

    /// 打开一首歌的评论；同一首歌重复打开不重载，评论和滚动位置都保留。
    pub(crate) fn open(&mut self, song: Song, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .song
            .as_ref()
            .is_some_and(|current| current.id == song.id)
        {
            return;
        }
        self.song = Some(song.clone());
        self.comments.update(cx, |comments, cx| {
            comments.set_resource(Some(CommentResource::Song(song.id)), cx);
            comments.configure(ListMode::Owned, cx);
        });
        // 换歌即换语境：上一首的草稿不该落到新歌的发布框里；
        // 占位位置记录一起丢掉，新头部量出来前不画到旧位置。
        self.comment_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.composer.reset();
        cx.notify();
    }

    /// 页头条目：封面 + 歌名 + 专辑 / 歌手，和官方客户端的评论页对齐。
    fn render_header(&self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = Theme::global(cx).tokens.colors;
        let Some(song) = self.song.as_ref() else {
            return div().into_any_element();
        };
        let artists = song
            .ar
            .iter()
            .filter_map(|artist| artist.name.clone())
            .collect::<Vec<_>>()
            .join(" / ");
        div()
            .w_full()
            .px(px(40.))
            .pt(px(18.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        div()
                            .size(px(64.))
                            .flex_none()
                            .overflow_hidden()
                            .rounded(px(8.))
                            .when_some(song.al.pic_url.as_deref(), |cover, url| {
                                cover.child(
                                    // `overflow_hidden` 只按矩形裁剪，圆角要直接设在图片上。
                                    img(thumbnail_url(url, 128))
                                        .size_full()
                                        .rounded(px(8.))
                                        .object_fit(ObjectFit::Cover),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(colors.foreground)
                                    .truncate()
                                    .child(song.name.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(24.))
                                    .min_w(px(0.))
                                    .text_size(px(12.))
                                    .text_color(colors.muted_foreground)
                                    .children(song.al.name.clone().map(|album| {
                                        div()
                                            .min_w(px(0.))
                                            .truncate()
                                            .child(format!("专辑：{album}"))
                                    }))
                                    .child(div().min_w(px(0.)).truncate().child(format!(
                                        "歌手：{}",
                                        if artists.is_empty() {
                                            "未知歌手".to_string()
                                        } else {
                                            artists
                                        }
                                    ))),
                            ),
                    ),
            )
            .child(self.composer.placeholder())
            .into_any_element()
    }
}

impl Render for SongCommentsPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .child(self.comments.clone())
            .child(self.composer.overlay())
    }
}
