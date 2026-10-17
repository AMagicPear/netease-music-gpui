//! 歌单评论页的文档头部：元数据、页签行和吸顶编辑框的占位。
//! 列表、分页和滚动条由 CommentsView 管理，吸顶编辑框由 CommentComposer 提供。

use super::*;

impl PlaylistPage {
    pub fn comments_active(&self, cx: &App) -> bool {
        self.tabs.read(cx).selected_index() == PlaylistTab::Comments as usize
    }

    pub(super) fn render_comments_header(&self, _: &mut Window, cx: &Context<Self>) -> AnyElement {
        let colors = Theme::global(cx).tokens.colors;
        div()
            .w_full()
            .px(px(40.))
            .pt(px(18.))
            .child(self.playlist_metadata(colors, cx))
            .child(self.header_controls_row(None))
            // 编辑器始终留在列表外，只让等高占位参与虚拟列表的布局。
            .child(self.composer.placeholder())
            .into_any_element()
    }
}
