use gpui::{App, BoxShadow, Div, Styled, div, px};
use gpui_kit::base::Theme;

/// 共用底色、投影和圆角，不设内边距；尺寸和内容间距由各弹层定义。
pub fn popover_surface(cx: &App) -> Div {
    let colors = Theme::global(cx).tokens.colors;
    div().bg(colors.surface).rounded(px(10.)).shadow(vec![
        BoxShadow::new(px(0.), px(4.), colors.foreground.alpha(0.16)).blur_radius(px(20.)),
    ])
}
