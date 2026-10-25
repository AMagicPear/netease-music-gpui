//! 歌单 / 歌曲评论页共用的「吸顶评论框」。
//!
//! 编辑框始终留在虚拟列表外：列表头部条目里只放一个等高占位，真正的编辑框画在
//! 列表上层的覆盖层里——没滚过占位时贴着占位画，滚过去后吸附在视口顶。
//! 两个页面只有列表头部条目的内容不同，这块完全一致。

use std::{cell::Cell, rc::Rc};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::input::{InputEditorStyle, Textarea, TextareaState};
use gpui_kit::base::{Button, Theme};

use super::CommentsView;
use crate::ui::cover_color::Backdrop;

pub(in crate::ui) const COMMENT_LIMIT: usize = 1000;
const COMPOSER_HEIGHT: Pixels = px(84.);
const COMPOSER_SPACING: Pixels = px(19.);
/// 吸顶时和上沿留的距离，免得贴着顶。
const COMPOSER_TOP_MARGIN: Pixels = px(20.);

/// 吸附块自己占的高度：上留白 + 编辑框 + 下留白。占位与实际框同高，滚动时才不会跳。
pub(in crate::ui) fn block_height() -> Pixels {
    COMPOSER_TOP_MARGIN + COMPOSER_HEIGHT + COMPOSER_SPACING
}

/// 建一个限长、带主题样式的输入框；两个评论页共用同一套约定。
pub(in crate::ui) fn new_input(window: &mut Window, cx: &mut App) -> Entity<TextareaState> {
    cx.new(|cx| {
        let mut input = TextareaState::new(window, cx)
            .placeholder("说点什么吧")
            .rows(2)
            .scroll_beyond_last_line(Some(0));
        let colors = Theme::global(cx).tokens.colors;
        input.set_editor_style(InputEditorStyle {
            // 正文用墨色 70%（#283248B3）：比标题淡一档，长评论读起来不刺眼。
            foreground: colors.foreground.alpha(0.7),
            muted_foreground: colors.muted_foreground,
            selection: colors.selection,
            ..Default::default()
        });
        input
    })
}

/// 把超限的尾部裁掉，保留编辑历史和当前选区。放在 `InputEvent::Change` 订阅里调用。
pub(in crate::ui) fn clamp_to_limit(
    input: &mut TextareaState,
    window: &mut Window,
    cx: &mut Context<TextareaState>,
) {
    let value = input.value();
    if let Some((end, _)) = value.char_indices().nth(COMMENT_LIMIT) {
        let selection = input.selected_range();
        // 只裁掉超限尾部，保留编辑历史和当前选区。
        input.set_selected_range(end..value.len(), cx);
        input.replace("", window, cx);
        input.set_selected_range(selection.start.min(end)..selection.end.min(end), cx);
    }
}

pub(in crate::ui) struct CommentComposer {
    input: Entity<TextareaState>,
    comments: Entity<CommentsView>,
    /// 歌单页有随封面变化的渐变底，覆盖层要按自己的屏幕位置补画同一份渐变才无缝；
    /// 纯色背景的页面（歌曲评论页）传 None。
    backdrop: Option<Backdrop>,
    /// 占位的屏幕 y。覆盖层没吸顶时按它定位，布局时由占位自己量出来。
    input_y: Rc<Cell<Option<Pixels>>>,
}

impl CommentComposer {
    pub(in crate::ui) fn new(
        input: Entity<TextareaState>,
        comments: Entity<CommentsView>,
        backdrop: Option<Backdrop>,
    ) -> Self {
        Self {
            input,
            comments,
            backdrop,
            input_y: Rc::new(Cell::new(None)),
        }
    }

    /// 换资源时丢掉上一页记录的位置：新头部还没量出来前不该画到旧位置。
    pub(in crate::ui) fn reset(&self) {
        self.input_y.set(None);
    }

    /// 等高占位：放进列表头部条目参与布局，顺带记下自己的屏幕位置。
    pub(in crate::ui) fn placeholder(&self) -> impl IntoElement {
        let input_y = self.input_y.clone();
        canvas(
            move |bounds, _, _| input_y.set(Some(bounds.top())),
            |_, _, _, _| {},
        )
        .w_full()
        .h(block_height())
    }

