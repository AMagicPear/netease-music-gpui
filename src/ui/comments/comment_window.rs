//! 发布评论的独立小窗。
//!
//! 正在播放页的评论屏只是那页的下半屏，没有编辑框；悬浮的「发布评论」按钮
//! 打开这个窗口：上面一个输入框，下面一个居中的红色胶囊发布按钮。
//!
//! 窗口用的是正在播放页那一份评论实体，所以发布成功后评论直接落在它的列表顶部，
//! 不需要另开一份数据再同步。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::base::{Button, ColorTokens, Theme};

use super::{COMMENT_LIMIT, CommentsView, clamp_to_limit, new_input};
use crate::desktop;
use crate::ui::components::window_drag_area;
use crate::ui::events::CommentPublished;
use crate::ui::theme::PRESSED_ICON_ALPHA;

const WINDOW_SIZE: Size<Pixels> = size(px(420.), px(268.));
const TITLE_TOP_GAP: Pixels = px(24.);
const TITLE_BOTTOM_GAP: Pixels = px(14.);
const CONTENT_PADDING: Pixels = px(20.);
/// 字数 / 失败原因那一行的高度。
const STATUS_HEIGHT: Pixels = px(16.);
const PUBLISH_HEIGHT: Pixels = px(34.);

/// 打开发布评论的小窗。同一时刻只允许存在一个：重复点击只是把它叫到前面。
///
/// `parent` 用来定位新窗口并确定它落在哪块显示器上。
pub(in crate::ui) fn open_comment_window(
    parent: &Window,
    comments: Entity<CommentsView>,
    cx: &mut App,
) {
    if let Some(handle) = desktop::comment_window(cx) {
        // 已经开着了：叫到前面就行。窗口里的评论实体是同一个，内容不用换。
        let alive = handle
            .update(cx, |_, window, cx| {
                cx.activate(true);
                window.activate_window();
            })
            .is_ok();
        if alive {
            return;
        }
        // 句柄指向的窗口已经没了（主窗口关过一轮），清掉它重新开。
        desktop::set_comment_window(None, cx);
    }
    let parent_bounds = parent.window_bounds().get_bounds();
    let bounds = Bounds::centered_at(parent_bounds.center(), WINDOW_SIZE);
    // 多显示器下必须同时传入父窗口的显示器，避免落到主屏。
    let display_id = parent.display(cx).map(|display| display.id());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        titlebar: Some(TitlebarOptions {
            appears_transparent: true,
            ..Default::default()
        }),
        app_owns_titlebar_drag: true,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, move |window, cx| {
        #[cfg(target_os = "macos")]
        crate::ui::components::hide_traffic_lights(window, true);
        window.on_window_should_close(cx, |_, cx| {
            desktop::set_comment_window(None, cx);
            true
        });
        cx.new(|cx| CommentWindow::new(comments, window, cx))
    }) {
        Ok((window, _)) => desktop::set_comment_window(Some(window), cx),
        Err(error) => log::warn!("打开发布评论窗口失败：{error}"),
    }
}

struct CommentWindow {
    comments: Entity<CommentsView>,
    input: Entity<TextareaState>,
    /// 三个订阅都挂在窗口实体上（gpui 的 Subscription 一丢就退订）：
    /// 输入变化要限长、发布成功要关窗、评论状态变化要重绘。
    _subscriptions: [Subscription; 3],
}

impl CommentWindow {
    fn new(comments: Entity<CommentsView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = new_input(window, cx);
        // 焦点只在这里给一次：窗口一出现输入框就接着打字，之后谁点谁拿走。
        // 焦点是按窗口记的，主窗口那份从头到尾没被动过，所以关窗时也不会
        // 有「焦点跳回去」的动作——那要等主窗口自己重新拿到系统焦点。
        input.update(cx, |input, cx| input.focus(window, cx));
        let edited = comments.clone();
        let input_subscription = cx.subscribe_in(
            &input,
            window,
            move |_, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    input.update(cx, |input, cx| clamp_to_limit(input, window, cx));
                    // 用户又开始改了，上一次的失败提示就不再相关。
                    edited.update(cx, |view, cx| view.clear_publish_error(cx));
                }
                cx.notify();
            },
        );
        // 发布成功：评论已经进了列表，这个窗口的任务完成，关掉。
        let publish_subscription = cx.subscribe_in(
            &comments,
            window,
            |_, _, _: &CommentPublished, window, cx| {
                close(window, cx);
            },
        );
        // 发布中 / 失败原因都在评论实体上，它变了窗口要跟着重绘。
        let comments_subscription = cx.observe(&comments, |_, _, cx| cx.notify());
        Self {
            comments,
            input,
            _subscriptions: [
                input_subscription,
                publish_subscription,
                comments_subscription,
            ],
        }
    }
}

