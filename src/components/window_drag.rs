use gpui::*;

/// 视图只要把一个 `bool` 标志位暴露出来，就可以用
/// [`WindowDragArea::window_drag`] 把任意元素变成窗口拖动手柄。
///
/// 之所以用 trait 而不是把状态直接塞进这个模块，是因为 GPUI 的事件回调
/// 只能通过 `&mut View` 访问状态，借助 trait 就能让不同视图复用同一套逻辑，
/// 而各自仍然保有对自身字段的所有权。
pub(crate) trait WindowDragState {
    fn window_move_pending_mut(&mut self) -> &mut bool;
}

pub(crate) trait WindowDragArea: Sized {
    /// 将当前元素注册为窗口拖动手柄：在该区域按下左键并移动鼠标即可拖动整个窗口。
    ///
    /// 需要元素先调用 `.id()` 变成 `Stateful<Div>`，因为鼠标事件只在这一层生效。
    fn window_drag<V>(self, cx: &mut Context<V>) -> Self
    where
        V: WindowDragState + 'static;
}

impl WindowDragArea for Stateful<Div> {
    fn window_drag<V>(self, cx: &mut Context<V>) -> Self
    where
        V: WindowDragState + 'static,
    {
        self.on_mouse_down(
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
            // 只在"按下之后第一次移动"时启动拖动，随后立刻清掉标志位，
            // 避免窗口移动期间鼠标事件继续触发重复调用。
            if *this.window_move_pending_mut() {
                *this.window_move_pending_mut() = false;
                window.start_window_move();
            }
        }))
    }
}
