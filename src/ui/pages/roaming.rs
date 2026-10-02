use gpui::*;

use super::ContentPage;

/// 漫游页
pub struct RoamingPage;

impl Render for RoamingPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(ContentPage::Roaming.title())
    }
}