/// 关窗。延后一拍，避免在事件广播链里直接拆窗口。
fn close(window: &mut Window, cx: &mut Context<CommentWindow>) {
    cx.spawn_in(window, async move |_, cx| {
        let _ = cx.update(|window, cx| {
            desktop::set_comment_window(None, cx);
            window.remove_window();
        });
    })
    .detach();
}

/// 没有红绿灯的平台（Windows / Linux）自己画一个关闭按钮。
fn close_button(colors: ColorTokens) -> impl IntoElement {
    svg()
        .path("icons/window/close.svg")
        .size(px(20.))
        .absolute()
        .top(px(24.))
        .right(px(20.))
        .flex_none()
        .cursor_pointer()
        .text_color(colors.foreground.alpha(0.6))
        .hover(|style| style.text_color(colors.foreground))
        .id("comment-window-close")
        .role(Role::Button)
        .aria_label("关闭")
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
        .on_click(|_, window, cx| {
            desktop::set_comment_window(None, cx);
            window.remove_window();
        })
}

impl Render for CommentWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let draft = self.input.read(cx).value();
        let remaining = COMMENT_LIMIT.saturating_sub(self.input.read(cx).text().len_chars());
        let publishing = self.comments.read(cx).publishing(cx);
        let error = self.comments.read(cx).publish_error(cx);
        let can_publish = !publishing && !draft.trim().is_empty();
        let input = self.input.clone();
        let comments = self.comments.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.foreground)
            .overflow_hidden()
            // 标题栏：可拖拽，标题居中。
            .child(
                window_drag_area("comment-window-titlebar")
                    .relative()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pt(TITLE_TOP_GAP)
                    .pb(TITLE_BOTTOM_GAP)
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("发布评论"),
                    )
                    // macOS 有红绿灯，别的平台得自己给一个关闭按钮。
                    .when(!cfg!(target_os = "macos"), |bar| {
                        bar.child(close_button(colors))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .px(CONTENT_PADDING)
                    .pb(CONTENT_PADDING)
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(colors.border)
                            // 沿用评论页编辑框的底色配方：几乎透明，只留一道细边。
                            .bg(colors.foreground.alpha(0.04))
                            .px(px(10.))
                            .py(px(8.))
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .text_color(colors.foreground)
                            .child(Textarea::new(&self.input)),
                    )
                    // 状态行：出错时先说为什么错，否则说还能打多少字。
                    .child(
                        div()
                            .id("comment-window-status")
                            .flex_none()
                            .h(STATUS_HEIGHT)
                            .flex()
                            .items_center()
                            .text_size(px(11.))
                            .truncate()
                            .when_some(error.clone(), |status, error| {
                                status
                                    .text_color(colors.destructive)
                                    .aria_label(error.clone())
                                    .child(error)
                            })
                            .when(error.is_none(), |status| {
                                status
                                    .text_color(colors.muted_foreground)
                                    .aria_label(format!("还可输入 {remaining} 字"))
                                    .child(remaining.to_string())
                            }),
                    )
                    // 底部：居中的红色胶囊发布按钮。
                    .child(
                        div().flex_none().flex().justify_center().child(
                            Button::new("comment-window-publish")
                                .h(PUBLISH_HEIGHT)
                                .px(px(34.))
                                .rounded_full()
                                .bg(if can_publish {
                                    colors.primary
                                } else {
                                    colors.primary.alpha(0.35)
                                })
                                .text_color(colors.primary_foreground)
                                .text_size(px(14.))
                                .disabled(!can_publish)
                                .on_click(move |_, _, cx| {
                                    let content = input.read(cx).value().to_string();
                                    comments.update(cx, |view, cx| view.publish(content, cx));
                                })
                                .child(if publishing { "发布中" } else { "发布" }),
                        ),
                    ),
            )
    }
}
