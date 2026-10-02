use gpui::*;

/// 拖动分隔条时跟着鼠标走的拖影 —— 实际上什么都不画。
///
/// GPUI 的 `on_drag` 必须返回一个 `Entity` 当"被拖起来的东西"，而这个场景里真正被拖的是
/// 分隔条的位置（由视图状态推出来、每帧重排），不需要有东西跟着光标画，所以给一个零尺寸
/// 的空视图就够。侧边栏分隔条（`MainContent`）和列分界线（`FavoriteMusicPage`）都用它。
pub struct ResizeDragPreview;

impl Render for ResizeDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.))
    }
}
