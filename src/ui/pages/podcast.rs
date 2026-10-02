use gpui::*;

use super::ContentPage;

/// 播客页
pub struct PodcastPage;

impl Render for PodcastPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(ContentPage::Podcast.title())
    }
}
