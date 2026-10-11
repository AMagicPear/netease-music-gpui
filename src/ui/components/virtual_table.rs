//! 虚拟滚动表格引擎，外加音乐列表页共用的「表格长相」和行内零件。
//!
//! 歌单详情页和搜索结果页是两个各自独立的页面：列定义、排序、行内交互都不一样，
//! 但表头、行高、列间距、hover 效果这些必须是同一套，否则摆在一起不像同一种表格。
//! 公共部分集中在这里，而不是让搜索页去 `use` 歌单页里的东西——
//! 那等于把「搜索页需要什么」写进歌单页，两个页面从此再也拆不开。
//!
//! 下面分三段：尺寸常量与列模型、表格引擎、公共零件。

use std::{ops::Range, rc::Rc};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use crate::models::Song;
use crate::ui::assets::track_cover_url;
use crate::ui::components::{empty_state, format_duration};
use crate::ui::theme::{PRESSED_ICON_ALPHA, PRESSED_OPACITY};

// ---------------------------------------------------------------------------
// 1. 尺寸常量与列模型
// ---------------------------------------------------------------------------

const OVERSCAN_ROWS: usize = 4;

/// 固定行高。虚拟滚动靠它把行下标换算成 y 坐标，所以行高不能按内容变。
///
/// 导出是因为调用方偶尔要让某个单元格撑满整行 —— 比如让标题格的点击区盖住
/// 行内上下留白，而不是只有文字那么高。
pub const ROW_HEIGHT: Pixels = px(56.);
/// 表头行高。调用者要在表头上叠东西（比如可拖动的分界线）时，用同一个值对齐。
pub const HEADER_HEIGHT: Pixels = px(40.);
/// 行与表头的文字字号。
pub const ROW_TEXT_SIZE: Pixels = px(13.);
/// 列与列之间的间距。表头与每一行用同一个值，列的边界才对得齐。
pub const COLUMN_GAP: Pixels = px(6.);
/// 相邻内容仍相隔 20px：两侧 padding 加上 6px 列间距。
pub const CELL_PADDING: Pixels = px(7.);

/// 内容区左右留白。它同时是行 hover 投影的横向余量：投影拖尾有 3σ = 36px，
/// 而 padding 属于裁剪框内部，所以留白必须加在**滚动容器自己**身上，
/// 留在父级的话投影会被滚动容器的边界切出硬边。
pub const PAGE_SIDE_PADDING: Pixels = px(40.);

/// 次要文字统一用常规字重：副标题、专辑名、时长、计数都靠灰度而不是粗细退让。
pub const SECONDARY_FONT_WEIGHT: FontWeight = FontWeight::NORMAL;

/// 列表上方那排按钮的尺寸，高度和内边距成对使用；那一行按高度对齐，
/// 按钮时有时无时行高才不会塌陷。
pub const ACTION_BUTTON_HEIGHT: Pixels = px(36.);
pub const ACTION_BUTTON_PADDING: Pixels = px(12.);

/// 「喜欢」列的固定宽度。
pub const LIKE_COLUMN_WIDTH: Pixels = px(42.);
/// 「时长」列的固定宽度。
pub const DURATION_COLUMN_WIDTH: Pixels = px(66.);

/// Dolphin 里最宽的数字是 600/1000 em。
const DOLPHIN_WIDEST_DIGIT_EM: f32 = 0.6;

/// 序号列宽。表头和虚拟行各自独立布局，必须共用同一个函数，否则列边界错开。
/// 取「放得下最长序号」和「放得下播放键」两者的较大值。
pub fn index_column_width(row_count: usize) -> Pixels {
    // 末行渲染的是 `format!("{:02}", row_count)`，它就是最长的那个序号。
    let digits = format!("{:02}", row_count).chars().count() as f32;
    let width = digits * DOLPHIN_WIDEST_DIGIT_EM * f32::from(ROW_TEXT_SIZE);
    // 向上取整：小数宽度按设备像素取整后可能差一丁点，而这点差距就够让数字折行。
    px(width.ceil().max(20.))
}

/// hover 投影的 Y 偏移（正数向下）、模糊半径与颜色透明度（颜色取主题墨色，见行 hover 处）。
const ROW_HOVER_SHADOW_OFFSET_Y: Pixels = px(2.);
const ROW_HOVER_SHADOW_BLUR: Pixels = px(12.);
/// 柔光拖尾按几个 sigma 算：GPUI 的投影顶点着色器（`vs_shadow`）恰好把网格外扩 3σ，
/// 片元里高斯又直接取 `sigma = blur_radius`，所以 3σ 就是能画到的最远处，且那里已经淡到看不见。
const ROW_HOVER_SHADOW_TAIL_SIGMAS: f32 = 3.;

