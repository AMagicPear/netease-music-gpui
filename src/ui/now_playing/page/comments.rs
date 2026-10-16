//! 歌曲评论页：`gpui::list` 虚拟化渲染 + 自己的滚轮协调。
//!
//! 评论行高随正文换行变化，`gpui::list` 在布局时实测行高、只构建视口附近的
//! 条目（gpui-kit 的 `VirtualList` 需要预先给出每行精确高度，`component::list`
//! 要求等高行，都不适合这种「正文任意换行」的列表）。
//!
//! 这一屏嵌在外层滚动容器（`super`）的第二页里，两层都要吃滚轮。规矩只有一条：
//! **外层翻到评论区（整屏到位）之前，一个像素都不给评论列表**。判定由外层算好
//! 传进来（[`ListMode`]），这里只认它，不反推。
//!
//! 条目布局：`[0]` 标题行，`[1..=n]` 评论，`[n+1]` 脚标（加载 / 重试 / 空态 /
//! 底部留白）。脚标恒占一个条目，状态切换不用动 `ListState` 的条目数。

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Scrollbar, ScrollbarMode, TextSelectionLayer, Theme};

use crate::api::MusicApi;
use crate::models::SongComment;
use crate::ui::components::{comment_row, empty_state, spinner};
use crate::ui::cover_color::{Backdrop, dark_colors, dark_gradient};
use crate::ui::theme::DOLPHIN_FAMILY;

/// 底部留白：原来的 `pb(100.)`，现在由脚标条目的高度承担，
/// 这样 `max_offset_for_scrollbar` 测出来的滚动范围和原来一致。
const FOOTER_SPACING: Pixels = px(100.);

/// 外层这一帧把评论列表当什么。
///
/// 三个状态各对应一种「谁吃位移、位置归谁」，缺一不可：`Owned` 是目的地，
/// `Parked` 是待命，`Held` 是「外层正在飞、内层别动」——少了它，回顶动画
/// 一开始列表就会自己跳回顶部。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ListMode {
    /// 滚轮归列表：外层已经整屏翻到位。
    Owned,
    /// 外层正在飞（「回到唱片」的补间）：不吃位移，也不动位置——
    /// 整页连着评论区一起升上去，内层跟着动就成了两段式。
    Held,
    /// 外层还没翻到评论区：钉在顶部待命。
    Parked,
}

pub(super) struct CommentsView {
    backdrop: Backdrop,
    /// 虚拟化列表状态：条目数由数据驱动（`splice`/`reset`），行高布局时实测。
    list: ListState,
    song_id: Option<u64>,
    generation: u64,
    comments: Vec<SongComment>,
    total: u64,
    offset: usize,
    more: bool,
    loading: bool,
    error: Option<String>,
    request: Option<tokio::task::AbortHandle>,
    /// 这一帧滚轮归谁、位置归谁，见 [`ListMode`]。
    mode: ListMode,
    /// 上一次轮到列表接管滚轮时它停在哪。不接管期间钉回这里，而不是无条件钉到顶
    /// ——指针不在列表上时它根本不会偷滚，无脑钉顶只会让可见内容凭空跳回开头。
    parked_at: ListOffset,
    layout_pending: bool,
}

