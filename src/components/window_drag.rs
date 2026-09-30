use gpui::*;
use gpui_kit::base::InteractiveElementExt;

pub(crate) trait WindowDragState {
    fn window_move_pending_mut(&mut self) -> &mut bool;
}

pub(crate) fn window_drag_region<V>(id: impl Into<ElementId>, cx: &mut Context<V>) -> Stateful<Div>
where
    V: WindowDragState + 'static,
{
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        .on_double_click(|_, window, _| {
            #[cfg(target_os = "macos")]
            window.titlebar_double_click();
            #[cfg(target_os = "linux")]
            window.zoom_window();
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| *this.window_move_pending_mut() = true),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, _| *this.window_move_pending_mut() = false),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, _, _, _| *this.window_move_pending_mut() = false),
        )
        .on_mouse_down_out(cx.listener(|this, _, _, _| *this.window_move_pending_mut() = false))
        .on_mouse_move(cx.listener(|this, _, window, _| {
            if *this.window_move_pending_mut() {
                *this.window_move_pending_mut() = false;
                window.start_window_move();
            }
        }))
}