/// Some 是固定宽度；None 按 weight 分配剩余宽度。表头与行共用列定义。
pub struct TableColumn {
    pub title: SharedString,
    pub width: Option<Pixels>,
    pub weight: f32,
    /// 表头的水平对齐。单元格内容由调用者自己构建，组件替它对齐不了，所以这里描述的
    /// 其实是「这一列按什么方式对齐」；调用者应让单元格内容用同样的对齐，两边才对得上。
    pub align: TextAlign,
}

impl TableColumn {
    pub fn new(title: impl Into<SharedString>, width: Option<Pixels>) -> Self {
        Self {
            title: title.into(),
            width,
            weight: 1.,
            align: TextAlign::Left,
        }
    }

    pub fn weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// 表头靠右对齐，配套右对齐的数字类列使用。
    pub fn align_right(mut self) -> Self {
        self.align = TextAlign::Right;
        self
    }
}

fn cell(column: &TableColumn, content: AnyElement, padded: bool) -> Div {
    // `min_w(0)` 是给 `truncate()` 让路的：flex 项默认最小宽度等于内容宽度，
    // 不压到 0 的话列会被文字撑宽，而不是把文字截断成省略号。
    let cell = div()
        .min_w(px(0.))
        .overflow_hidden()
        .px(if padded { CELL_PADDING } else { px(0.) })
        .child(content);
    match column.width {
        Some(width) => cell.w(width + CELL_PADDING * 2.).flex_none(),
        None => cell
            .flex_1()
            .flex_basis(CELL_PADDING * 2.)
            .flex_grow(column.weight),
    }
}

/// 表头与数据行共用的骨架：撑满宽度、横向排列、列间距与字号一致，再按列宽把单元格摆好。
fn table_row(
    columns: &[TableColumn],
    cells: impl IntoIterator<Item = AnyElement>,
    padded: bool,
) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(COLUMN_GAP)
        .text_size(ROW_TEXT_SIZE)
        .children(
            columns
                .iter()
                .zip(cells)
                .map(|(column, content)| cell(column, content, padded)),
        )
}

/// 给阴影留出空间；GPUI 仍会与祖先裁剪框取交集。
fn rows_clip(bounds: Bounds<Pixels>) -> ContentMask<Pixels> {
    // 四边都要盖住 3σ 的拖尾，否则尾巴还剩百分之一点几的可见度时就被切断，边缘会留一道硬边。
    // 调用这份代码的滚动容器要在水平方向留足 `PAGE_SIDE_PADDING`（40px > 36px），
    // 并且那笔留白要是它的 padding —— 见 `PAGE_SIDE_PADDING` 的说明。
    let tail = ROW_HOVER_SHADOW_BLUR * ROW_HOVER_SHADOW_TAIL_SIGMAS;
    ContentMask {
        bounds: bounds.extend(Edges {
            left: tail,
            right: tail,
            // 投影整体下移了 offset_y，所以上边可以少留、下边要多留，合起来仍是两个 tail。
            top: tail - ROW_HOVER_SHADOW_OFFSET_Y,
            bottom: tail + ROW_HOVER_SHADOW_OFFSET_Y,
        }),
    }
}

// ---------------------------------------------------------------------------
// 2. 表格引擎
// ---------------------------------------------------------------------------

