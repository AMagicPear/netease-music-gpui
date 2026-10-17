//! 歌曲 / 歌单共用的变高虚拟评论列表：分页、重试与滚动位置由同一个实体管理。
//!
//! 评论行高随正文换行变化，`gpui::list` 在布局时实测行高、只构建视口附近的
//! 条目（gpui-kit 的 `VirtualList` 需要预先给出每行精确高度，`component::list`
//! 要求等高行，都不适合这种「正文任意换行」的列表）。
//!
//! 在播放页中嵌入第二屏，两层都要吃滚轮。规矩只有一条：
//! **外层翻到评论区（整屏到位）之前，一个像素都不给评论列表**。判定由外层算好
//! 传进来（[`ListMode`]），这里只认它，不反推。
//!
//! 条目布局：可选页面头部、「精选评论」小节（有热评才出现）、「最新评论」小节、评论、
//! 脚标（加载 / 重试 / 空态 / 留白）。页面头部参与同一个滚动范围；脚标恒占一个条目，
//! 状态切换不改变条目数。

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Scrollbar, ScrollbarMode, TextSelectionHandle, Theme};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::api::MusicApi;
use crate::models::{SongComment, SongComments};
use crate::ui::components::{comment_row, empty_state, spinner};
use crate::ui::cover_color::{Backdrop, dark_colors, dark_gradient};
use crate::ui::theme::DOLPHIN_FAMILY;

/// 底部留白：原来的 `pb(100.)`，现在由脚标条目的高度承担，
/// 这样 `max_offset_for_scrollbar` 测出来的滚动范围和原来一致。
const FOOTER_SPACING: Pixels = px(100.);
/// 「精选评论」小节最多展示的热评条数。
const HOT_SECTION_MAX: usize = 5;

