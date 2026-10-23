//! 跟随祖先滚动容器的固定行高表格，业务页面提供列、单元格和行事件。

use std::{ops::Range, rc::Rc};

use gpui::*;
use gpui_kit::base::{InteractiveElementExt, Theme};

const OVERSCAN_ROWS: usize = 4;

/// 虚拟布局按固定行高换算下标与坐标。
pub const ROW_HEIGHT: Pixels = px(56.);
pub const HEADER_HEIGHT: Pixels = px(40.);
pub const ROW_TEXT_SIZE: Pixels = px(13.);
pub const COLUMN_GAP: Pixels = px(6.);
pub const CELL_PADDING: Pixels = px(7.);

/// 留白必须加在滚动容器内部，避免裁掉行 hover 的 36px 阴影拖尾。
pub const PAGE_SIDE_PADDING: Pixels = px(40.);

const ROW_HOVER_SHADOW_OFFSET_Y: Pixels = px(2.);
const ROW_HOVER_SHADOW_BLUR: Pixels = px(12.);
/// GPUI 的投影顶点向外扩展 3σ。
const ROW_HOVER_SHADOW_TAIL_SIGMAS: f32 = 3.;

/// Some 是固定宽度；None 按 weight 分配剩余宽度。表头与行共用列定义。
pub struct TableColumn {
    pub title: SharedString,
    pub width: Option<Pixels>,
    pub weight: f32,
    /// 表头对齐方式，调用方的单元格应采用相同对齐。
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

    pub fn align_right(mut self) -> Self {
        self.align = TextAlign::Right;
        self
    }
}

fn cell(column: &TableColumn, content: AnyElement, padded: bool) -> Div {
    // 允许 flex 项收缩，长文本才能截断。
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
    let tail = ROW_HOVER_SHADOW_BLUR * ROW_HOVER_SHADOW_TAIL_SIGMAS;
    ContentMask {
        bounds: bounds.extend(Edges {
            left: tail,
            right: tail,
            top: tail - ROW_HOVER_SHADOW_OFFSET_Y,
            bottom: tail + ROW_HOVER_SHADOW_OFFSET_Y,
        }),
    }
}

/// 只创建可见范围及上下缓冲行，滚动容器由调用方提供。
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
    on_row_double_click: impl 'static + Fn(&mut V, u64, &mut Context<V>),
    on_row_aux_click: impl 'static + Fn(&mut V, u64, Point<Pixels>, &mut Window, &mut Context<V>),
    cx: &App,
) -> impl IntoElement {
    let colors = Theme::global(cx).tokens.colors;
    let on_row_hover = Rc::new(on_row_hover);
    let on_row_double_click = Rc::new(on_row_double_click);
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
                    // 结构变化由页面处理，hover 样式只能改变当前元素。
                    .on_hover({
                        let on_row_hover = on_row_hover.clone();
                        cx.listener(move |view, hovered: &bool, _, cx| {
                            on_row_hover(view, key, *hovered, cx);
                        })
                    })
                    .on_double_click({
                        let on_row_double_click = on_row_double_click.clone();
                        cx.listener(move |view, _, _, cx| on_row_double_click(view, key, cx))
                    })
                    .on_aux_click({
                        let on_row_aux_click = on_row_aux_click.clone();
                        cx.listener(move |view, event: &ClickEvent, window, cx| {
                            on_row_aux_click(view, key, event.position(), window, cx);
                        })
                    })
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
        // prepaint 时祖先已提交裁剪框和滚动后的坐标。
        let viewport = window.content_mask().bounds;
        if bounds.right() <= viewport.left() || bounds.left() >= viewport.right() {
            return Vec::new();
        }
        let range = visible_rows(
            self.row_count,
            f32::from(viewport.top() - bounds.top()),
            f32::from(viewport.bottom() - bounds.top()),
        );
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
