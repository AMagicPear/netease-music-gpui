use std::{ops::Range, rc::Rc};

use gpui::*;
use gpui_kit::base::Theme;

const OVERSCAN_ROWS: usize = 4;
const ROW_HEIGHT: Pixels = px(56.);
/// 表头行高。调用者要在表头上叠东西（比如可拖动的分界线）时，用同一个值对齐。
pub const HEADER_HEIGHT: Pixels = px(36.);
/// 行与表头的文字字号。
pub const ROW_TEXT_SIZE: Pixels = px(13.);
/// 列与列之间的间距。表头与每一行用同一个值，列的边界才对得齐。
///
/// 它落在列宽**之外**（列宽只描述这一列自己的盒子）：调用者要在某两条列边界之间放东西，
/// 比如可拖动的分界线，间距就是天然的落点 —— 那时它同时也是拖动热区的宽度，别调太小。
pub const COLUMN_GAP: Pixels = px(20.);

/// 行 hover 时的圆角。
///
/// 它同时作用于底色和投影：GPUI 画 drop shadow 时读的就是元素的 `corner_radii`，
/// 所以底色圆了、投影也跟着圆，不用两处各写一遍。
const ROW_HOVER_RADIUS: Pixels = px(12.);
/// hover 投影的 Y 偏移（正数向下）、模糊半径与颜色透明度（颜色取主题墨色，见行 hover 处）。
///
/// 偏移刻意不为 0：整个投影往下挪一点，下方就比上方重一档，读起来是"这一行被托起来"，
/// 而不是四边等重的发光圈（上方有效范围 ≈ 模糊 − 偏移，下方 ≈ 模糊 + 偏移）。
/// spread 保持 0：一旦有了扩散，56px 的行高里会先摊出一层实色，像描边而不像阴影。
const ROW_HOVER_SHADOW_OFFSET_Y: Pixels = px(2.);
const ROW_HOVER_SHADOW_BLUR: Pixels = px(12.);
const ROW_HOVER_SHADOW_ALPHA: f32 = 0.10;

/// 行区域的裁剪框：比行盒子四周各让出「投影能糊出去多远」。
///
/// 不这么做的话投影会被切平，左右两侧最先看出来 —— 那两条边正好压在列边界上。
/// 外扩量取四周的最大值：竖直方向是 `模糊 + |偏移|`（下移之后往下糊得更远），水平方向只有 `模糊`，
/// 而 `Bounds::dilate` 只能均匀外扩，所以按最大值来，宁可多留一点。
/// 它直接从投影参数算出来，改偏移或模糊时自动跟着变，不存在"两处要同步"的问题。
///
/// 放宽是安全的：行内容本身不可能溢出（单元格都带 `overflow_hidden`，行也是
/// `Definite(ROW_HEIGHT)` 定高的），所以唯一会画到行盒子外面的就是投影；再往外还有页面内边距
/// 和滚动容器的裁剪兜着，阴影糊不出表格所在的区域。
fn rows_clip(bounds: Bounds<Pixels>) -> ContentMask<Pixels> {
    ContentMask {
        bounds: bounds.dilate(ROW_HOVER_SHADOW_BLUR + ROW_HOVER_SHADOW_OFFSET_Y),
    }
}

/// 固定宽度列传 Some；None 列平分剩余宽度。表头与单元格共用同一套宽度。
pub struct TableColumn {
    pub title: SharedString,
    pub width: Option<Pixels>,
    /// 表头的水平对齐。单元格内容由调用者自己构建，组件替它对齐不了，所以这里描述的
    /// 其实是「这一列按什么方式对齐」；调用者应让单元格内容用同样的对齐，两边才对得上。
    pub align: TextAlign,
}

impl TableColumn {
    pub fn new(title: impl Into<SharedString>, width: Option<Pixels>) -> Self {
        Self {
            title: title.into(),
            width,
            align: TextAlign::Left,
        }
    }

    /// 表头靠右对齐，配套右对齐的数字类列使用。
    pub fn align_right(mut self) -> Self {
        self.align = TextAlign::Right;
        self
    }
}

fn cell(column: &TableColumn, content: AnyElement) -> Div {
    // `min_w(0)` 是给 `truncate()` 让路的：flex 项默认最小宽度等于内容宽度，
    // 不压到 0 的话列会被文字撑宽，而不是把文字截断成省略号。
    let cell = div().min_w(px(0.)).overflow_hidden().child(content);
    match column.width {
        Some(width) => cell.w(width).flex_none(),
        None => cell.flex_1(),
    }
}

/// 表头与数据行共用的骨架：撑满宽度、横向排列、列间距与字号一致，再按列宽把单元格摆好。
///
/// 表头是一棵布局树，每个数据行又是**各自独立**的一棵（虚拟化的要求），两边的列边界
/// 只能靠这里保持一致。列间距、行内字号、单元格包装收在一处，就不会出现"改了一边、
/// 表头和数据悄悄错位"的 bug。
fn table_row(columns: &[TableColumn], cells: impl IntoIterator<Item = AnyElement>) -> Div {
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
                .map(|(column, content)| cell(column, content)),
        )
}

