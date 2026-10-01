use std::{ops::Range, rc::Rc};

use gpui::*;
use gpui_kit::base::Theme;

const OVERSCAN_ROWS: usize = 4;
const ROW_HEIGHT: Pixels = px(56.);

/// 固定宽度列传 Some；None 列平分剩余宽度。表头与单元格共用同一套宽度。
pub struct TableColumn {
    pub title: SharedString,
    pub width: Option<Pixels>,
}

impl TableColumn {
    pub fn new(title: impl Into<SharedString>, width: Option<Pixels>) -> Self {
        Self {
            title: title.into(),
            width,
        }
    }
}

fn cell(column: &TableColumn, content: AnyElement) -> Div {
    let cell = div()
        .min_w(px(0.))
        .px(px(12.))
        .overflow_hidden()
        .child(content);
    match column.width {
        Some(width) => cell.w(width).flex_none(),
        None => cell.flex_1(),
    }
}

/// 跟随祖先滚动的固定行高表格。回调仅为可见范围及上下各四行缓冲创建单元格。
/// 数据、列的业务含义与顶部页面内容都由调用者决定。
pub fn virtual_table<V: Render>(
    view: Entity<V>,
    id: impl Into<ElementId>,
    columns: Rc<Vec<TableColumn>>,
    row_count: usize,
    render_cells: impl 'static + Fn(&mut V, usize, &mut Window, &mut Context<V>) -> Vec<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    let colors = Theme::global(cx).tokens.colors;
    let header = div()
        .w_full()
        .h(px(36.))
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(colors.muted_foreground)
        .border_b_1()
        .border_color(colors.foreground.alpha(0.06))
        .children(
            columns
                .iter()
                .map(|column| cell(column, div().child(column.title.clone()).into_any_element())),
        );
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
                let row = div()
                    .id(("row", index))
                    .w_full()
                    .h(ROW_HEIGHT)
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .text_color(colors.foreground)
                    .hover(|style| style.bg(colors.foreground.alpha(0.06)));
                row.children(
                    columns
                        .iter()
                        .zip(cells)
                        .map(|(column, content)| cell(column, content)),
                )
                .into_any_element()
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

// ponytail: 固定行高；需要换行或展开行时再引入逐行高度缓存。
fn visible_rows(count: usize, height: f32, top: f32, bottom: f32) -> Range<usize> {
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
            f32::from(ROW_HEIGHT),
            f32::from(viewport.top() - bounds.top()),
            f32::from(viewport.bottom() - bounds.top()),
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
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
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
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
        assert_eq!(visible_rows(1193, 56., -250., 230.), 0..9);
        assert_eq!(visible_rows(1193, 56., 560., 1120.), 6..24);
        assert_eq!(visible_rows(1193, 56., 561., 1121.), 6..25);
        assert_eq!(visible_rows(1193, 56., 66640., 67200.), 1186..1193);
        assert_eq!(visible_rows(1193, 56., -500., -1.), 0..0);
        assert_eq!(visible_rows(1193, 56., 66808., 68000.), 0..0);
        assert_eq!(visible_rows(0, 56., 0., 500.), 0..0);
        assert_eq!(visible_rows(10, 56., 100., 100.), 0..0);
    }
}