/// 跟随祖先滚动的固定行高表格。回调仅为可见范围及上下各四行缓冲创建单元格。
///
/// 祖先滚动容器由调用方提供，可以是整页滚动，也可以只包住表格区域 ——
/// 本组件在 `prepaint` 里读当前裁剪框算出可见行，所以对滚动层级的位置没有要求。
#[expect(
    clippy::too_many_arguments,
    reason = "各回调职责独立，保留显式参数，避免为一个调用方引入配置层"
)]
pub fn virtual_table<V: Render>(
    view: Entity<V>,
    id: impl Into<ElementId>,
    columns: Rc<Vec<TableColumn>>,
    row_count: usize,
    render_cells: impl 'static + Fn(&mut V, usize, &mut Window, &mut Context<V>) -> Vec<AnyElement>,
    row_key: impl 'static + Fn(&V, usize, &App) -> u64,
    render_header: impl 'static + Fn(&TableColumn, usize) -> AnyElement,
    on_header_layout: impl 'static + Fn(Vec<Bounds<Pixels>>),
    on_row_hover: impl 'static + Fn(&mut V, u64, bool, &mut Context<V>),
    // 行上右键（非左键的 click）时回调。只把行 key 和窗口坐标交回调用者：
    // 组件不知道菜单是什么，由调用者决定在哪弹什么。
    on_row_aux_click: impl 'static + Fn(&mut V, u64, Point<Pixels>, &mut Window, &mut Context<V>),
    cx: &App,
) -> impl IntoElement {
    let colors = Theme::global(cx).tokens.colors;
    // 每一行都要挂一份回调，所以包成 `Rc`：逐行 clone 的是引用，不是闭包本体。
    let on_row_hover = Rc::new(on_row_hover);
    let on_row_aux_click = Rc::new(on_row_aux_click);
    let header = table_row(
        &columns,
        columns.iter().enumerate().map(|(index, column)| {
            div()
                .w_full()
                .text_align(column.align)
                .whitespace_nowrap()
                .child(render_header(column, index))
                .into_any_element()
        }),
        false,
    )
    .h(HEADER_HEIGHT)
    .on_children_prepainted(move |bounds, _, _| on_header_layout(bounds))
    .text_color(colors.muted_foreground)
    .border_b_1()
    .border_color(colors.foreground.alpha(0.06));
    div().w_full().child(header).child(VirtualRows {
        id: id.into(),
        row_count,
        render_row: Box::new(move |index, window, cx| {
            view.update(cx, |view, cx| {
                let key = row_key(view, index, cx);
                let cells = render_cells(view, index, window, cx);
                assert_eq!(
                    cells.len(),
                    columns.len(),
                    "one cell per table column is required"
                );
                let row = table_row(&columns, cells, true)
                    .id(("row", key))
                    .h(ROW_HEIGHT)
                    .text_color(colors.foreground)
                    // 悬浮状态交给调用者：`.hover()` 只能改这一层的样式，变不了子元素结构
                    // （比如把序号换成播放键、在标题右侧长出操作图标）。
                    .on_hover({
                        let on_row_hover = on_row_hover.clone();
                        cx.listener(move |view, hovered: &bool, _, cx| {
                            on_row_hover(view, key, *hovered, cx);
                        })
                    })
                    // 右键挂在整行上而不是某个图标上，行内任何位置都能唤出菜单。
                    // 用 `on_aux_click`（非左键的 click），和行内「更多」按钮的 `on_click`
                    // 是同一套按下-抬起判定，只是按键不同。
                    .on_aux_click({
                        let on_row_aux_click = on_row_aux_click.clone();
                        cx.listener(move |view, event: &ClickEvent, window, cx| {
                            on_row_aux_click(view, key, event.position(), window, cx);
                        })
                    })
                    // hover 不是「把底色压深」，而是把整行托起来：底色换成项目的表面色
                    .hover(|style| {
                        style.bg(colors.surface).rounded(px(12.)).shadow(vec![
                            BoxShadow::new(
                                px(0.),
                                ROW_HOVER_SHADOW_OFFSET_Y,
                                colors.foreground.alpha(0.1),
                            )
                            .blur_radius(ROW_HOVER_SHADOW_BLUR),
                        ])
                    });
                row.into_any_element()
            })
        }),
    })
}

type RenderRow = dyn FnMut(usize, &mut Window, &mut App) -> AnyElement;

struct VirtualRows {
    id: ElementId,
    row_count: usize,
    render_row: Box<RenderRow>,
}

