use gpui::*;

use super::ContentPage;

/// 精选页
pub struct FeaturedPage;

impl Render for FeaturedPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(ContentPage::Featured.title())
    }
}
