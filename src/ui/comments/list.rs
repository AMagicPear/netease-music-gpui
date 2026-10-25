//! 变高虚拟评论列表；数据由 CommentsStore 管理，滚动和选区留在 View。

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Scrollbar, ScrollbarMode, TextSelectionHandle, Theme};
use std::{collections::HashMap, rc::Rc};

use crate::models::CommentResource;
use crate::state::comments::{CommentPublished as Published, CommentsStore};
use crate::ui::components::{empty_state, spinner};
use crate::ui::cover_color::{Backdrop, dark_colors, dark_gradient};
use crate::ui::events::{CommentCountChanged, CommentPublished};
use crate::ui::theme::DOLPHIN_FAMILY;

use super::comment_row;

/// 留白参与列表测量，确保滚动条范围包含底部间距。
const FOOTER_SPACING: Pixels = px(100.);
/// 「精选评论」小节最多展示的热评条数。
const HOT_SECTION_MAX: usize = 5;

type ListHeader = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// 按评论 ID 保留选区，并订阅变化以重绘虚拟列表的缓存条目。
struct SelectionEntry {
    handle: TextSelectionHandle,
    refresh: Option<Subscription>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListMode {
    /// 滚轮归列表：外层已经整屏翻到位。
    Owned,
    /// 外层还没翻到评论区：保持上次位置，滚轮交给外层。
    Parked,
}

/// 列表里一个下标的角色。条目顺序：
/// 精选标题（有热评才有）、热评行、最新评论标题、其余行、脚标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    FeaturedHeading,
    LatestHeading,
    Comment(usize),
    Footer,
}

pub struct CommentsView {
    backdrop: Option<Backdrop>,
    /// 虚拟化列表状态：条目数由数据驱动（`splice`/`reset`），行高布局时实测。
    list: ListState,
    header: Option<ListHeader>,
    data: Entity<CommentsStore>,
    revision: u64,
    rows: usize,
    selection: HashMap<u64, SelectionEntry>,
    _data_subscription: Subscription,
    _publish_subscription: Subscription,
    /// 这一帧滚轮归谁、位置归谁，见 [`ListMode`]。
    mode: ListMode,
    /// 上一次轮到列表接管滚轮时它停在哪。不接管期间钉回这里，而不是无条件钉到顶
    /// ——指针不在列表上时它根本不会偷滚，无脑钉顶只会让可见内容凭空跳回开头。
    parked_at: ListOffset,
    layout_pending: bool,
}

/// 发布成功只发一个信号：编辑框要不要清草稿、怎么清，是消费者的事。
impl EventEmitter<CommentPublished> for CommentsView {}
impl EventEmitter<CommentCountChanged> for CommentsView {}

impl CommentsView {
    pub(in crate::ui) fn new(backdrop: Option<Backdrop>, cx: &mut Context<Self>) -> Self {
        let data = cx.new(|_| CommentsStore::default());
        let data_subscription = cx.observe(&data, |this, data, cx| {
            let data = data.read(cx);
            let len = data.comments().len();
            if this.revision != data.revision {
                this.revision = data.revision;
                this.selection.clear();
                this.list.reset(
                    usize::from(this.header.is_some())
                        + usize::from(data.hot_count() > 0)
                        + len
                        + 2,
                );
                this.parked_at = ListOffset::default();
            } else if len > this.rows {
                let footer = this.rows
                    + usize::from(data.hot_count() > 0)
                    + usize::from(this.header.is_some())
                    + 1;
                this.list.splice(footer..footer, len - this.rows);
            }
            let count = data
                .data
                .value()
                .and(data.resource)
                .map(|resource| CommentCountChanged {
                    resource,
                    total: data.total(),
                });
            if let Some(count) = count {
                cx.emit(count);
            }
            this.rows = len;
            this.layout_pending = true;
            cx.notify();
        });
        let publish_subscription =
            cx.subscribe(&data, |_, _, _: &Published, cx| cx.emit(CommentPublished));
        let mode = if backdrop.is_some() {
            ListMode::Parked
        } else {
            ListMode::Owned
        };
        Self {
            backdrop,
            // 标题行 + 脚标两个条目起步；资源到位后 reset。
            // 均匀高度提示让滚动条拇指从一开始就接近真实大小，随实测收敛。
            list: ListState::new(2, ListAlignment::Top, px(300.))
                .with_uniform_item_height(px(110.)),
            header: None,
            data,
            revision: 0,
            rows: 0,
            selection: HashMap::new(),
            _data_subscription: data_subscription,
            _publish_subscription: publish_subscription,
            mode,
            parked_at: ListOffset::default(),
            layout_pending: false,
        }
    }

