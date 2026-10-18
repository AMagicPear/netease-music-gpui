use std::collections::{HashMap, HashSet};

use gpui::{Context, Entity, EventEmitter, ReadGlobal, Subscription};

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

/// 「把歌曲收藏进歌单」的结果（消息层事件）。
///
/// 请求在 [`MusicLibrary`] 里发，收藏弹窗只是订阅结果：弹窗随时可能被关掉，
/// 而请求和本地记账（歌单歌曲数、喜欢集合）必须活着，所以不在窗口里 `spawn`。
#[derive(Clone)]
pub struct SongsCollected {
    /// 目标歌单；新建歌单失败时是 0（没有歌单可指）。
    pub playlist_id: u64,
    pub result: Result<(), String>,
}

impl EventEmitter<SongsCollected> for MusicLibrary {}

impl MusicLibrary {
    pub fn new(user: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&user, |this, user, cx| {
            this.load(user.read(cx).profile.user_id, cx);
        });
        let mut this = Self {
            loading: user.read(cx).loading,
            _user_subscription: Some(subscription),
            ..Default::default()
        };
        let user_id = user.read(cx).profile.user_id;
        if user_id != 0 || !user.read(cx).loading {
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
            self.error = Some("请点击顶部头像扫码登录".into());
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

    /// 喜欢 / 取消喜欢（翻转当前状态）。
    pub fn toggle_like(&mut self, song: Song, cx: &mut Context<Self>) {
        let liked = !self.liked_song_ids.contains(&song.id);
        self.set_liked(song, liked, cx);
    }

    /// 设成指定的喜欢状态。先改本地状态让界面立即响应，请求失败再回滚。
    ///
    /// 需要整个 [`Song`] 而不只是 id：点亮时把它存进 [`Self::liked_song_cache`]，
    /// 喜欢的音乐歌单就能立刻把这首歌插进去，不必等重新拉取。
    ///
    /// 「收藏到喜欢」是「确保点亮」而不是翻转：已经在喜欢列表里的歌再收藏一次
    /// 不该把红心取消掉，所以这里先比状态，相同就什么都不做。
    pub fn set_liked(&mut self, song: Song, liked: bool, cx: &mut Context<Self>) {
        let song_id = song.id;
        if song_id == 0 || self.user_id == 0 {
            return;
        }
        if self.liked_song_ids.contains(&song_id) == liked {
            return;
        }
        self.set_liked_locally(song_id, liked);
        if liked {
            self.liked_song_cache.insert(song_id, song.clone());
        } else {
            self.liked_song_cache.remove(&song_id);
        }
        let api = MusicApi::global(cx);
        let generation = self.generation;
        let client = api.client.clone();
        let request = api
            .runtime
            .spawn(MusicApi::set_song_liked(client, song_id, liked));
        cx.spawn(async move |this, cx| {
            if let Ok(Err(message)) = request.await {
                eprintln!("{message}");
                let _ = this.update(cx, |library, cx| {
                    if library.generation != generation {
                        return;
                    }
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

    /// 把歌曲收藏进指定歌单，结果通过 [`SongsCollected`] 广播。
    ///
    /// 「我喜欢的音乐」（`special_type == 5`）不能走歌单歌曲接口：这个歌单的内容
    /// 就是账号的喜欢列表，服务端只认 like 接口。分流放在这里，收藏弹窗按歌单 id
    /// 一视同仁地提交即可。
    pub fn collect_songs(&mut self, playlist_id: u64, songs: Vec<Song>, cx: &mut Context<Self>) {
        if songs.is_empty() {
            // 空批次不该发出请求，但也不能一声不响：弹窗要据此解除「提交中」。
            self.report_failure(playlist_id, "没有要收藏的歌曲", cx);
            return;
        }
        if self.user_id == 0 {
            self.report_failure(playlist_id, "请先登录后再收藏", cx);
            return;
        }
        let Some(is_liked_playlist) = self
            .playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.special_type == 5)
        else {
            self.report_failure(playlist_id, "歌单已不在列表里，请重新打开收藏窗口", cx);
            return;
        };
        if is_liked_playlist {
            self.like_songs(songs, playlist_id, cx);
        } else {
            self.add_songs(playlist_id, songs, cx);
        }
    }

    /// 新建歌单并把歌曲直接放进去：两步在同一个任务里串行，歌单还没建好就发收藏
    /// 请求会被服务端拒掉。
    pub fn create_playlist_and_collect(
        &mut self,
        name: String,
        songs: Vec<Song>,
        cx: &mut Context<Self>,
    ) {
        let name = name.trim().to_owned();
        // 提交不出去也要回一个结果，否则弹窗会一直停在「提交中」。
        if self.user_id == 0 {
            self.report_failure(0, "请先登录后再收藏", cx);
            return;
        }
        if name.is_empty() {
            self.report_failure(0, "歌单名不能为空", cx);
            return;
        }
        if songs.is_empty() {
            self.report_failure(0, "没有要收藏的歌曲", cx);
            return;
        }
        let song_ids: Vec<u64> = songs.iter().map(|song| song.id).collect();
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let request = api.runtime.spawn(async move {
            let playlist = MusicApi::create_playlist(&client, &name).await?;
            let (added, error) =
                MusicApi::add_songs_to_playlist(client, playlist.id, song_ids).await;
            Ok::<_, String>((playlist, added, error))
        });
        cx.spawn(async move |this, cx| {
            let outcome = request
                .await
                .unwrap_or_else(|_| Err("新建歌单请求任务失败".into()));
            let _ = this.update(cx, |library, cx| {
                if library.generation != generation {
                    return;
                }
                let event = match outcome {
                    Ok((mut playlist, added, error)) => {
                        // 新歌单的歌曲数就是刚放进去的这批。
                        playlist.track_count = added;
                        library.insert_playlist(playlist.clone());
                        SongsCollected {
                            playlist_id: playlist.id,
                            result: error.map_or(Ok(()), Err),
                        }
                    }
                    Err(message) => SongsCollected {
                        playlist_id: 0,
                        result: Err(message),
                    },
                };
                cx.emit(event);
                cx.notify();
            });
        })
        .detach();
    }

    /// 走歌单歌曲接口收藏。接口收下就说明服务端已生效，本地歌曲数直接跟上，
    /// 不等下一次刷新列表——否则收藏弹窗关掉后侧栏还停在旧数字。
    fn add_songs(&mut self, playlist_id: u64, songs: Vec<Song>, cx: &mut Context<Self>) {
        let song_ids: Vec<u64> = songs.iter().map(|song| song.id).collect();
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let request = api.runtime.spawn(MusicApi::add_songs_to_playlist(
            client,
            playlist_id,
            song_ids,
        ));
        cx.spawn(async move |this, cx| {
            let (added, error) = request
                .await
                .unwrap_or((0, Some("收藏请求任务失败".to_string())));
            let _ = this.update(cx, |library, cx| {
                if library.generation != generation {
                    return;
                }
                if let Some(playlist) = library
                    .playlists
                    .iter_mut()
                    .find(|playlist| playlist.id == playlist_id)
                {
                    playlist.track_count += added;
                }
                cx.emit(SongsCollected {
                    playlist_id,
                    result: error.map_or(Ok(()), Err),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// 收藏进「我喜欢的音乐」：逐首点亮，失败只回滚没生效的那几首。
    fn like_songs(&mut self, songs: Vec<Song>, playlist_id: u64, cx: &mut Context<Self>) {
        // 已经点亮的不再发请求，接口对重复喜欢也会报错。
        let pending: Vec<Song> = songs
            .into_iter()
            .filter(|song| !self.liked_song_ids.contains(&song.id))
            .collect();
        if pending.is_empty() {
            cx.emit(SongsCollected {
                playlist_id,
                result: Ok(()),
            });
            cx.notify();
            return;
        }
        for song in &pending {
            self.set_liked_locally(song.id, true);
            self.liked_song_cache.insert(song.id, song.clone());
        }
        let song_ids: Vec<u64> = pending.iter().map(|song| song.id).collect();
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let request = api
            .runtime
            .spawn(MusicApi::set_songs_liked(client, song_ids.clone(), true));
        cx.spawn(async move |this, cx| {
            let (succeeded, error) = request
                .await
                .unwrap_or((Vec::new(), Some("喜欢请求任务失败".to_string())));
            let _ = this.update(cx, |library, cx| {
                if library.generation != generation {
                    return;
                }
                if error.is_some() {
                    for song_id in song_ids
                        .iter()
                        .filter(|song_id| !succeeded.contains(song_id))
                    {
                        library.set_liked_locally(*song_id, false);
                        library.liked_song_cache.remove(song_id);
                    }
                }
                cx.emit(SongsCollected {
                    playlist_id,
                    result: error.map_or(Ok(()), Err),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 提交不出去（未登录 / 歌单已不在列表 / 空批次）时也回一个失败结果：
    /// 收藏弹窗据此解除「提交中」——否则它会一直等一个永远不会来的请求。
    fn report_failure(&mut self, playlist_id: u64, message: &str, cx: &mut Context<Self>) {
        cx.emit(SongsCollected {
            playlist_id,
            result: Err(message.to_owned()),
        });
        cx.notify();
    }

    /// 新建的歌单紧跟在「我喜欢的音乐」后面，和官方列表顺序一致。
    fn insert_playlist(&mut self, playlist: Playlist) {
        let index = self
            .playlists
            .iter()
            .position(|existing| existing.special_type == 5)
            .map_or(0, |index| index + 1);
        self.playlists.insert(index, playlist);
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.load(self.user_id, cx);
    }
}
