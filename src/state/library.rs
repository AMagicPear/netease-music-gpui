use std::collections::{HashMap, HashSet};

use gpui::{Context, Entity, ReadGlobal, Subscription};

use super::account::AccountState;
use crate::api::MusicApi;
use crate::models::{Playlist, Song};

#[derive(Default)]
pub struct MusicLibrary {
    pub playlists: Vec<Playlist>,
    pub liked_song_ids: HashSet<u64>,
    /// 本次会话里点亮过的歌曲。喜欢的音乐歌单靠它不用重新拉取就能把新歌插进列表。
    pub liked_song_cache: HashMap<u64, Song>,
    pub user_id: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub likes_error: Option<String>,
    generation: u64,
    _user_subscription: Option<Subscription>,
}

impl MusicLibrary {
    pub fn new(user: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&user, |this, user, cx| {
            this.load(user.read(cx).profile.user_id, cx);
        });
        let mut this = Self {
            loading: true,
            _user_subscription: Some(subscription),
            ..Default::default()
        };
        let user_id = user.read(cx).profile.user_id;
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
            self.liked_song_cache.clear();
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

    /// 喜欢 / 取消喜欢。先改本地状态让界面立即响应，请求失败再回滚。
    ///
    /// 需要整个 [`Song`] 而不只是 id：点亮时把它存进 [`Self::liked_song_cache`]，
    /// 喜欢的音乐歌单就能立刻把这首歌插进去，不必等重新拉取。
    pub fn toggle_like(&mut self, song: Song, cx: &mut Context<Self>) {
        let song_id = song.id;
        if song_id == 0 {
            return;
        }
        let liked = !self.liked_song_ids.contains(&song_id);
        self.set_liked_locally(song_id, liked);
        if liked {
            self.liked_song_cache.insert(song_id, song.clone());
        } else {
            self.liked_song_cache.remove(&song_id);
        }
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let request = api
            .runtime
            .spawn(MusicApi::set_song_liked(client, song_id, liked));
        cx.spawn(async move |this, cx| {
            if let Ok(Err(message)) = request.await {
                eprintln!("{message}");
                let _ = this.update(cx, |library, cx| {
                    library.set_liked_locally(song_id, !liked);
                    // 回滚时缓存也要跟着退回：重新喜欢则补回歌曲，取消喜欢则清掉。
                    if liked {
                        library.liked_song_cache.remove(&song_id);
                    } else {
                        library.liked_song_cache.insert(song_id, song);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn set_liked_locally(&mut self, song_id: u64, liked: bool) {
        if liked {
            self.liked_song_ids.insert(song_id);
        } else {
            self.liked_song_ids.remove(&song_id);
        }
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.load(self.user_id, cx);
    }
}
