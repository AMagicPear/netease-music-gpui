use gpui::*;

use crate::theme::Theme;

pub struct ProgressBar {
    theme: Theme,
    hovered: bool,
}

impl ProgressBar {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            hovered: false,
        }
    }
}

impl Render for ProgressBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let track_height = if self.hovered { px(4.) } else { px(2.) };
        let theme = self.theme;

        div()
            .id("player-progress-bar")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(12.))
            .on_hover(cx.listener(|this, hovered, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .child(div().w_full().h(track_height).bg(theme.black10).child(
                div().h_full().w(relative(0.35)).bg(linear_gradient(
                    270.,
                    linear_color_stop(theme.secondary1_2, 0.),
                    linear_color_stop(theme.secondary1_1, 1.),
                )),
            ))
    }
}
