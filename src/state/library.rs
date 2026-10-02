use std::collections::HashSet;

use gpui::{Context, Entity, ReadGlobal, Subscription};

use super::{playlist::Playlist, user::UserProfile};
use crate::api::MusicApi;

#[derive(Default)]
pub struct MusicLibrary {
    pub playlists: Vec<Playlist>,
    pub liked_song_ids: HashSet<u64>,
    pub user_id: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub likes_error: Option<String>,
    generation: u64,
    _user_subscription: Option<Subscription>,
}

impl MusicLibrary {
    pub fn new(user: Entity<UserProfile>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&user, |this, user, cx| {
            this.load(user.read(cx).user_id, cx);
        });
        let mut this = Self {
            loading: true,
            _user_subscription: Some(subscription),
            ..Default::default()
        };
        let user_id = user.read(cx).user_id;
        if user_id != 0 {
            this.load(user_id, cx);
        }
        this
    }

    fn load(&mut self, user_id: u64, cx: &mut Context<Self>) {
        if user_id != 0
            && self.user_id == user_id
            && (self.loading || (self.error.is_none() && self.likes_error.is_none()))
        {
            return;
        }
        if self.user_id != user_id {
            self.playlists.clear();
            self.liked_song_ids.clear();
        }
        self.user_id = user_id;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.error = None;
        self.likes_error = None;
        self.loading = user_id != 0;
        if user_id == 0 {
            self.error = Some("请检查 COOKIE 登录状态".into());
            cx.notify();
            return;
        }
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let request = api.runtime.spawn(async move {
            tokio::join!(
                MusicApi::user_playlists(&client, user_id),
                MusicApi::liked_song_ids(&client, user_id)
            )
        });
        cx.spawn(async move |this, cx| {
            let (playlists, likes) = request.await.unwrap_or_else(|_| {
                (
                    Err("歌单列表请求任务失败".into()),
                    Err("喜欢状态请求任务失败".into()),
                )
            });
            let _ = this.update(cx, |library, cx| {
                if library.user_id != user_id || library.generation != generation {
                    return;
                }
                library.loading = false;
                match playlists {
                    Ok(playlists) => library.playlists = playlists,
                    Err(message) => library.error = Some(message),
                }
                match likes {
                    Ok(ids) => library.liked_song_ids = ids.into_iter().collect(),
                    Err(message) => library.likes_error = Some(message),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn favorite_playlist(&self) -> Option<&Playlist> {
        self.playlists
            .iter()
            .find(|playlist| playlist.special_type == 5)
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.load(self.user_id, cx);
    }
}