    pub fn open_resource(
        &mut self,
        resource: CommentResource,
        total: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        self.data.update(cx, |data, cx| {
            if data.resource == Some(resource) {
                data.refresh(cx);
            } else {
                data.set_resource(Some(resource), cx);
            }
            data.estimate_total(total, cx);
        });
    }

    pub fn set_resource(&mut self, resource: Option<CommentResource>, cx: &mut Context<Self>) {
        self.data
            .update(cx, |data, cx| data.set_resource(resource, cx));
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.data.update(cx, |data, cx| data.refresh(cx));
    }

    /// 页面头部也是列表条目，和评论共用滚动范围及滚动条。
    pub fn set_header(
        &mut self,
        header: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.header.is_none() {
            self.list.splice(0..0, 1);
        }
        self.header = Some(Rc::new(header));
        self.layout_pending = true;
        cx.notify();
    }

    pub fn scroll_state(&self) -> ListState {
        self.list.clone()
    }

    /// 条目下标 → 角色。只在「最新评论」标题那里把行下标让一位：它插在热评行
    /// 与其余行之间；没有精选时它直接占 0，不需要让。
    fn item_at(&self, ix: usize, cx: &App) -> Item {
        let data = self.data.read(cx);
        let hot_rows = data.hot_count().min(HOT_SECTION_MAX);
        let featured = hot_rows > 0;
        if ix == if featured { hot_rows + 1 } else { 0 } {
            return Item::LatestHeading;
        }
        if ix == 0 && featured {
            return Item::FeaturedHeading;
        }
        // 让出「精选评论」标题；过了最新标题的行再让一个位置。
        let Some(row) = ix.checked_sub(1) else {
            return Item::Footer;
        };
        let row = if featured && row >= hot_rows {
            row - 1
        } else {
            row
        };
        if row < data.comments().len() {
            Item::Comment(row)
        } else {
            Item::Footer
        }
    }

    /// 外层通知这一帧的归属；这一页只有这一个开关。
    pub fn configure(&mut self, mode: ListMode, cx: &mut Context<Self>) -> bool {
        if self.mode == mode {
            return false;
        }
        if self.mode == ListMode::Owned {
            self.parked_at = self.list.logical_scroll_top();
        }
        if mode == ListMode::Owned {
            self.refresh(cx);
        }
        self.mode = mode;
        self.layout_pending = true;
        cx.notify();
        true
    }

    pub fn publish(&mut self, content: String, cx: &mut Context<Self>) {
        self.data.update(cx, |data, cx| data.publish(content, cx));
    }

    pub fn publishing(&self, cx: &App) -> bool {
        self.data.read(cx).publish.is_loading()
    }
    pub fn publish_error(&self, cx: &App) -> Option<String> {
        self.data.read(cx).publish.error().map(str::to_owned)
    }
    pub fn clear_publish_error(&mut self, cx: &mut Context<Self>) {
        self.data
            .update(cx, |data, cx| data.clear_publish_error(cx));
    }