/// 跟随祖先滚动的固定行高表格。回调仅为可见范围及上下各四行缓冲创建单元格。
/// 数据、列的业务含义与顶部页面内容都由调用者决定。
///
/// `on_row_hover` 在某一行悬浮状态变化时回调（行号 + 是否进入）。样式层面的 `.hover()`
/// 改不了子元素结构，所以"hover 时长出额外内容"这类需求要靠调用者记住行号再重绘。
pub fn virtual_table<V: Render>(
    view: Entity<V>,
    id: impl Into<ElementId>,
    columns: Rc<Vec<TableColumn>>,
    row_count: usize,
    render_cells: impl 'static + Fn(&mut V, usize, &mut Window, &mut Context<V>) -> Vec<AnyElement>,
    on_row_hover: impl 'static + Fn(&mut V, usize, bool, &mut Context<V>),
    cx: &App,
) -> impl IntoElement {
    let colors = Theme::global(cx).tokens.colors;
    // 每一行都要挂一份回调，所以包成 `Rc`：逐行 clone 的是引用，不是闭包本体。
    let on_row_hover = Rc::new(on_row_hover);
    let header = table_row(
        &columns,
        columns.iter().map(|column| {
            div()
                // 撑满单元格后按列的对齐方式摆表头，「#」这类右对齐列才跟数字对齐。
                .w_full()
                .text_align(column.align)
                // 表头就是一行标签，列再窄也不该折成两行。
                .whitespace_nowrap()
                .child(column.title.clone())
                .into_any_element()
        }),
    )
    .h(HEADER_HEIGHT)
    .text_color(colors.muted_foreground)
    .border_b_1()
    .border_color(colors.foreground.alpha(0.06));
    div().w_full().child(header).child(VirtualRows {
        id: id.into(),
        row_count,
        render_row: Box::new(move |index, window, cx| {
            view.update(cx, |view, cx| {
                let cells = render_cells(view, index, window, cx);
                assert_eq!(
                    cells.len(),
                    columns.len(),
                    "one cell per table column is required"
                );
                let row = table_row(&columns, cells)
                    .id(("row", index))
                    .h(ROW_HEIGHT)
                    .text_color(colors.foreground)
                    // 悬浮状态交给调用者：`.hover()` 只能改这一层的样式，变不了子元素结构
                    // （比如把序号换成播放键、在标题右侧长出操作图标）。
                    .on_hover({
                        let on_row_hover = on_row_hover.clone();
                        cx.listener(move |view, hovered: &bool, _, cx| {
                            on_row_hover(view, index, *hovered, cx);
                        })
                    })
                    // hover 不是「把底色压深」，而是把整行托起来：底色换成项目的表面色
                    // （`colors.surface` = #fafafa，播放栏用的同一层柔和白），配圆角 + 柔和投影。
                    // 不用纯白：纯白是"最亮"而不是"项目的白"，换主题时会跟其它卡片脱节。
                    //
                    // 投影只留模糊半径 + 一点向下的偏移，不给 spread：偏移让下方比上方重一档，
                    // 这一行看起来才是被托起来的；加 spread 则会先铺开一层实色，在 56px 的行高里
                    // 就是一条硬边，像描边而不像阴影。颜色用主题墨色而不是纯黑，
                    // 和 `player_bar` 的投影同一套做法，深浅随主题走。
                    //
                    // 参数都用上面的常量：`rows_clip()` 直接从它们算出裁剪框要外扩多少。
                    .hover(|style| {
                        style
                            .bg(colors.surface)
                            .rounded(ROW_HOVER_RADIUS)
                            .shadow(vec![
                                BoxShadow::new(
                                    px(0.),
                                    ROW_HOVER_SHADOW_OFFSET_Y,
                                    colors.foreground.alpha(ROW_HOVER_SHADOW_ALPHA),
                                )
                                .blur_radius(ROW_HOVER_SHADOW_BLUR),
                            ])
                    });
                row.into_any_element()
            })
        }),
    })
}

struct VirtualRows {
    id: ElementId,
    row_count: usize,
    render_row: Box<dyn FnMut(usize, &mut Window, &mut App) -> AnyElement>,
}

impl IntoElement for VirtualRows {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// 算出需要渲染的行区间，上下各多带 `OVERSCAN_ROWS` 行缓冲。
///
/// `top` / `bottom` 是视口相对表格顶部的纵坐标。行高固定（`ROW_HEIGHT`），
/// 所以第 n 行的位置就是 `ROW_HEIGHT * n`，一次除法即可定位。
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

#[cfg(test)]
mod tests {
    use super::visible_rows;

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
}
