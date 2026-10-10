use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, Scrollbar, ScrollbarMode, Theme};

use crate::api::MusicApi;
use crate::models::SongComment;
use crate::ui::components::{comment_row, spinner};
use crate::ui::cover_color::{Backdrop, dark_colors, dark_gradient};

pub(super) struct ScrollBack;

pub(super) struct CommentsView {
    backdrop: Backdrop,
    outer: ScrollHandle,
    scroll: ScrollHandle,
    song_id: Option<u64>,
    generation: u64,
    comments: Vec<SongComment>,
    total: u64,
    offset: usize,
    more: bool,
    loading: bool,
    error: Option<String>,
    request: Option<tokio::task::AbortHandle>,
    enabled: bool,
    show_comments: bool,
    collapsed: bool,
    layout_pending: bool,
}

impl EventEmitter<ScrollBack> for CommentsView {}

impl CommentsView {
    pub(super) fn new(outer: ScrollHandle, backdrop: Backdrop) -> Self {
        Self {
            backdrop,
            outer,
            scroll: ScrollHandle::default(),
            song_id: None,
            generation: 0,
            comments: Vec::new(),
            total: 0,
            offset: 0,
            more: false,
            loading: false,
            error: None,
            request: None,
            enabled: false,
            show_comments: false,
            collapsed: false,
            layout_pending: false,
        }
    }

    pub(super) fn set_song(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        if self.song_id == id {
            return;
        }
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.song_id = id;
        self.generation = self.generation.wrapping_add(1);
        self.comments.clear();
        self.total = 0;
        self.offset = 0;
        self.more = false;
        self.loading = false;
        self.error = None;
        self.collapsed = false;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.layout_pending = true;
        if id.is_some() {
            self.load(cx);
        }
        cx.notify();
    }

    pub(super) fn configure(&mut self, enabled: bool, show_comments: bool, cx: &mut Context<Self>) {
        if (self.enabled, self.show_comments) == (enabled, show_comments) {
            return;
        }
        self.enabled = enabled;
        self.show_comments = show_comments;
        self.layout_pending = true;
        cx.notify();
    }

    pub(super) fn set_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.collapsed == collapsed {
            return;
        }
        self.collapsed = collapsed;
        self.layout_pending = true;
        cx.notify();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(id) = self.song_id else { return };
        let offset = self.offset;
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::song_comments(api.client.clone(), id, offset));
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request.await.unwrap_or_else(|_| Err("评论加载失败".into()));
            let _ = this.update(cx, |this, cx| {
                // 被 abort 的请求也可能已有主线程回调，必须同时检查歌曲与代次。
                if this.song_id != Some(id) || this.generation != generation {
                    return;
                }
                this.loading = false;
                this.request = None;
                match result {
                    Ok(page) => {
                        this.total = page.total;
                        this.more = page.more;
                        this.offset = offset + 20;
                        let before = this.comments.len();
                        for comment in page.comments {
                            if !this
                                .comments
                                .iter()
                                .any(|existing| existing.id == comment.id)
                            {
                                this.comments.push(comment);
                            }
                        }
                        if this.comments.len() == before {
                            this.more = false;
                        }
                        this.layout_pending = true;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_near_bottom(&mut self, cx: &mut Context<Self>) {
        if self.layout_pending || self.collapsed || self.scroll.bounds().size.height <= px(0.) {
            return;
        }
        if can_load_more_comments(
            self.enabled,
            self.show_comments,
            self.more,
            self.loading,
            self.error.is_some(),
        ) && self.scroll.max_offset().y + self.scroll.offset().y < px(120.)
        {
            self.load(cx);
        }
    }
}

impl Render for CommentsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.layout_pending && self.enabled && self.show_comments && !self.collapsed {
            // 新页内容先经 prepaint 更新 bounds，再判断是否需要填满视口。
            cx.on_next_frame(window, |this, _, cx| {
                this.layout_pending = false;
                cx.notify();
            });
        } else {
            self.load_near_bottom(cx);
        }
        let colors = dark_colors(
            Theme::global(cx).tokens.colors,
            self.backdrop
                .gradient()
                .unwrap_or_else(|| dark_gradient(None))
                .0[1]
                .into(),
        );
        let scrollable = self.enabled && self.show_comments && !self.collapsed;
        let mut list = div();
        if !self.collapsed {
            list =
                list.children(self.comments.iter().map(comment_row))
                    .when(self.loading, |list| {
                        list.child(div().py_5().flex().justify_center().child(spinner(
                            "album-comments-loading",
                            20.,
                            colors.muted_foreground,
                        )))
                    });
            if self.error.is_some() {
                list = list.child(
                    Button::new("retry-song-comments")
                        .child("评论加载失败，重试")
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                );
            } else if !self.loading && self.comments.is_empty() {
                list = list.child(
                    div()
                        .py_8()
                        .text_color(colors.muted_foreground)
                        .child("还没有评论"),
                );
            }
        }
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id("album-comments-region")
                    .debug_selector(|| "album-comments-region".into())
                    .size_full()
                    .overflow_hidden()
                    .when(scrollable, |comments| comments.overflow_y_scroll())
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                        if this.enabled && this.show_comments && !this.collapsed {
                            // 原生滚动先更新内层，越过顶部的剩余位移才交回外层。
                            let remainder = this.scroll.offset().y.max(px(0.));
                            if remainder > px(0.) {
                                this.scroll.set_offset(point(px(0.), px(0.)));
                                this.outer
                                    .set_offset(this.outer.offset() + point(px(0.), remainder));
                                cx.emit(ScrollBack);
                            }
                            cx.stop_propagation();
                        }
                    }))
                    .px(px(80.))
                    .pt(px(24.))
                    .pb(px(100.))
                    .child(
                        div()
                            .debug_selector(|| "album-comments-heading".into())
                            .flex()
                            .items_center()
                            .gap_1()
                            .mb_4()
                            .text_color(white())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(18.))
                            .child("全部评论")
                            .child(div().text_size(px(12.)).child(self.total.to_string())),
                    )
                    .child(list),
            )
            .when(scrollable, |comments| {
                comments.child(
                    // 滚动条必须在滚动容器外，避免自身高度计入内容。
                    Scrollbar::vertical(&self.scroll)
                        .mode(ScrollbarMode::Scrolling)
                        .styles(|styles| {
                            styles
                                .thumb(|thumb| thumb.bg(white().alpha(0.25)))
                                .thumb_hover(|thumb| thumb.bg(white().alpha(0.35)))
                        }),
                )
            })
    }
}

fn can_load_more_comments(
    enabled: bool,
    visible: bool,
    more: bool,
    loading: bool,
    failed: bool,
) -> bool {
    enabled && visible && more && !loading && !failed
}