    /// 覆盖层：绝对定位铺满页面，吸顶与否在每次布局时按列表位置现算。
    pub(in crate::ui) fn overlay(&self) -> AnyElement {
        let input = self.input.clone();
        let comments = self.comments.clone();
        let backdrop = self.backdrop.clone();
        let input_y = self.input_y.clone();
        canvas(
            move |bounds, window, cx| {
                let top = if comments
                    .read(cx)
                    .scroll_state()
                    .logical_scroll_top()
                    .item_ix
                    > 0
                {
                    bounds.top()
                } else {
                    input_y.get()?.max(bounds.top())
                };
                let mut composer = input_box(input.clone(), comments.clone(), backdrop.clone(), cx);
                composer.layout_as_root(
                    size(
                        AvailableSpace::Definite((bounds.size.width - px(80.)).max(px(0.))),
                        AvailableSpace::Definite(block_height()),
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
}

fn input_box(
    input: Entity<TextareaState>,
    comments: Entity<CommentsView>,
    backdrop: Option<Backdrop>,
    cx: &App,
) -> AnyElement {
    let colors = Theme::global(cx).tokens.colors;
    let draft = input.read(cx).value();
    let remaining = COMMENT_LIMIT.saturating_sub(input.read(cx).text().len_chars());
    // 发布状态归列表所有：它才是评论数据的主人，编辑框只负责把内容递过去。
    let publishing = comments.read(cx).publishing();
    let publish_error = comments.read(cx).publish_error().map(str::to_owned);
    // 空内容没有可发布的，也就没必要让按钮亮着。
    let can_publish = !publishing && !draft.trim().is_empty();
    // 滚轮和发布按钮都要用这两个实体，各拿一份。
    let scroll_comments = comments.clone();
    let publish_input = input.clone();
    let publish_comments = comments.clone();
    div()
        .id("comment-composer")
        .w_full()
        .h(block_height())
        .pt(COMPOSER_TOP_MARGIN)
        .pb(COMPOSER_SPACING)
        .occlude()
        // 底色 + 背景渐变两层叠出和页面完全相同的颜色：不透明，评论不会透上来；
        // 渐变按自己的屏幕位置采，滚到哪都无缝。
        .bg(colors.background)
        .when_some(backdrop, |box_, backdrop| {
            box_.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| backdrop.paint(bounds, window),
                )
                .absolute()
                .inset_0(),
            )
        })
        // Textarea 只有实际滚动文字时才停止冒泡；空框及文字边界都继续滚动页面。
        .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
            scroll_comments.update(cx, |comments, cx| {
                comments
                    .scroll_state()
                    .scroll_by(-event.delta.pixel_delta(px(20.)).y);
                cx.notify();
            });
            cx.stop_propagation();
        })
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
                        .text_size(px(14.))
                        .line_height(px(20.))
                        .text_color(colors.foreground)
                        .child(Textarea::new(&input)),
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
                            // 发布失败的原因顶掉字数提示：出错时更需要知道为什么错。
                            div()
                                .id("comment-status")
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(10.))
                                .truncate()
                                .when_some(publish_error.clone(), |status, error| {
                                    status
                                        .text_color(colors.destructive)
                                        .aria_label(error.clone())
                                        .child(error)
                                })
                                .when(publish_error.is_none(), |status| {
                                    status
                                        .text_color(colors.muted_foreground)
                                        .aria_label(format!("还可输入 {remaining} 字"))
                                        .child(remaining.to_string())
                                }),
                        )
                        .child(
                            div().flex().items_center().gap(px(10.)).child(
                                Button::new("comment-publish")
                                    .disabled(!can_publish)
                                    .aria_label("发布评论")
                                    .h(px(20.))
                                    .text_size(px(13.))
                                    .text_color(if can_publish {
                                        colors.foreground
                                    } else {
                                        colors.muted_foreground
                                    })
                                    .on_click(move |_, _, cx| {
                                        // 内容在点击这一刻才读：按钮按下时框里是什么就发什么。
                                        let content = publish_input.read(cx).value().to_string();
                                        publish_comments.update(cx, |view, cx| {
                                            view.publish(content, cx);
                                        });
                                    })
                                    .child(if publishing { "发布中" } else { "发布" }),
                            ),
                        ),
                ),
        )
        .into_any_element()
}
