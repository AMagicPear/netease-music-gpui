use std::collections::HashMap;

use ncm_api_rs::Query;

use super::{ApiClient, MusicApi, song::parse_songs};
use crate::models::{Playlist, Song, SongComments, TrackId, parse_comment_page};

// 服务端限制单次添加数量，队列收藏必须分批提交。
const COLLECT_BATCH: usize = 100;

impl MusicApi {
    pub async fn user_playlists(client: &ApiClient, user_id: u64) -> Result<Vec<Playlist>, String> {
        let mut playlists = Vec::new();
        loop {
            let mut response = Self::request(
                client.user_playlist(
                    &Query::new()
                        .param("uid", &user_id.to_string())
                        .param("limit", "100")
                        .param("offset", &playlists.len().to_string()),
                ),
            )
            .await?;
            let page: Vec<Playlist> = serde_json::from_value(response.body["playlist"].take())
                .map_err(|error| format!("歌单列表格式无效：{error}"))?;
            let more = response.body["more"].as_bool() == Some(true);
            let empty = page.is_empty();
            playlists.extend(page);
            if !more || empty {
                break;
            }
        }

        Ok(playlists)
    }

    pub async fn liked_song_ids(client: &ApiClient, user_id: u64) -> Result<Vec<u64>, String> {
        let like_query = Query::new().param("uid", &user_id.to_string());
        let mut likes = Self::request(client.likelist(&like_query)).await?;
        serde_json::from_value(likes.body["ids"].take())
            .map_err(|_| "喜欢的歌曲列表格式无效".to_string())
    }

    /// 与歌曲评论共用分页语义和解析。
    pub async fn playlist_comments(
        client: ApiClient,
        playlist_id: u64,
        offset: usize,
    ) -> Result<SongComments, String> {
        let response = Self::request(
            client.comment_playlist(
                &Query::new()
                    .param("id", &playlist_id.to_string())
                    .param("limit", "20")
                    .param("offset", &offset.to_string()),
            ),
        )
        .await?;
        parse_comment_page(response.body, offset)
    }

    /// 分批提交，返回成功数量与首个错误；失败时前面的批次可能已经生效。
    pub async fn add_songs_to_playlist(
        client: ApiClient,
        playlist_id: u64,
        song_ids: Vec<u64>,
    ) -> (usize, Option<String>) {
        let mut added = 0;
        for chunk in song_ids.chunks(COLLECT_BATCH) {
            let tracks = chunk
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let result = Self::request(
                client.playlist_tracks(
                    &Query::new()
                        .param("op", "add")
                        .param("pid", &playlist_id.to_string())
                        .param("tracks", &tracks),
                ),
            )
            .await;
            match result {
                Ok(_) => added += chunk.len(),
                Err(message) => return (added, Some(message)),
            }
        }
        (added, None)
    }

    /// 响应可能只返回顶层 id，摘要缺失时留待索引刷新补齐。
    pub async fn create_playlist(client: &ApiClient, name: &str) -> Result<Playlist, String> {
        let mut response =
            Self::request(client.playlist_create(&Query::new().param("name", name))).await?;
        if let Ok(playlist) = serde_json::from_value::<Playlist>(response.body["playlist"].take())
            && playlist.id != 0
        {
            return Ok(playlist);
        }
        let id = response.body["id"]
            .as_u64()
            .filter(|id| *id != 0)
            .ok_or_else(|| "新建歌单没有返回歌单 id".to_string())?;
        Ok(Playlist::empty(id, name.to_owned()))
    }

    pub async fn search_playlists(
        client: ApiClient,
        keywords: &str,
    ) -> Result<Vec<Playlist>, String> {
        let mut response = Self::request(
            client.cloudsearch(
                &Query::new()
                    .param("keywords", keywords)
                    .param("type", "1000")
                    .param("limit", "50"),
            ),
        )
        .await?;
        let playlists: Vec<Playlist> =
            serde_json::from_value(response.body["result"]["playlists"].take())
                .map_err(|error| format!("歌单搜索结果格式无效：{error}"))?;
        Ok(playlists)
    }

    /// 详情先返回，歌曲分批加载，页头无需等待全部歌曲。
    pub async fn playlist_meta(client: ApiClient, playlist_id: u64) -> Result<Playlist, String> {
        let mut response = Self::request(
            client.playlist_detail(&Query::new().param("id", &playlist_id.to_string())),
        )
        .await?;
        serde_json::from_value(response.body["playlist"].take())
            .map_err(|error| format!("歌单详情格式无效：{error}"))
    }

    pub async fn playlist_songs(
        client: ApiClient,
        playlist: Playlist,
    ) -> Result<Vec<Song>, String> {
        let mut songs = Vec::new();
        for tracks in playlist.track_ids.chunks(200) {
            let ids = tracks
                .iter()
                .map(|track| track.id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let mut response =
                Self::request(client.song_detail(&Query::new().param("ids", &ids))).await?;
            let batch = parse_songs(&mut response.body)
                .map_err(|error| format!("歌曲详情格式无效：{error}"))?;
            songs.extend(ordered_playlist_songs(tracks, batch));
        }
        Ok(songs)
    }

    pub async fn playlist(
        client: ApiClient,
        playlist_id: u64,
    ) -> Result<(Playlist, Vec<Song>), String> {
        let playlist = Self::playlist_meta(client.clone(), playlist_id).await?;
        let songs = Self::playlist_songs(client, playlist.clone()).await?;
        Ok((playlist, songs))
    }
}

/// song_detail 的响应顺序不作为歌单顺序；缺失和额外的歌曲不补造。
fn ordered_playlist_songs(tracks: &[TrackId], songs: Vec<Song>) -> Vec<Song> {
    let mut by_id: HashMap<_, _> = songs.into_iter().map(|song| (song.id, song)).collect();
    tracks
        .iter()
        .filter_map(|track| by_id.remove(&track.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_order_follows_track_ids_and_skips_unavailable_songs() {
        let tracks: Vec<_> = [3, 2, 1].into_iter().map(|id| TrackId { id }).collect();
        let songs = [5, 1, 3]
            .into_iter()
            .map(|id| Song {
                id,
                ..Default::default()
            })
            .collect();
        let ordered = ordered_playlist_songs(&tracks, songs);
        assert_eq!(
            ordered.iter().map(|song| song.id).collect::<Vec<_>>(),
            [3, 1]
        );
    }
}
