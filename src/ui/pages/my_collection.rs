use gpui::*;

use super::ContentPage;

/// 我的收藏
pub struct MyCollectionPage;

impl Render for MyCollectionPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(ContentPage::MyCollection.title())
    }
}
