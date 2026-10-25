use super::{
    account::AccountState,
    resource::{Resource, request},
};
use crate::{
    api::MusicApi,
    models::{Playlist, Song},
};
use gpui::{Context, Entity, ReadGlobal, Subscription};
use std::collections::{HashMap, HashSet};

mod collection;
pub use collection::SongsCollected;

#[derive(Default)]
pub struct MusicLibrary {
    pub playlists: Resource<Vec<Playlist>>,
    pub likes: Resource<HashSet<u64>>,
    pub liked_song_cache: HashMap<u64, Song>,
    pub user_id: u64,
    generation: u64,
    pending_likes: HashSet<u64>,
    _account: Option<Subscription>,
}

impl MusicLibrary {
    pub fn new(account: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&account, |this, account, cx| {
            let account = account.read(cx);
            if !account.loading && this.user_id != account.profile.user_id {
                this.set_account(account.profile.user_id, cx);
            }
        });
        let mut this = Self {
            _account: Some(subscription),
            ..Default::default()
        };
        let account = account.read(cx);
        if !account.loading {
            this.set_account(account.profile.user_id, cx);
        }
        this
    }

    fn set_account(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.user_id = user_id;
        self.playlists.reset();
        self.likes.reset();
        self.pending_likes.clear();
        self.liked_song_cache.clear();
        self.retry(cx);
    }

    pub fn playlists(&self) -> &[Playlist] {
        self.playlists.value().map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn is_liked(&self, id: u64) -> bool {
        self.likes.value().is_some_and(|ids| ids.contains(&id))
    }
    pub fn favorite_playlist(&self) -> Option<&Playlist> {
        self.playlists()
            .iter()
            .find(|playlist| playlist.special_type == 5)
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        let id = self.user_id;
        if id == 0 {
            self.playlists.set(Vec::new());
            self.likes.set(HashSet::new());
            cx.notify();
            return;
        }
        let client = MusicApi::global(cx).client.clone();
        let playlists_client = client.clone();
        self.playlists.fetch(
            "library playlists",
            cx,
            |this| &mut this.playlists,
            async move { MusicApi::user_playlists(&playlists_client, id).await },
        );
        if self.pending_likes.is_empty() {
            self.likes
                .fetch("liked songs", cx, |this| &mut this.likes, async move {
                    MusicApi::liked_song_ids(&client, id)
                        .await
                        .map(|ids| ids.into_iter().collect())
                });
        }
    }

    pub fn toggle_like(&mut self, song: Song, cx: &mut Context<Self>) {
        let id = song.id;
        if id == 0
            || self.user_id == 0
            || self.likes.value().is_none()
            || !self.pending_likes.insert(id)
        {
            return;
        }
        let liked = !self.is_liked(id);
        self.set_liked_locally(&song, liked);
        let client = MusicApi::global(cx).client.clone();
        let generation = self.generation;
        // 写操作由音乐库持有，切换页面不会取消它。
        request(
            cx,
            "toggle like",
            MusicApi::set_song_liked(client, id, liked),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                this.pending_likes.remove(&id);
                if result.is_err() {
                    this.set_liked_locally(&song, !liked);
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn set_liked_locally(&mut self, song: &Song, liked: bool) {
        self.likes.cancel();
        if let Some(ids) = self.likes.value_mut() {
            if liked {
                ids.insert(song.id);
                self.liked_song_cache.insert(song.id, song.clone());
            } else {
                ids.remove(&song.id);
                self.liked_song_cache.remove(&song.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimistic_like_invalidates_an_older_read_and_can_roll_back() {
        let mut library = MusicLibrary::default();
        library.likes.set(HashSet::from([1]));
        let read = library.likes.begin();
        let song = Song {
            id: 2,
            ..Default::default()
        };
        library.set_liked_locally(&song, true);
        assert!(!library.likes.apply(read, Ok(HashSet::from([1]))));
        assert!(library.is_liked(1) && library.is_liked(2));
        library.set_liked_locally(&song, false);
        assert!(library.is_liked(1) && !library.is_liked(2));
        assert!(!library.liked_song_cache.contains_key(&2));
    }
}
