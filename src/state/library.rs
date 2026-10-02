use std::collections::HashSet;

use gpui::{Context, Entity, ReadGlobal, Subscription};

use super::{playlist::Playlist, song::Song, user::UserProfile};
use crate::api::MusicApi;

#[derive(Default)]
pub struct MusicLibrary {
    pub playlists: Vec<Playlist>,
    pub songs: Vec<Song>,
    pub liked_song_ids: HashSet<u64>,
    pub user_id: u64,
    pub loading: bool,
    pub error: Option<String>,
    _user_subscription: Option<Subscription>,
}

impl MusicLibrary {
    pub fn new(user: Entity<UserProfile>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&user, |this, user, cx| {
            let user_id = user.read(cx).user_id;
            if user_id == 0 {
                this.loading = false;
                this.error = Some("请检查 COOKIE 登录状态".into());
                cx.notify();
                return;
            }
            if this.user_id == user_id {
                return;
            }
            this.user_id = user_id;
            let api = MusicApi::global(cx);
            let request = api
                .runtime
                .spawn(MusicApi::library(api.client.clone(), user_id));
            cx.spawn(async move |this, cx| {
                let result = request
                    .await
                    .unwrap_or_else(|_| Err("音乐库请求任务失败".into()));
                let _ = this.update(cx, |library, cx| {
                    library.loading = false;
                    match result {
                        Ok((playlists, songs, liked_song_ids)) => {
                            library.playlists = playlists;
                            library.songs = songs;
                            library.liked_song_ids = liked_song_ids.into_iter().collect();
                        }
                        Err(message) => {
                            eprintln!("{message}");
                            library.error = Some(message);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        });
        Self {
            loading: true,
            _user_subscription: Some(subscription),
            ..Default::default()
        }
    }

    pub fn favorite_playlist(&self) -> Option<&Playlist> {
        self.playlists
            .iter()
            .find(|playlist| playlist.special_type == 5)
    }
}
