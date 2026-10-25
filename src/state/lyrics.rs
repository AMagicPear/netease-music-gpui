use super::resource::Resource;
use crate::{api::MusicApi, models::LyricLine};
use gpui::{Context, ReadGlobal};

#[derive(Default)]
pub struct LyricsStore {
    song_id: Option<u64>,
    pub data: Resource<Vec<LyricLine>>,
}

impl LyricsStore {
    pub fn lyrics(&self) -> &[LyricLine] {
        self.data.value().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn set_song(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        if self.song_id == id {
            return;
        }
        self.song_id = id;
        self.data.reset();
        self.refresh(cx);
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.song_id else {
            return;
        };
        let client = MusicApi::global(cx).client.clone();
        self.data.fetch(
            "lyrics",
            cx,
            |this| &mut this.data,
            MusicApi::song_lyrics(client, id),
        );
    }
}
