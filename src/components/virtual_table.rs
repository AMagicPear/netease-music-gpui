use std::{ops::Range, rc::Rc};

use gpui::*;
use gpui_kit::base::Theme;

const OVERSCAN_ROWS: usize = 4;
const ROW_HEIGHT: Pixels = px(56.);
/// 表头行高。调用者要在表头上叠东西（比如可拖动的分界线）时，用同一个值对齐。
pub const HEADER_HEIGHT: Pixels = px(40.);
/// 行与表头的文字字号。
pub const ROW_TEXT_SIZE: Pixels = px(13.);
/// 列与列之间的间距。表头与每一行用同一个值，列的边界才对得齐。
pub const COLUMN_GAP: Pixels = px(6.);
/// 相邻内容仍相隔 20px：两侧 padding 加上 6px 列间距。
pub const CELL_PADDING: Pixels = px(7.);

/// hover 投影的 Y 偏移（正数向下）、模糊半径与颜色透明度（颜色取主题墨色，见行 hover 处）。
const ROW_HOVER_SHADOW_OFFSET_Y: Pixels = px(2.);
const ROW_HOVER_SHADOW_BLUR: Pixels = px(12.);

/// 给阴影留出空间；GPUI 仍会与祖先裁剪框取交集。
fn rows_clip(bounds: Bounds<Pixels>) -> ContentMask<Pixels> {
    ContentMask {
        bounds: bounds.dilate(ROW_HOVER_SHADOW_BLUR + ROW_HOVER_SHADOW_OFFSET_Y),
    }
}

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

fn cell(column: &TableColumn, content: AnyElement) -> Div {
    // `min_w(0)` 是给 `truncate()` 让路的：flex 项默认最小宽度等于内容宽度，
    // 不压到 0 的话列会被文字撑宽，而不是把文字截断成省略号。
    let cell = div().min_w(px(0.)).overflow_hidden().child(content);
    match column.width {
        Some(width) => cell.w(width + CELL_PADDING * 2.).flex_none(),
        None => cell
            .flex_1()
            .flex_basis(CELL_PADDING * 2.)
            .flex_grow(column.weight),
    }
}

/// 表头与数据行共用的骨架：撑满宽度、横向排列、列间距与字号一致，再按列宽把单元格摆好。
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
    cx: &App,
) -> impl IntoElement {
    let colors = Theme::global(cx).tokens.colors;
    // 每一行都要挂一份回调，所以包成 `Rc`：逐行 clone 的是引用，不是闭包本体。
    let on_row_hover = Rc::new(on_row_hover);
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
                let row = table_row(
                    &columns,
                    cells.into_iter().map(|content| {
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .px(CELL_PADDING)
                            .child(content)
                            .into_any_element()
                    }),
                )
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