impl IntoElement for VirtualRows {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// 算出需要渲染的行区间，上下各多带 `OVERSCAN_ROWS` 行缓冲。
fn visible_rows(count: usize, top: f32, bottom: f32) -> Range<usize> {
    let height = f32::from(ROW_HEIGHT);
    if count == 0 || bottom <= 0. || top >= count as f32 * height || bottom <= top {
        return 0..0;
    }
    let start = ((top.max(0.) / height).floor() as usize).saturating_sub(OVERSCAN_ROWS);
    let end = ((bottom.max(0.) / height).ceil() as usize)
        .saturating_add(OVERSCAN_ROWS)
        .min(count);
    start..end
}

impl Element for VirtualRows {
    type RequestLayoutState = ();
    type PrepaintState = Vec<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (ROW_HEIGHT * self.row_count as f32).into();
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        // 外层 overflow 滚动容器在 prepaint 时已经设置了裁剪区域和滚动后的坐标。
        // 因此不用复制滚动偏移，也不用在 render 阶段猜测顶部区域的高度。
        let viewport = window.content_mask().bounds;
        if bounds.right() <= viewport.left() || bounds.left() >= viewport.right() {
            return Vec::new();
        }
        let range = visible_rows(
            self.row_count,
            f32::from(viewport.top() - bounds.top()),
            f32::from(viewport.bottom() - bounds.top()),
        );
        // 裁剪框比行盒子外扩一圈（见 `rows_clip`），好让 hover 投影的柔光画到行外面去。
        window.with_content_mask(Some(rows_clip(bounds)), |window| {
            range
                .map(|index| {
                    let mut row = (self.render_row)(index, window, cx);
                    row.layout_as_root(
                        size(
                            AvailableSpace::Definite(bounds.size.width),
                            AvailableSpace::Definite(ROW_HEIGHT),
                        ),
                        window,
                        cx,
                    );
                    row.prepaint_at(
                        bounds.origin + point(px(0.), ROW_HEIGHT * index as f32),
                        window,
                        cx,
                    );
                    row
                })
                .collect()
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        rows: &mut Vec<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // 同 prepaint，用同一个裁剪框：两处必须一致，否则预排版和绘制会对不上。
        window.with_content_mask(Some(rows_clip(bounds)), |window| {
            for row in rows {
                row.paint(window, cx);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// 3. 公共零件
// ---------------------------------------------------------------------------

/// 「当前 hover 在哪一行」的状态转移规则。两个列表页都只有这一个状态。
///
/// 离开事件只允许清掉「自己」那一行：鼠标从 A 移到 B 时事件可能「B 进入」后才
/// 「A 离开」，无脑写成 `hovered.then_some(id)` 会把已经点亮的新行清掉，
/// 表现就是序号变播放键时有时无。
pub fn hovered_row_after(current: Option<u64>, id: u64, hovered: bool) -> Option<u64> {
    if hovered {
        Some(id)
    } else if current == Some(id) {
        None
    } else {
        current
    }
}

/// 一首歌在列表行里要显示的内容。构造一次、行内多次复用，避免每帧重算。
pub struct SongDisplay {
    pub cover: Option<ImageSource>,
    pub title: SharedString,
    pub subtitle: std::ops::Range<usize>,
    pub album: SharedString,
    pub duration: SharedString,
}

impl SongDisplay {
    pub fn new(song: &Song) -> Self {
        let mut title = song.name.clone();
        let start = title.len();
        if let Some(subtitle) = song.tns.first().or_else(|| song.alia.first()) {
            title.push_str(&format!("（{subtitle}）"));
        }
        let end = title.len();
        Self {
            cover: Some(track_cover_url(song.al.pic_url.as_deref(), 72).into()),
            title: title.into(),
            subtitle: start..end,
            album: song
                .al
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("未知专辑")
                .to_owned()
                .into(),
            duration: format_duration(song.duration()).into(),
        }
    }
}

/// 使用同一段 StyledText，标题和副标题一起截断；颜色随当前主题计算。
pub fn title_with_subtitle(display: &SongDisplay, colors: ColorTokens) -> StyledText {
    if display.subtitle.is_empty() {
        return StyledText::new(display.title.clone());
    }

    // highlight 先 blend 再 fade_out：先替换 RGB，再恢复 alpha。
    // 因此播放中的红色标题不会污染副标题，也无需预合成背景色。
    let subtitle_color = colors.muted_foreground;

    StyledText::new(display.title.clone()).with_highlights([(
        display.subtitle.clone(),
        HighlightStyle {
            color: Some(subtitle_color.alpha(1.)),
            font_weight: Some(SECONDARY_FONT_WEIGHT),
            fade_out: Some(1. - subtitle_color.a),
            ..Default::default()
        },
    )])
}

/// 行 hover 时才出现的图标：序号位置的播放键、标题右侧那排操作按钮，共用这一套样式。
pub fn row_hover_icon(
    id: (&'static str, u64),
    path: &'static str,
    label: &'static str,
    size: Pixels,
    colors: ColorTokens,
) -> AnyElement {
    svg()
        .path(path)
        .size(size)
        .flex_none()
        .text_color(colors.foreground.alpha(0.6))
        .hover(|style| style.text_color(colors.foreground))
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        // active 属于 StatefulInteractiveElement，必须跟在 `.id()` 之后（此时是 Stateful<Svg>）。
        .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
        .into_any_element()
}

/// 播放量：十万以下照写，十万起折算成万，万位满四位不再带小数。
pub fn play_count_label(count: u64) -> String {
    const WAN: u64 = 10_000;
    if count < 10 * WAN {
        return count.to_string();
    }
    let wan = count / WAN;
    // 十分位截断而不是四舍五入：812568 是 81.2万，不是 81.3万。
    let tenth = count % WAN / 1_000;
    if wan >= 1_000 || tenth == 0 {
        format!("{wan}万")
    } else {
        format!("{wan}.{tenth}万")
    }
}

/// 「播放全部」：主题红实心按钮，白字白图标，无边框。
pub fn play_all_button(colors: ColorTokens, compact: bool) -> Button {
    Button::new(if compact {
        "playlist-floating-play"
    } else {
        "playlist-play-all-button"
    })
    .h(ACTION_BUTTON_HEIGHT)
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .gap(px(4.))
    // 宽度交给内容：图标 + 文字 + 左右内边距
    .px(ACTION_BUTTON_PADDING)
    .when(compact, |button| button.w(ACTION_BUTTON_HEIGHT).px(px(0.)))
    .rounded_lg()
    .bg(colors.primary)
    // hover 只改颜色（向背景靠一档），按下统一降整体透明度
    .hover(|style| style.bg(colors.primary.alpha(0.88)))
    .active(|style| style.opacity(PRESSED_OPACITY))
    .text_size(px(13.))
    .text_color(colors.primary_foreground)
    .child(
        svg()
            .path("icons/play.svg")
            .size(px(18.))
            .flex_none()
            .text_color(colors.primary_foreground),
    )
    .aria_label("播放全部")
    .when(!compact, |button| button.child("播放全部"))
}

/// 空状态在列表区水平居中，顶部对齐（不另加边距）。
pub fn centered_empty(image: &'static str, tip: &str, colors: ColorTokens) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .child(empty_state(image, tip, colors))
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{
        PAGE_SIDE_PADDING, ROW_HOVER_SHADOW_BLUR, ROW_HOVER_SHADOW_TAIL_SIGMAS,
        hovered_row_after, index_column_width, play_count_label, visible_rows,
    };

    #[test]
    fn visible_range_tracks_outer_viewport() {
        assert_eq!(visible_rows(1193, -250., 230.), 0..9);
        assert_eq!(visible_rows(1193, 560., 1120.), 6..24);
        assert_eq!(visible_rows(1193, 561., 1121.), 6..25);
        assert_eq!(visible_rows(1193, 66640., 67200.), 1186..1193);
        assert_eq!(visible_rows(1193, -500., -1.), 0..0);
        assert_eq!(visible_rows(1193, 66808., 68000.), 0..0);
        assert_eq!(visible_rows(0, 0., 500.), 0..0);
        assert_eq!(visible_rows(10, 100., 100.), 0..0);
    }

    /// 「后到的离开事件」不能把已经点亮的新行清掉，否则行内序号/播放键的切换会时有时无。
    #[test]
    fn leaving_a_row_only_clears_that_row() {
        // 进入 B，然后 A 离开（无论先后到达）都要停在 B。
        assert_eq!(hovered_row_after(Some(1), 2, true), Some(2));
        assert_eq!(hovered_row_after(Some(2), 1, false), Some(2));
        assert_eq!(hovered_row_after(Some(3), 9, false), Some(3));
        // 同一行自己进出。
        assert_eq!(hovered_row_after(None, 7, true), Some(7));
        assert_eq!(hovered_row_after(Some(7), 7, false), None);
        assert_eq!(hovered_row_after(None, 1, false), None);
    }

    /// 投影拖尾必须小于内容区留白，否则水平方向会被滚动容器切出硬边。
    #[test]
    fn side_padding_exceeds_the_shadow_tail() {
        let tail = ROW_HOVER_SHADOW_BLUR * ROW_HOVER_SHADOW_TAIL_SIGMAS;
        assert!(f32::from(PAGE_SIDE_PADDING) > f32::from(tail));
    }

    /// 下面的期望值按 13px 字号算出来的，改字号时这里会红。
    #[test]
    fn index_column_follows_digits_but_never_below_the_play_icon() {
        // 一位数只有 7.8px，下限 20px 兜住播放键。
        assert_eq!(index_column_width(9), px(20.));
        assert_eq!(index_column_width(30), px(20.));
        assert_eq!(index_column_width(999), px(24.));
        // `{:02}` 只补到两位不截断，1193 是四位数。
        assert_eq!(index_column_width(1193), px(32.));
    }

    #[test]
    fn play_count_switches_to_wan_and_truncates_rather_than_rounds() {
        assert_eq!(play_count_label(0), "0");
        assert_eq!(play_count_label(99_999), "99999");
        assert_eq!(play_count_label(100_000), "10万");
        // 十分位截断：812568 是 81.2万，不是 81.3万。
        assert_eq!(play_count_label(812_568), "81.2万");
        assert_eq!(play_count_label(150_000), "15万");
        // 万位满四位不再带小数。
        assert_eq!(play_count_label(12_345_678), "1234万");
    }
}
