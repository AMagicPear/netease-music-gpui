//! 歌单评论的文档头部与吸顶编辑框；列表、分页和滚动条由 CommentsView 管理。

use super::*;
use gpui_kit::base::input::Textarea;

pub(super) const COMMENT_LIMIT: usize = 1000;
const COMPOSER_HEIGHT: Pixels = px(84.);
const COMPOSER_SPACING: Pixels = px(19.);
/// 吸顶时和上沿留的距离，免得贴着顶。
const COMPOSER_TOP_MARGIN: Pixels = px(20.);
/// 吸附条自己占的高度：上留白 + 编辑框 + 下留白。背景要盖满整块。
fn composer_block_height() -> Pixels {
    COMPOSER_TOP_MARGIN + COMPOSER_HEIGHT + COMPOSER_SPACING
}

impl PlaylistPage {
    pub fn comments_active(&self, cx: &App) -> bool {
        self.tabs.read(cx).selected_index() == PlaylistTab::Comments as usize
    }

    pub(super) fn render_comments_header(&self, _: &mut Window, cx: &Context<Self>) -> AnyElement {
        let colors = Theme::global(cx).tokens.colors;
        let input_y = self.comment_input_y.clone();
        div()
            .w_full()
            .px(px(40.))
            .pt(px(18.))
            .child(self.playlist_metadata(colors, cx))
            .child(self.header_controls_row(None))
            .child(
                // 编辑器始终留在列表外，只让等高占位参与虚拟列表的布局。
                canvas(
                    move |bounds, _, _| input_y.set(Some(bounds.top())),
                    |_, _, _, _| {},
                )
                .w_full()
                .h(composer_block_height()),
            )
            .into_any_element()
    }

    pub(super) fn comment_input_overlay(&self, cx: &Context<Self>) -> AnyElement {
        let page = cx.weak_entity();
        let input_y = self.comment_input_y.clone();
        let list = self.comments.read(cx).scroll_state();
        canvas(
            move |bounds, window, cx| {
                let top = if list.logical_scroll_top().item_ix > 0 {
                    bounds.top()
                } else {
                    input_y.get()?.max(bounds.top())
                };
                let mut composer = page
                    .update(cx, |page, cx| page.comment_input_box(cx))
                    .ok()?;
                composer.layout_as_root(
                    size(
                        AvailableSpace::Definite((bounds.size.width - px(80.)).max(px(0.))),
                        AvailableSpace::Definite(composer_block_height()),
                    ),
                    window,
                    cx,
                );
                composer.prepaint_at(point(bounds.left() + px(40.), top), window, cx);
                Some(composer)
            },
            |_, composer, window, cx| {
                if let Some(mut composer) = composer {
                    composer.paint(window, cx);
                }
            },
        )
        // Canvas 自身没有 hitbox；只有实际编辑框这一条区域遮挡下面的评论。
        .absolute()
        .inset_0()
        .into_any_element()
    }

    fn comment_input_box(&self, cx: &Context<Self>) -> AnyElement {
        let colors = Theme::global(cx).tokens.colors;
        let remaining =
            COMMENT_LIMIT.saturating_sub(self.comment_input.read(cx).text().len_chars());
        div()
            .id("playlist-comment-composer")
            .w_full()
            .h(composer_block_height())
            .pt(COMPOSER_TOP_MARGIN)
            .pb(COMPOSER_SPACING)
            .occlude()
            // 底色 + 背景渐变两层叠出和页面完全相同的颜色：不透明，评论不会透上来；
            // 渐变按自己的屏幕位置采，滚到哪都无缝。
            .bg(colors.background)
            .child({
                let backdrop = self.backdrop.clone();
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| backdrop.paint(bounds, window),
                )
                .absolute()
                .inset_0()
            })
            // Textarea 只有实际滚动文字时才停止冒泡；空框及文字边界都继续滚动页面。
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                this.comments.update(cx, |comments, cx| {
                    comments
                        .scroll_state()
                        .scroll_by(-event.delta.pixel_delta(px(20.)).y);
                    cx.notify();
                });
                cx.notify();
                cx.stop_propagation();
            }))
            .child(
                div()
                    .w_full()
                    .h(COMPOSER_HEIGHT)
                    .px(px(16.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    // 沿用全屏胶囊的 0.04 / 0.10 配方，颜色换成本页主题的前景色：
                    // 几乎透明，只在浅色背景上留下一点底色和一道细边。
                    .border_color(colors.foreground.alpha(0.10))
                    .bg(colors.foreground.alpha(0.04))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .w_full()
                            .flex_1()
                            .min_h(px(0.))
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(colors.foreground)
                            .child(Textarea::new(&self.comment_input)),
                    )
                    .child(
                        div()
                            .h(px(20.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_color(colors.muted_foreground)
                            .child(
                                div()
                                    .id("playlist-comment-remaining")
                                    .text_size(px(10.))
                                    .aria_label(format!("还可输入 {remaining} 字"))
                                    .child(remaining.to_string()),
                            )
                            .child(
                                div().flex().items_center().gap(px(10.)).child(
                                    Button::new("playlist-publish-comment")
                                        .disabled(true)
                                        .aria_label("发布评论（暂未支持）")
                                        .h(px(20.))
                                        .text_size(px(13.))
                                        .text_color(colors.muted_foreground)
                                        .child("发布"),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