    /// 滚到接近底部就续拉下一页。外层的活（拍板何时接管、翻页补位）不在这里，
    /// 这里只管列表自己还差几页数据。
    fn load_near_bottom(&mut self, cx: &mut Context<Self>) {
        let data = self.data.read(cx);
        if self.mode != ListMode::Owned
            || !data.more()
            || data.data.is_loading()
            || data.load_failed()
        {
            return;
        }
        // 行高要等 prepaint 实测，视口高度为 0 时两者都还没有意义。
        if self.layout_pending || self.list.viewport_bounds().size.height <= px(0.) {
            return;
        }
        let remaining =
            self.list.max_offset_for_scrollbar().y + self.list.scroll_px_offset_for_scrollbar().y;
        if remaining < px(120.) {
            self.data.update(cx, |data, cx| data.load_more(cx));
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

        // 滚动条也能改变位置；渲染时同步锚点，非接管状态则保持原位。
        match self.mode {
            ListMode::Owned => self.parked_at = self.list.logical_scroll_top(),
            ListMode::Parked => self.list.scroll_to(self.parked_at),
        }

        // 每条评论一个稳定的选中身份。handle 必须跨帧复用：`SelectableText`
        // 把拖拽锚点、runs 的几何存在 handle 里，每帧新建等于每帧丢锚点。
        let missing: Vec<_> = self
            .data
            .read(cx)
            .comments()
            .iter()
            .filter(|comment| !self.selection.contains_key(&comment.id))
            .map(|comment| {
                (
                    comment.id,
                    crate::ui::emoji::expand(&comment.content).into_owned(),
                )
            })
            .collect();
        for (id, text) in missing {
            // 与 `comment_row::body` 渲染的文本保持一致：handle 存的是展开后的
            // 文本，表情已变成 Unicode 字符，复制出来也是 emoji 而不是 `[大笑]`。
            let handle = TextSelectionHandle::new(text, cx);
            self.selection.insert(
                id,
                SelectionEntry {
                    handle,
                    refresh: None,
                },
            );
        }
        // 只订阅一次：选区每变一次就让窗口重绘一帧，高亮跟着指针走。
        for entry in self.selection.values_mut() {
            if entry.refresh.is_none() {
                entry.refresh = Some(entry.handle.refresh_window_on_change(window, cx));
            }
        }

        // render_item 闭包只有 App 上下文，数据经实体读出来。
        let entity = cx.entity();
        let list_el = list(self.list.clone(), move |ix, window, cx| {
            let header = entity.read(cx).header.clone();
            if ix == 0
                && let Some(header) = header.as_ref()
            {
                return header(window, cx);
            }
            let ix = ix - usize::from(header.is_some());
            let view = entity.read(cx);
            let data = view.data.read(cx);
            // 行组件颜色全部走 tokens：深色封面上的取值由 colors_for 统一改写。
            let colors = colors_for(view.backdrop.as_ref(), cx);
            let item = match view.item_at(ix, cx) {
                Item::LatestHeading => heading(data.total(), colors, data.hot_count() > 0),
                Item::FeaturedHeading => featured_heading(data.hot_count() as u64, colors),
                Item::Comment(row) => {
                    let comment = &data.comments()[row];
                    // 顺序用评论在列表里的下标：热评在前、其余在后，与视觉顺序一致。
                    let selection = (view.selection[&comment.id].handle.clone(), row as u64);
                    comment_row(comment, colors, selection).into_any_element()
                }
                Item::Footer => footer(&entity, data, colors),
            };
            // 列表视口撑满整个区域（滚动条贴边），左右留白由每个条目自带。
            // w_full：条目以独立布局根测量，flex 根的 auto 宽会收缩，必须显式拉满。
            let padding = if view.backdrop.is_some() {
                px(80.)
            } else {
                px(40.)
            };
            div().w_full().px(padding).child(item).into_any_element()
        })
        // `gpui::list` 的条目不走 taffy 布局（prepaint 手动定位），必须显式给尺寸，
        // 否则布局高度为 0，什么都画不出来。
        .size_full()
        .pt(if self.header.is_some() {
            px(0.)
        } else {
            px(24.)
        });

        let colors = colors_for(self.backdrop.as_ref(), cx);
        div()
            .size_full()
            .relative()
            // 选择宿主由窗口根（gpui-kit 的 `Root`）统一安装一次。这里再挂一个
            // `TextSelectionLayer` 会有两套 WindowSelectionState 同时接管鼠标：
            // 一套收不到参与者，另一套又被抢事件，选中行为就不稳定。
            .child(
                div()
                    .id("album-comments-region")
                    .debug_selector(|| "album-comments-region".into())
                    .size_full()
                    .overflow_hidden()
                    .on_scroll_wheel(cx.listener(|this, _: &ScrollWheelEvent, _, cx| {
                        // 原生 list 先执行滚动；未接管时立即撤销，不能等缓存视图重绘。
                        if this.mode != ListMode::Owned {
                            this.list.scroll_to(this.parked_at);
                            return;
                        }
                        let top = this.list.logical_scroll_top();
                        let moved = top.item_ix != this.parked_at.item_ix
                            || top.offset_in_item != this.parked_at.offset_in_item;
                        this.parked_at = top;
                        // 到顶的这一事件也由内层独占；下一事件无位移才交给外层。
                        if moved {
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
                        .mode(if self.header.is_some() {
                            ScrollbarMode::Always
                        } else {
                            ScrollbarMode::Scrolling
                        })
                        .styles(|styles| {
                            styles
                                .thumb(|thumb| thumb.bg(colors.foreground.alpha(0.25)))
                                .thumb_hover(|thumb| thumb.bg(colors.foreground.alpha(0.35)))
                        }),
                )
            })
    }
}

fn colors_for(backdrop: Option<&Backdrop>, cx: &App) -> ColorTokens {
    let colors = Theme::global(cx).tokens.colors;
    match backdrop {
        Some(backdrop) => dark_colors(
            colors,
            backdrop.gradient().unwrap_or_else(|| dark_gradient(None)).0[1].into(),
        ),
        None => colors,
    }
}

/// 「最新评论」小节标题：跟随内容一起滚动（原来就在滚动容器里）。总数未知（0）时先不显数。
/// `separated`：上面还有精选小节，要点一下间距，否则紧贴着热评行。
fn heading(total: u64, colors: ColorTokens, separated: bool) -> AnyElement {
    section("最新评论", (total > 0).then_some(total), colors)
        .when(separated, |heading| heading.mt(px(12.)))
        .debug_selector(|| "album-comments-heading".into())
        .into_any_element()
}

/// 「精选评论」小节标题：数字是热评总数，小节本身最多只列 [`HOT_SECTION_MAX`] 行。
fn featured_heading(hot: u64, colors: ColorTokens) -> AnyElement {
    section("精选评论", (hot > 0).then_some(hot), colors).into_any_element()
}

/// 小节标题。下面不额外留白：评论行自带 19px 顶内距，靠它顶开就够。
fn section(text: &'static str, count: Option<u64>, colors: ColorTokens) -> Div {
    div()
        .flex()
        .items_start()
        .text_color(colors.foreground)
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(text),
        )
        .children(count.map(|total| {
            div()
                .ml(px(1.))
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .font_family(DOLPHIN_FAMILY)
                .child(total.to_string())
        }))
}

/// 脚标条目：加载指示、重试按钮、空态；无事可示时退化为底部留白。
fn footer(entity: &Entity<CommentsView>, data: &CommentsStore, colors: ColorTokens) -> AnyElement {
    if data.data.is_loading() && data.data.value().is_none() {
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
    } else if data.load_failed() {
        let entity = entity.clone();
        Button::new("retry-song-comments")
            .child("评论加载失败，重试")
            .on_click(move |_, _, cx| {
                entity.update(cx, |view, cx| {
                    view.data.update(cx, |data, cx| data.retry(cx));
                });
            })
            .into_any_element()
    } else if data.comments().is_empty() {
        empty_state("images/empty/cm_empty_wupinglun.png", "还没有评论", colors).into_any_element()
    } else {
        div()
            .h(FOOTER_SPACING)
            .pt(px(20.))
            .text_size(px(12.))
            .text_color(colors.muted_foreground)
            .flex()
            .justify_center()
            .when(!data.more(), |footer| footer.child("没有更多评论了"))
            .into_any_element()
    }
}
