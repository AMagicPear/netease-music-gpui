use gpui::*;

use super::ContentPage;

/// 推荐页
pub struct RecommendPage;

impl Render for RecommendPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(ContentPage::Recommend.title())
    }
}
