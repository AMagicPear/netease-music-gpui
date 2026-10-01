use gpui::*;

use super::ContentPage;

/// 最近播放
pub struct RecentPage;

impl Render for RecentPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(ContentPage::Recent.title())
    }
}
