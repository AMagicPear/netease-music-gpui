use std::collections::{HashMap, HashSet};

use gpui::{Context, ReadGlobal};

use crate::api::MusicApi;
use crate::models::{Playlist, Song};

/// 当前打开的歌单数据；排序、标签和 hover 留在页面 View 中。
#[derive(Default)]
pub struct PlaylistDetail {
    pub id: Option<u64>,
    pub playlist: Option<Playlist>,
    pub songs: Vec<Song>,
    pub loading: bool,
    pub error: Option<String>,
    generation: u64,
    request: Option<tokio::task::AbortHandle>,
}

impl PlaylistDetail {
    pub fn open(&mut self, id: Option<u64>, summary: Option<Playlist>, cx: &mut Context<Self>) {
        if self.id == id && (self.loading || self.error.is_none()) {
            return;
        }
        self.clear();
        self.id = id;
        self.playlist = summary;
        let Some(id) = id else {
            cx.notify();
            return;
        };
        self.loading = true;
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::playlist(api.client.clone(), id));
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("歌单请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.finish(id, generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 「喜欢的音乐」（`special_type == 5`）的歌曲集合就是账号的喜欢列表，
    /// 所以点亮 / 取消喜欢后要把已加载的 songs 同步成同一集合：
    /// 取消的去掉，新点亮的从 `cache`（音乐库本次会话缓存过的歌曲）补到队首
    /// ——服务器歌单也是新喜欢的在前。
    /// 返回是否有变化，调用方据此决定要不要重绘。
    pub fn reconcile_liked(&mut self, liked: &HashSet<u64>, cache: &HashMap<u64, Song>) -> bool {
        let is_liked_playlist = self
            .playlist
            .as_ref()
            .is_some_and(|playlist| playlist.special_type == 5);
        if !is_liked_playlist {
            return false;
        }
        let before = self.songs.len();
        self.songs.retain(|song| liked.contains(&song.id));
        let mut changed = self.songs.len() != before;
        // 缓存里可能有同时点亮的多首；HashMap 顺序不定，按 id 排一下，
        // 免得画面每次重绘都换一个插入顺序。
        let mut added: Vec<&Song> = cache
            .iter()
            .filter(|(id, _)| liked.contains(id))
            .map(|(_, song)| song)
            .filter(|song| !self.songs.iter().any(|existing| existing.id == song.id))
            .collect();
        added.sort_by_key(|song| song.id);
        for song in added {
            self.songs.insert(0, song.clone());
            changed = true;
        }
        if changed {
            // 表头的歌曲数取自 track_count，增删了行要一起更新。
            if let Some(playlist) = self.playlist.as_mut() {
                playlist.track_count = self.songs.len();
            }
        }
        changed
    }

    fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.id = None;
        self.playlist = None;
        self.songs.clear();
        self.loading = false;
        self.error = None;
    }

    fn finish(
        &mut self,
        id: u64,
        generation: u64,
        result: Result<(Playlist, Vec<Song>), String>,
    ) -> bool {
        // 取消网络任务之外，再拦截已经送回界面的旧结果。
        if self.id != Some(id) || self.generation != generation {
            return false;
        }
        self.request = None;
        self.loading = false;
        match result {
            Ok((playlist, songs)) => {
                self.playlist = Some(playlist);
                self.songs = songs;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
        true
    }
}

impl Drop for PlaylistDetail {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_playlists_discards_old_results_and_clears_rows() {
        let mut detail = PlaylistDetail::default();
        detail.id = Some(2);
        detail.loading = true;
        detail.songs = vec![Song::default()];
        detail.generation = 3;
        assert!(!detail.finish(1, 2, Err("旧歌单失败".into())));
        assert!(!detail.finish(2, 1, Err("切回同一个歌单的旧结果".into())));
        assert!(detail.loading && detail.error.is_none());
        assert!(detail.finish(2, 3, Err("当前歌单失败".into())));
        assert!(!detail.loading);
        assert_eq!(detail.error.as_deref(), Some("当前歌单失败"));
        detail.clear();
        assert!(detail.id.is_none() && detail.songs.is_empty() && detail.error.is_none());
        assert_eq!(detail.generation, 4);
    }

    fn playlist_with_special_type(special_type: u32) -> Playlist {
        serde_json::from_value(serde_json::json!({
            "id": 1, "name": "歌单",
            "creator": {"userId": 1, "nickname": "我", "avatarUrl": ""},
            "createTime": 0, "playCount": 0, "trackCount": 2, "subscribedCount": 0,
            "specialType": special_type
        }))
        .unwrap()
    }

    fn song(id: u64) -> Song {
        Song {
            id,
            ..Default::default()
        }
    }

    #[test]
    fn liked_playlist_drops_unliked_songs_and_updates_count() {
        let mut detail = PlaylistDetail::default();
        detail.playlist = Some(playlist_with_special_type(5));
        detail.songs = vec![song(1), song(2)];
        let liked: HashSet<u64> = [2].into_iter().collect();
        assert!(detail.reconcile_liked(&liked, &HashMap::new()));
        assert_eq!(
            detail.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(detail.playlist.as_ref().unwrap().track_count, 1);
        // 集合没再变时返回 false，避免每次 library 通知都白重绘。
        assert!(!detail.reconcile_liked(&liked, &HashMap::new()));
    }

    #[test]
    fn liked_playlist_inserts_newly_liked_song_from_cache() {
        let mut detail = PlaylistDetail::default();
        detail.playlist = Some(playlist_with_special_type(5));
        detail.songs = vec![song(2)];
        let liked: HashSet<u64> = [1, 2].into_iter().collect();
        let cache = HashMap::from([(1, song(1))]);
        assert!(detail.reconcile_liked(&liked, &cache));
        // 新喜欢的是插到队首（服务器歌单也是新喜欢的在前）。
        assert_eq!(
            detail.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        // 已经在列表里的不会被重复插入。
        assert!(!detail.reconcile_liked(&liked, &cache));
        assert_eq!(detail.songs.len(), 2);
    }

    #[test]
    fn ordinary_playlists_keep_songs_when_unliked() {
        let mut detail = PlaylistDetail::default();
        detail.playlist = Some(playlist_with_special_type(0));
        detail.songs = vec![song(1)];
        assert!(!detail.reconcile_liked(&HashSet::new(), &HashMap::from([(1, song(1))])));
        assert_eq!(detail.songs.len(), 1);
    }
}
