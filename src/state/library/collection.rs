//! 收藏请求由音乐库持有，关闭弹窗不影响请求和本地状态更新。

use gpui::{Context, EventEmitter, ReadGlobal};

use super::MusicLibrary;
use crate::api::MusicApi;
use crate::models::{Playlist, Song};
use crate::state::resource::request;

#[derive(Clone)]
pub struct SongsCollected {
    /// 新建歌单失败时为 0。
    pub playlist_id: u64,
    pub result: Result<(), String>,
}

impl EventEmitter<SongsCollected> for MusicLibrary {}

impl MusicLibrary {
    pub fn collect_songs(&mut self, playlist_id: u64, songs: Vec<Song>, cx: &mut Context<Self>) {
        if songs.is_empty() {
            self.report_collection(playlist_id, Err("没有要收藏的歌曲".into()), cx);
            return;
        }
        if self.user_id == 0 {
            self.report_collection(playlist_id, Err("请先登录后再收藏".into()), cx);
            return;
        }
        let Some(playlist) = self
            .playlists()
            .iter()
            .find(|playlist| playlist.id == playlist_id)
        else {
            self.report_collection(
                playlist_id,
                Err("歌单已不在列表里，请重新打开收藏窗口".into()),
                cx,
            );
            return;
        };
        // 喜欢列表只认 like 接口，不能使用普通歌单的歌曲接口。
        if playlist.special_type == 5 {
            self.like_songs(songs, playlist_id, cx);
        } else {
            self.add_songs(playlist_id, songs, cx);
        }
    }

    pub fn create_playlist_and_collect(
        &mut self,
        name: String,
        songs: Vec<Song>,
        cx: &mut Context<Self>,
    ) {
        let name = name.trim().to_owned();
        if self.user_id == 0 {
            self.report_collection(0, Err("请先登录后再收藏".into()), cx);
            return;
        }
        if name.is_empty() {
            self.report_collection(0, Err("歌单名不能为空".into()), cx);
            return;
        }
        if songs.is_empty() {
            self.report_collection(0, Err("没有要收藏的歌曲".into()), cx);
            return;
        }
        let song_ids = songs.iter().map(|song| song.id).collect();
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        request(
            cx,
            "create playlist and collect",
            async move {
                let playlist = MusicApi::create_playlist(&client, &name).await?;
                let (added, error) =
                    MusicApi::add_songs_to_playlist(client, playlist.id, song_ids).await;
                Ok((playlist, added, error))
            },
            move |library, outcome, cx| {
                if library.generation != generation {
                    return;
                }
                match outcome {
                    Ok((mut playlist, added, error)) => {
                        playlist.track_count = added;
                        let playlist_id = playlist.id;
                        library.insert_playlist(playlist);
                        library.report_collection(playlist_id, error.map_or(Ok(()), Err), cx);
                    }
                    Err(message) => library.report_collection(0, Err(message), cx),
                }
            },
        );
    }

    fn add_songs(&mut self, playlist_id: u64, songs: Vec<Song>, cx: &mut Context<Self>) {
        let song_ids = songs.iter().map(|song| song.id).collect();
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        request(
            cx,
            "collect songs",
            async move { Ok(MusicApi::add_songs_to_playlist(client, playlist_id, song_ids).await) },
            move |library, result, cx| {
                let (added, error) = result.unwrap_or_else(|error| (0, Some(error)));
                if library.generation != generation {
                    return;
                }
                library.playlists.cancel();
                if let Some(playlist) = library
                    .playlists
                    .value_mut()
                    .into_iter()
                    .flatten()
                    .find(|playlist| playlist.id == playlist_id)
                {
                    // 分批请求可能部分成功，失败也要记下已经添加的数量。
                    playlist.track_count += added;
                }
                library.report_collection(playlist_id, error.map_or(Ok(()), Err), cx);
            },
        );
    }

    fn like_songs(&mut self, songs: Vec<Song>, playlist_id: u64, cx: &mut Context<Self>) {
        if self.likes.value().is_none() {
            self.report_collection(playlist_id, Err("喜欢列表尚未加载，请稍后重试".into()), cx);
            return;
        }
        let pending: Vec<Song> = songs
            .into_iter()
            .filter(|song| !self.is_liked(song.id) && !self.pending_likes.contains(&song.id))
            .collect();
        if pending.is_empty() {
            self.report_collection(playlist_id, Ok(()), cx);
            return;
        }
        for song in &pending {
            self.pending_likes.insert(song.id);
            self.set_liked_locally(song, true);
        }
        let song_ids = pending.iter().map(|song| song.id).collect();
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        request(
            cx,
            "like songs",
            async move { Ok(MusicApi::set_songs_liked(client, song_ids, true).await) },
            move |library, result, cx| {
                let (succeeded, error) = result.unwrap_or_else(|error| (Vec::new(), Some(error)));
                if library.generation != generation {
                    return;
                }
                for song in &pending {
                    library.pending_likes.remove(&song.id);
                }
                if error.is_some() {
                    for song in pending.iter().filter(|song| !succeeded.contains(&song.id)) {
                        library.set_liked_locally(song, false);
                    }
                }
                library.report_collection(playlist_id, error.map_or(Ok(()), Err), cx);
            },
        );
        cx.notify();
    }

    // 校验失败也必须通知结果，否则弹窗会一直停在提交状态。
    fn report_collection(
        &self,
        playlist_id: u64,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = &result {
            log::warn!("collect songs: {error}");
        }
        cx.emit(SongsCollected {
            playlist_id,
            result,
        });
        cx.notify();
    }

    fn insert_playlist(&mut self, playlist: Playlist) {
        let index = self
            .playlists()
            .iter()
            .position(|existing| existing.special_type == 5)
            .map_or(0, |index| index + 1);
        self.playlists.cancel();
        if let Some(playlists) = self.playlists.value_mut() {
            playlists.insert(index, playlist);
        } else {
            self.playlists.set(vec![playlist]);
        }
    }
}
