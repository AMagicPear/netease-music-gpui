use gpui::*;

use crate::state::playback::PlaybackState;
use crate::theme::Theme;

pub struct ProgressBar {
    theme: Theme,
    playback: Entity<PlaybackState>,
    hovered: bool,
    _playback_subscription: Subscription,
}

impl ProgressBar {
    pub fn new(theme: Theme, playback: Entity<PlaybackState>, cx: &mut Context<Self>) -> Self {
        let playback_subscription = cx.observe(&playback, |_, _, cx| cx.notify());
        Self {
            theme,
            playback,
            hovered: false,
            _playback_subscription: playback_subscription,
        }
    }
}

impl Render for ProgressBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let track_height = if self.hovered { px(4.) } else { px(2.) };
        let theme = self.theme;
        let progress = self.playback.read(cx).progress();

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
                div().h_full().w(relative(progress)).bg(linear_gradient(
                    270.,
                    linear_color_stop(theme.secondary1_2, 0.),
                    linear_color_stop(theme.secondary1_1, 1.),
                )),
            ))
    }
}
