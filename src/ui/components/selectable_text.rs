//! 评论正文的可选文本。复用 gpui-base 的选择状态，仅在本地控制选区绘制。
//! 元素生命周期沿用 gpui-kit 1e41f17 的 SelectableText（Apache-2.0）。

use std::ops::Range;

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, StyledText, Window, fill, point,
};

use gpui_kit::base::{
    TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun,
};

pub(in crate::ui) struct SelectableText {
    id: ElementId,
    handle: TextSelectionHandle,
    text: SharedString,
    styled_text: StyledText,
    document_order: u64,
}

impl SelectableText {
    pub fn new(
        id: impl Into<ElementId>,
        handle: TextSelectionHandle,
        text: impl Into<SharedString>,
        document_order: u64,
    ) -> Self {
        let text = text.into();
        Self {
            id: id.into(),
            handle,
            styled_text: StyledText::new(text.clone()),
            text,
            document_order,
        }
    }

    fn paint_selection(
        layout: &gpui::TextLayout,
        range: Range<usize>,
        color: Hsla,
        window: &mut Window,
    ) {
        let (Some(start), Some(end)) = (
            layout.position_for_index(range.start),
            layout.position_for_index(range.end),
        ) else {
            return;
        };
        let line_height = layout.line_height();
        let origin = layout.bounds().origin;
        let mut y = origin.y;
        for line in layout.line_layouts() {
            let mut previous_x = Pixels::ZERO;
            let row_ends = line
                .wrap_boundaries
                .iter()
                .map(|boundary| {
                    line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix]
                        .position
                        .x
                })
                .chain([line.unwrapped_layout.width]);
            for (row, row_end) in row_ends.enumerate() {
                let row_y = y + line_height * row;
                let right = origin.x + row_end - previous_x;
                if row_y >= start.y && row_y <= end.y {
                    let left = if row_y == start.y {
                        start.x.max(origin.x)
                    } else {
                        origin.x
                    };
                    let right = if row_y == end.y {
                        end.x.min(right)
                    } else {
                        right
                    };
                    if right > left {
                        window.paint_quad(fill(
                            Bounds::from_corners(
                                point(left, row_y),
                                point(right, row_y + line_height),
                            ),
                            color,
                        ));
                    }
                }
                previous_x = row_end;
            }
            y += line.size(line_height).height;
        }
    }
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.styled_text
            .request_layout(global_id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled_text
            .prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_document_order(self.document_order)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(&self.handle, window, cx);
        self.handle.register(registration, window, cx);
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled_text.layout().clone();
        let selected_text_before = TextSelection::selected_text(window, cx);
        let projection = self.handle.update_runs(
            &[
                TextSelectionRun::new(self.text.clone(), layout.clone(), bounds)
                    .with_document_order(self.document_order),
            ],
            cx,
        );
        if selected_text_before != TextSelection::selected_text(window, cx) {
            window.refresh();
        }
        let color = gpui_kit::base::Theme::global(cx).tokens.colors.selection;
        for range in projection.ranges().iter().flatten().cloned() {
            Self::paint_selection(&layout, range, color, window);
        }
        self.styled_text.paint(
            global_id,
            inspector_id,
            bounds,
            &mut (),
            &mut (),
            window,
            cx,
        );
    }
}