impl CommentsView {
    pub(super) fn new(backdrop: Backdrop) -> Self {
        Self {
            backdrop,
            // 标题行 + 脚标两个条目起步；歌曲到位后 `set_song` 会 reset。
            // 均匀高度提示让滚动条拇指从一开始就接近真实大小，随实测收敛。
            list: ListState::new(2, ListAlignment::Top, px(300.))
                .with_uniform_item_height(px(110.)),
            song_id: None,
            generation: 0,
            comments: Vec::new(),
            total: 0,
            offset: 0,
            more: false,
            loading: false,
            error: None,
            request: None,
            mode: ListMode::Parked,
            parked_at: ListOffset::default(),
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
        self.list.reset(2);
        self.parked_at = ListOffset::default();
        self.layout_pending = true;
        if id.is_some() {
            self.load(cx);
        }
        cx.notify();
    }

    /// 外层通知这一帧的归属；这一页只有这一个开关。
    pub(super) fn configure(&mut self, mode: ListMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
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
                        // 新评论插在脚标之前：条目 = 标题(1) + 评论 + 脚标(1)。
                        this.list
                            .splice(before + 1..before + 1, this.comments.len() - before);
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

    /// 滚到接近底部就续拉下一页。外层的活（拍板何时接管、翻页补位）不在这里，
    /// 这里只管列表自己还差几页数据。
    fn load_near_bottom(&mut self, cx: &mut Context<Self>) {
        if self.mode != ListMode::Owned || !self.more || self.loading || self.error.is_some() {
            return;
        }
        // 行高要等 prepaint 实测，视口高度为 0 时两者都还没有意义。
        if self.layout_pending || self.list.viewport_bounds().size.height <= px(0.) {
            return;
        }
        let remaining =
            self.list.max_offset_for_scrollbar().y + self.list.scroll_px_offset_for_scrollbar().y;
        if remaining < px(120.) {
            self.load(cx);
        }
    }
}

impl Render for CommentsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owns_scroll = self.mode == ListMode::Owned;
        if self.layout_pending && owns_scroll {
            // 新页内容先经 prepaint 更新行高，再判断是否需要填满视口。
            cx.on_next_frame(window, |this, _, cx| {
                this.layout_pending = false;
                cx.notify();
            });
        } else {
            self.load_near_bottom(cx);
        }

        // 钉住位置：`gpui::list` 的滚轮监听注册在元素内部、外部关不掉，只要指针
        // 落在列表上（外层翻页的路上，露出半屏的评论页就够），它每次都会先吃掉
        // 一段位移。于是在重绘前把位置拉回去——这一帧画出来的评论纹丝不动，位移
        // 全部留给外层把这一屏翻完。
        //
        // 拉回去的目标是 `parked_at`（上一次接管时停的地方）而不是「顶部」：不接管
        // 的这段时间里列表该相对整页冻住、跟着页面一起升降，钉到顶会让指针不在
        // 列表上（因此根本没发生偷滚）的场合也凭空跳回开头。
        //
        // `Held` 什么都不做：外层正带着整页飞，这时拉一下就是让评论内容在飞行
        // 途中跳位置。
        match self.mode {
            ListMode::Owned => self.parked_at = self.list.logical_scroll_top(),
            ListMode::Parked => self.list.scroll_to(self.parked_at),
            ListMode::Held => {}
        }

        // render_item 闭包只有 App 上下文，数据经实体读出来。
        let entity = cx.entity();
        let list_el = list(self.list.clone(), move |ix, _window, cx| {
            let view = entity.read(cx);
            let item = match ix.checked_sub(1).and_then(|i| view.comments.get(i)) {
                Some(comment) => comment_row(comment).into_any_element(),
                None if ix == 0 => heading(view.total),
                None => footer(&entity, view, colors_for(&view.backdrop, cx)),
            };
            // 列表视口撑满整个区域（滚动条贴边），左右留白由每个条目自带。
            // w_full：条目以独立布局根测量，flex 根的 auto 宽会收缩，必须显式拉满。
            div().w_full().px(px(80.)).child(item).into_any_element()
        })
        // `gpui::list` 的条目不走 taffy 布局（prepaint 手动定位），必须显式给尺寸，
        // 否则布局高度为 0，什么都画不出来。
        .size_full()
        .pt(px(24.));

        div()
            .size_full()
            .relative()
            .child(TextSelectionLayer)
            .child(
                div()
                    .id("album-comments-region")
                    .debug_selector(|| "album-comments-region".into())
                    .size_full()
                    .overflow_hidden()
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                        // 没轮到评论区接管时什么都不做，让事件冒泡给外层翻这一屏。
                        if this.mode != ListMode::Owned {
                            return;
                        }
                        // 列表自己的滚轮监听先于本监听器执行（子元素后注册的先跑），
                        // 位移已经被它按边界裁剪消费，这里只看该方向还有没有余量：
                        // 吃得下就拦住事件，吃不下（到顶还要往上、到底还要往下）
                        // 就冒泡，交给外层继续滚。
                        let delta = event.delta.pixel_delta(window.line_height()).y;
                        let scrolled = -this.list.scroll_px_offset_for_scrollbar().y;
                        let room = if delta < px(0.) {
                            this.list.max_offset_for_scrollbar().y - scrolled
                        } else {
                            scrolled
                        };
                        if room >= delta.abs() {
                            cx.stop_propagation();
                        }
                    }))
                    .child(list_el),
            )
            .when(owns_scroll, |comments| {
                comments.child(
                    // 滚动条必须在滚动容器外，避免自身高度计入内容。
                    // gpui-kit 的 Scrollbar 直接实现了对 gpui::ListState 的支持。
                    Scrollbar::vertical(&self.list)
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

fn colors_for(backdrop: &Backdrop, cx: &App) -> ColorTokens {
    dark_colors(
        Theme::global(cx).tokens.colors,
        backdrop.gradient().unwrap_or_else(|| dark_gradient(None)).0[1].into(),
    )
}

/// 标题行：跟随内容一起滚动（原来就在滚动容器里）。
fn heading(total: u64) -> AnyElement {
    div()
        .debug_selector(|| "album-comments-heading".into())
        .flex()
        .items_center()
        .items_start()
        .mb_4()
        .text_color(white())
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("全部评论"),
        )
        .child(
            div()
                .ml(px(1.))
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .font_family(DOLPHIN_FAMILY)
                .child(total.to_string()),
        )
        .into_any_element()
}

/// 脚标条目：加载指示、重试按钮、空态；无事可示时退化为底部留白。
fn footer(entity: &Entity<CommentsView>, view: &CommentsView, colors: ColorTokens) -> AnyElement {
    if view.loading {
        div()
            .w_full()
            .pt(px(20.))
            .flex()
            .justify_center()
            .child(spinner(
                "album-comments-loading",
                20.,
                colors.muted_foreground,
            ))
            .h(FOOTER_SPACING)
            .into_any_element()
    } else if view.error.is_some() {
        let entity = entity.clone();
        Button::new("retry-song-comments")
            .child("评论加载失败，重试")
            .on_click(move |_, _, cx| {
                entity.update(cx, |view, cx| view.load(cx));
            })
            .into_any_element()
    } else if view.comments.is_empty() {
        empty_state("images/empty/cm_empty_wupinglun.png", "还没有评论", colors).into_any_element()
    } else {
        div().h(FOOTER_SPACING).into_any_element()
    }
}
