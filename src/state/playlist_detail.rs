use super::resource::{Resource, request};
use crate::{
    api::MusicApi,
    models::{Playlist, Song},
};
use gpui::{Context, ReadGlobal};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct PlaylistDetail {
    pub id: Option<u64>,
    pub playlist: Option<Playlist>,
    pub songs: Resource<Vec<Song>>,
}

impl PlaylistDetail {
    pub fn songs(&self) -> &[Song] {
        self.songs.value().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn open(&mut self, id: Option<u64>, summary: Option<Playlist>, cx: &mut Context<Self>) {
        if self.id == id {
            return;
        }
        self.songs.reset();
        self.playlist = summary;
        self.id = id;
        self.reload(cx);
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id else {
            cx.notify();
            return;
        };
        if self.songs.is_loading() {
            return;
        }
        let generation = self.songs.begin();
        let client = MusicApi::global(cx).client.clone();
        self.songs.track(request(
            cx,
            "playlist metadata",
            MusicApi::playlist_meta(client.clone(), id),
            move |this, result, cx| {
                if !this.songs.is_current(generation) {
                    return;
                }
                match result {
                    Ok(playlist) => {
                        this.playlist = Some(playlist.clone());
                        this.songs.track(request(
                            cx,
                            "playlist songs",
                            MusicApi::playlist_songs(client, playlist),
                            move |this, result, cx| {
                                if this.songs.apply(generation, result) {
                                    cx.notify();
                                }
                            },
                        ));
                    }
                    Err(error) => {
                        this.songs.apply(generation, Err(error));
                    }
                }
                cx.notify();
            },
        ));
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
        let Some(songs) = self.songs.value_mut() else {
            return false;
        };
        let before = songs.len();
        songs.retain(|song| liked.contains(&song.id));
        let mut changed = songs.len() != before;
        // 缓存里可能有同时点亮的多首；HashMap 顺序不定，按 id 排一下，
        // 免得画面每次重绘都换一个插入顺序。
        let mut added: Vec<&Song> = cache
            .iter()
            .filter(|(id, _)| liked.contains(id))
            .map(|(_, song)| song)
            .filter(|song| !songs.iter().any(|existing| existing.id == song.id))
            .collect();
        added.sort_by_key(|song| song.id);
        for song in added {
            songs.insert(0, song.clone());
            changed = true;
        }
        if changed {
            // 表头的歌曲数取自 track_count，增删了行要一起更新。
            if let Some(playlist) = self.playlist.as_mut() {
                playlist.track_count = songs.len();
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut detail = PlaylistDetail {
            playlist: Some(playlist_with_special_type(5)),
            ..Default::default()
        };
        detail.songs.set(vec![song(1), song(2)]);
        let liked: HashSet<u64> = [2].into_iter().collect();
        assert!(detail.reconcile_liked(&liked, &HashMap::new()));
        assert_eq!(
            detail
                .songs()
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(detail.playlist.as_ref().unwrap().track_count, 1);
        // 集合没再变时返回 false，避免每次 library 通知都白重绘。
        assert!(!detail.reconcile_liked(&liked, &HashMap::new()));
    }

    #[test]
    fn liked_playlist_inserts_newly_liked_song_from_cache() {
        let mut detail = PlaylistDetail {
            playlist: Some(playlist_with_special_type(5)),
            ..Default::default()
        };
        detail.songs.set(vec![song(2)]);
        let liked: HashSet<u64> = [1, 2].into_iter().collect();
        let cache = HashMap::from([(1, song(1))]);
        assert!(detail.reconcile_liked(&liked, &cache));
        // 新喜欢的是插到队首（服务器歌单也是新喜欢的在前）。
        assert_eq!(
            detail
                .songs()
                .iter()
                .map(|song| song.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        // 已经在列表里的不会被重复插入。
        assert!(!detail.reconcile_liked(&liked, &cache));
        assert_eq!(detail.songs().len(), 2);
    }

    #[test]
    fn ordinary_playlists_keep_songs_when_unliked() {
        let mut detail = PlaylistDetail {
            playlist: Some(playlist_with_special_type(0)),
            ..Default::default()
        };
        detail.songs.set(vec![song(1)]);
        assert!(!detail.reconcile_liked(&HashSet::new(), &HashMap::from([(1, song(1))])));
        assert_eq!(detail.songs().len(), 1);
    }
}