type ListHeader = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// 一条评论的选中身份：`handle` 按评论 id 稳定持有（见 [`comment_row`] 的
/// [`CommentSelection`]），`refresh` 订阅它的选区变化并强制窗口重绘。
///
/// 为什么需要订阅：`gpui::list` 的条目是有缓存视图的，鼠标拖动本身不会让它们
/// 重绘，而 `SelectableText` 的高亮只在 paint 时投影。不主动刷新的话，拖选时
/// 高亮要等下一次别的原因触发重绘才出现 —— 表现就是「选完了过一会儿才显示」。
struct SelectionEntry {
    handle: TextSelectionHandle,
    refresh: Option<Subscription>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommentResource {
    Song(u64),
    Playlist(u64),
}

/// 外层决定何时交出滚轮；翻页和返回动画期间列表都保持原位。
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
    resource: Option<CommentResource>,
    generation: u64,
    comments: Vec<SongComment>,
    /// 评论 id → 选中身份。随评论一起生长，切资源时整体丢弃。
    selection: HashMap<u64, SelectionEntry>,
    /// 开头多少条热评进「精选评论」小节：实际入列的条数，最多
    /// [`HOT_SECTION_MAX`]；为 0 时整节（含标题）不出现。
    hot_rows: usize,
    /// 接口给的热评总数，只用于小节标题上的数字（小节最多只显 5 行）。
    hot_total: u64,
    seen: HashSet<u64>,
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
    pub(in crate::ui) fn new(backdrop: Option<Backdrop>) -> Self {
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
            resource: None,
            generation: 0,
            comments: Vec::new(),
            selection: HashMap::new(),
            hot_rows: 0,
            hot_total: 0,
            seen: HashSet::new(),
            total: 0,
            offset: 0,
            more: false,
            loading: false,
            error: None,
            request: None,
            mode,
            parked_at: ListOffset::default(),
            layout_pending: false,
        }
    }

    pub fn set_resource(&mut self, resource: Option<CommentResource>, cx: &mut Context<Self>) {
        if self.resource == resource {
            return;
        }
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.resource = resource;
        self.generation = self.generation.wrapping_add(1);
        self.comments.clear();
        self.selection.clear();
        self.hot_rows = 0;
        self.hot_total = 0;
        self.seen.clear();
        self.total = 0;
        self.offset = 0;
        self.more = false;
        self.loading = false;
        self.error = None;
        self.list.reset(2 + usize::from(self.header.is_some()));
        self.parked_at = ListOffset::default();
        self.layout_pending = true;
        if resource.is_some() {
            self.load(cx);
        }
        cx.notify();
    }

    /// 调用方手上已经有的评论数：先用它渲染标题，接口返回后再覆盖成准确值。
    pub fn set_estimated_total(&mut self, total: Option<u64>, cx: &mut Context<Self>) {
        if let Some(total) = total {
            self.total = total;
            cx.notify();
        }
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
    fn item_at(&self, ix: usize) -> Item {
        let featured = self.hot_rows > 0;
        if ix == if featured { self.hot_rows + 1 } else { 0 } {
            return Item::LatestHeading;
        }
        if ix == 0 && featured {
            return Item::FeaturedHeading;
        }
        // 让出「精选评论」标题；过了最新标题的行再让一个位置。
        let Some(row) = ix.checked_sub(1) else {
            return Item::Footer;
        };
        let row = if featured && row >= self.hot_rows {
            row - 1
        } else {
            row
        };
        if row < self.comments.len() {
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
        self.mode = mode;
        self.layout_pending = true;
        cx.notify();
        true
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(resource) = self.resource else {
            return;
        };
        let offset = self.offset;
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let request = api.runtime.spawn(async move {
            match resource {
                CommentResource::Song(id) => MusicApi::song_comments(client, id, offset).await,
                CommentResource::Playlist(id) => {
                    MusicApi::playlist_comments(client, id, offset).await
                }
            }
        });
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request.await.unwrap_or_else(|_| Err("评论加载失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.finish(resource, generation, offset, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn finish(
        &mut self,
        resource: CommentResource,
        generation: u64,
        offset: usize,
        result: Result<SongComments, String>,
    ) -> bool {
        // abort 不能撤回已送达主线程的结果，资源类型、id 与代次都必须匹配。
        if self.resource != Some(resource) || self.generation != generation {
            return false;
        }
        self.loading = false;
        self.request = None;
        match result {
            Ok(page) => {
                self.error = None;
                self.total = page.total;
                self.more = page.more;
                self.offset = offset + 20;
                let before = self.comments.len();
                for comment in page.comments {
                    if self.seen.insert(comment.id) {
                        self.comments.push(comment);
                    }
                }
                if self.comments.len() == before {
                    self.more = false;
                }
                // 热评只合并在第一页；行数以真正入列的为准（去重可能吃掉几条），
                // 标题上的数字用总数——超出 5 条的热评只是不进精选小节，
                // 它们留在列表里跟着进「最新评论」。
                if offset == 0 {
                    self.hot_rows = page.hot_count.min(HOT_SECTION_MAX).min(self.comments.len());
                    self.hot_total = page.hot_count as u64;
                }
                // 精选小节标题插在行前面；脚标恒在末尾，插在它之前
                // 保留已有行的测量与滚动锚点。
                let section = usize::from(offset == 0 && self.hot_rows > 0);
                let footer = before + 1 + usize::from(self.header.is_some());
                self.list
                    .splice(footer..footer, (self.comments.len() - before) + section);
                self.layout_pending = true;
            }
            Err(error) => self.error = Some(error),
        }
        true
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

        // 滚动条也能改变位置；渲染时同步锚点，非接管状态则保持原位。
        match self.mode {
            ListMode::Owned => self.parked_at = self.list.logical_scroll_top(),
            ListMode::Parked => self.list.scroll_to(self.parked_at),
        }

        // 每条评论一个稳定的选中身份。handle 必须跨帧复用：`SelectableText`
        // 把拖拽锚点、runs 的几何存在 handle 里，每帧新建等于每帧丢锚点。
        for index in 0..self.comments.len() {
            let id = self.comments[index].id;
            if !self.selection.contains_key(&id) {
                // 与 `comment_row::body` 渲染的文本保持一致：handle 存的是展开后的
                // 文本，表情已变成 Unicode 字符，复制出来也是 emoji 而不是 `[大笑]`。
                let text = crate::ui::emoji::expand(&self.comments[index].content).into_owned();
                let handle = TextSelectionHandle::new(text, cx);
                self.selection.insert(
                    id,
                    SelectionEntry {
                        handle,
                        refresh: None,
                    },
                );
            }
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
            // 行组件颜色全部走 tokens：深色封面上的取值由 colors_for 统一改写。
            let colors = colors_for(view.backdrop.as_ref(), cx);
            let item = match view.item_at(ix) {
                Item::LatestHeading => heading(view.total, colors, view.hot_rows > 0),
                Item::FeaturedHeading => featured_heading(view.hot_total, colors),
                Item::Comment(row) => {
                    let comment = &view.comments[row];
                    // 顺序用评论在列表里的下标：热评在前、其余在后，与视觉顺序一致。
                    let selection = view
                        .selection
                        .get(&comment.id)
                        .map(|entry| (entry.handle.clone(), row as u64));
                    comment_row(comment, colors, selection).into_any_element()
                }
                Item::Footer => footer(&entity, view, colors),
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
        div()
            .h(FOOTER_SPACING)
            .pt(px(20.))
            .text_size(px(12.))
            .text_color(colors.muted_foreground)
            .flex()
            .justify_center()
            .when(!view.more, |footer| footer.child("没有更多评论了"))
            .into_any_element()
    }
}

impl Drop for CommentsView {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommentResource, CommentsView, Item, SongComment, SongComments};
    use gpui::{IntoElement, div};
    use std::rc::Rc;

    fn page(ids: &[u64]) -> Result<SongComments, String> {
        Ok(SongComments {
            total: 100,
            hot_count: 0,
            more: true,
            comments: ids
                .iter()
                .map(|&id| SongComment {
                    id,
                    nickname: String::new(),
                    avatar_url: String::new(),
                    content: String::new(),
                    time: 0,
                    liked_count: 0,
                })
                .collect(),
        })
    }

    #[test]
    fn hot_comments_get_their_own_section_header() {
        let resource = CommentResource::Song(1);
        let mut view = CommentsView::new(None);
        view.resource = Some(resource);
        let mut first = page(&[1, 2, 3]).unwrap();
        first.hot_count = 2;
        assert!(view.finish(resource, 0, 0, Ok(first)));
        assert_eq!(view.hot_rows, 2);
        // 精选标题 + 2 行热评 + 最新标题 + 1 行 + 脚标。
        assert_eq!(view.list.item_count(), 6);
        // 翻页不再带小节；重复的热评（id 2）被去重，只进一行新的。
        assert!(view.finish(resource, 0, 20, page(&[2, 4])));
        assert_eq!(view.list.item_count(), 7);
        assert_eq!(
            view.comments
                .iter()
                .map(|comment| comment.id)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn hot_section_keeps_at_most_five_rows() {
        let resource = CommentResource::Song(1);
        let mut view = CommentsView::new(None);
        view.resource = Some(resource);
        let mut first = page(&[1, 2, 3, 4, 5, 6, 7]).unwrap();
        first.hot_count = 7;
        assert!(view.finish(resource, 0, 0, Ok(first)));
        // 小节里只列 5 行，标题上的数字是总数。
        assert_eq!(view.hot_rows, 5);
        assert_eq!(view.hot_total, 7);
        // 精选标题 + 5 行热评 + 最新标题 + 剩下 2 行 + 脚标。
        assert_eq!(view.list.item_count(), 10);
    }

    fn items(view: &CommentsView, len: usize) -> Vec<Item> {
        (0..len).map(|ix| view.item_at(ix)).collect()
    }

    #[test]
    fn items_are_two_sections_then_footer() {
        let resource = CommentResource::Song(1);
        let mut view = CommentsView::new(None);
        view.resource = Some(resource);
        let mut first = page(&[1, 2, 3, 4, 5]).unwrap();
        first.hot_count = 2;
        assert!(view.finish(resource, 0, 0, Ok(first)));
        // 精选标题 → 2 行热评 → 最新标题 → 其余 3 行 → 脚标。
        assert_eq!(
            items(&view, 8),
            [
                Item::FeaturedHeading,
                Item::Comment(0),
                Item::Comment(1),
                Item::LatestHeading,
                Item::Comment(2),
                Item::Comment(3),
                Item::Comment(4),
                Item::Footer,
            ]
        );
    }

    #[test]
    fn no_hot_comments_means_a_single_latest_section() {
        let resource = CommentResource::Playlist(1);
        let mut view = CommentsView::new(None);
        view.resource = Some(resource);
        assert!(view.finish(resource, 0, 0, page(&[1, 2, 3])));
        assert_eq!(
            items(&view, 5),
            [
                Item::LatestHeading,
                Item::Comment(0),
                Item::Comment(1),
                Item::Comment(2),
                Item::Footer,
            ]
        );
    }

    #[test]
    fn paging_deduplicates_and_retry_preserves_existing_comments() {
        let resource = CommentResource::Playlist(1);
        let mut view = CommentsView::new(None);
        view.resource = Some(resource);
        assert!(view.finish(resource, 0, 0, page(&[1, 2, 2])));
        assert_eq!(view.list.item_count(), 4);
        assert!(view.finish(resource, 0, 20, Err("断网".into())));
        assert_eq!(view.offset, 20);
        assert_eq!(view.comments.len(), 2);
        assert!(view.finish(resource, 0, 20, page(&[2, 3])));
        assert!(view.error.is_none());
        assert_eq!(view.offset, 40);
        assert_eq!(view.list.item_count(), 5);
        assert!(view.finish(resource, 0, 40, page(&[3])));
        assert!(!view.more);
    }

    #[test]
    fn old_resource_and_generation_cannot_finish_current_request() {
        let mut view = CommentsView::new(None);
        view.resource = Some(CommentResource::Playlist(1));
        view.generation = 2;
        view.loading = true;
        assert!(!view.finish(CommentResource::Song(1), 2, 0, page(&[1])));
        assert!(!view.finish(CommentResource::Playlist(1), 1, 0, page(&[1])));
        assert!(view.loading);
        assert!(view.comments.is_empty());
    }

    #[test]
    fn paging_keeps_the_page_header_and_footer_in_the_list() {
        let resource = CommentResource::Playlist(1);
        let mut view = CommentsView::new(None);
        view.resource = Some(resource);
        view.header = Some(Rc::new(|_, _| div().into_any_element()));
        view.list.reset(3);
        assert!(view.finish(resource, 0, 0, page(&[1, 2])));
        assert_eq!(view.list.item_count(), 5);
        assert!(view.finish(resource, 0, 20, page(&[2, 3])));
        assert_eq!(view.list.item_count(), 6);
        assert_eq!(
            view.comments
                .iter()
                .map(|comment| comment.id)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }
}
