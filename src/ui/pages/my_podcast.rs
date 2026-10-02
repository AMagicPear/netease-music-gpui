use gpui::*;

use super::ContentPage;

/// 我的播客
pub struct MyPodcastPage;

impl Render for MyPodcastPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(ContentPage::MyPodcast.title())
    }
}
