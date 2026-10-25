use super::resource::Resource;
use crate::{
    api::MusicApi,
    models::{Playlist, Song},
};
use gpui::{Context, ReadGlobal};

#[derive(Default)]
pub struct SearchResults {
    pub keywords: Option<String>,
    pub songs: Resource<Vec<Song>>,
    pub playlists: Resource<Vec<Playlist>>,
}

impl SearchResults {
    pub fn songs(&self) -> &[Song] {
        self.songs.value().map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn playlists(&self) -> &[Playlist] {
        self.playlists.value().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn search(&mut self, keywords: String, cx: &mut Context<Self>) {
        if self.keywords.as_deref() != Some(&keywords) {
            self.songs.reset();
            self.playlists.reset();
            self.keywords = Some(keywords);
        }
        self.refresh_songs(cx);
    }

    pub fn refresh_songs(&mut self, cx: &mut Context<Self>) {
        let Some(keywords) = self.keywords.clone() else {
            return;
        };
        let client = MusicApi::global(cx).client.clone();
        self.songs
            .fetch("search songs", cx, |this| &mut this.songs, async move {
                MusicApi::search_songs(client, &keywords).await
            });
    }

    pub fn search_playlists(&mut self, cx: &mut Context<Self>) {
        let Some(keywords) = self.keywords.clone() else {
            return;
        };
        let client = MusicApi::global(cx).client.clone();
        self.playlists.fetch(
            "search playlists",
            cx,
            |this| &mut this.playlists,
            async move { MusicApi::search_playlists(client, &keywords).await },
        );
    }
}
