//! 歌曲评论页：播放栏、歌曲行或右键菜单的评论图标进来。
//!
//! 页面本身只有两件事——记住当前是哪首歌、渲染自己的页头条目；列表分页与吸顶编辑框
//! 都在 [`CommentsPanel`] 里，和歌单评论页共用同一套 wiring。热评跟着歌曲接口返回，
//! 列表顶部会多出「精选评论」小节。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::Theme;

use super::{CommentResource, CommentsPanel};
use crate::models::Song;
use crate::ui::assets::thumbnail_url;

/// 「查看评论」：带着歌来到 [SongCommentsPage]。
/// 歌曲原生菜单派发它；界面上的评论图标经导航总线发 `Navigate::SongComments` 意图。
/// 因为它不会从键位 JSON 反序列化，声明 `no_json` 以省去 `Deserialize` / `JsonSchema`。
#[derive(Action, Clone, PartialEq)]
#[action(namespace = comments, no_json)]
pub(in crate::ui) struct ViewSongComments(pub Song);

/// 所有歌曲共享一个 View；歌曲数据直接存在页面上（只有一首，不用独立 Entity）。
pub(in crate::ui) struct SongCommentsPage {
    song: Option<Song>,
    comments: CommentsPanel,
}

impl SongCommentsPage {
    pub(in crate::ui) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 头部条目渲染时才回头取当前歌曲； weak 引用避免页面和列表互相持有。
        let page = cx.weak_entity();
        let comments = CommentsPanel::new(window, cx, None, move |window, cx| {
            page.update(cx, |page, cx| page.render_header(window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        });
        Self {
            song: None,
            comments,
        }
    }

    /// 打开一首歌的评论；同一首歌重复打开不重载，评论和滚动位置都保留。
    pub(in crate::ui) fn open(&mut self, song: Song, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .song
            .as_ref()
            .is_some_and(|current| current.id == song.id)
        {
            return;
        }
        self.song = Some(song.clone());
        self.comments
            .open_resource(CommentResource::Song(song.id), None, cx);
        self.comments.clear_draft(window, cx);
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
            // 编辑框留在列表外编码 DX：只让等高占位参与虚拟列表的布局。
            .child(self.comments.placeholder())
            .into_any_element()
    }
}

impl Render for SongCommentsPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.comments.render()
    }
}
