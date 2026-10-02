use gpui::*;

use super::ContentPage;

/// 关注页
pub struct FollowingPage;

impl Render for FollowingPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(ContentPage::Following.title())
    }
}
