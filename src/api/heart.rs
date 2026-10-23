//! 心动接口给出红心歌曲与推荐歌曲的混合顺序，客户端不再洗牌。
use std::collections::{HashMap, HashSet};

use ncm_api_rs::Query;
use rand::seq::IndexedRandom;
use serde::Deserialize;

use super::{ApiClient, MusicApi, song::parse_songs};
use crate::models::Song;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeartEntry {
    id: u64,
    song_info: Option<Song>,
}

fn parse_entries(body: &serde_json::Value) -> Result<Vec<HeartEntry>, String> {
    let entries: Vec<HeartEntry> = serde_json::from_value(body["data"].clone())
        .map_err(|error| format!("心动推荐格式无效：{error}"))?;
    if entries.iter().any(|entry| {
        entry.id == 0
            || entry
                .song_info
                .as_ref()
                .is_some_and(|song| song.id != entry.id)
    }) {
        return Err("心动推荐歌曲 ID 无效".into());
    }
    Ok(entries)
}

fn resolve_entries(entries: Vec<HeartEntry>, details: Vec<Song>) -> Vec<Song> {
    let mut details: HashMap<_, _> = details.into_iter().map(|song| (song.id, song)).collect();
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter_map(|entry| {
            let song = entry.song_info.or_else(|| details.remove(&entry.id))?;
            seen.insert(song.id).then_some(song)
        })
        .collect()
}

impl MusicApi {
    pub async fn heart_playlist(client: &ApiClient, user_id: u64) -> Result<u64, String> {
        if user_id == 0 {
            return Err("请先登录，再开启心动模式".into());
        }
        Self::user_playlists(client, user_id)
            .await?
            .into_iter()
            .find(|playlist| playlist.special_type == 5 && playlist.id != 0)
            .map(|playlist| playlist.id)
            .ok_or_else(|| "没有找到我喜欢的音乐歌单".into())
    }

    pub async fn heart_songs(client: ApiClient, playlist_id: u64) -> Result<Vec<Song>, String> {
        if playlist_id == 0 {
            return Err("心动歌单 ID 无效".into());
        }
        // 每批重新取红心种子：取消喜欢后不继续使用旧种子，也不依赖未明确的 count 分页语义。
        let playlist = Self::playlist_meta(client.clone(), playlist_id).await?;
        let seed_id = playlist
            .track_ids
            .choose(&mut rand::rng())
            .map(|track| track.id)
            .filter(|id| *id != 0)
            .ok_or_else(|| "先收藏几首喜欢的音乐，再开启心动模式吧".to_string())?;
        let response = Self::request(
            client.playmode_intelligence_list(
                &Query::new()
                    .param("id", &seed_id.to_string())
                    .param("pid", &playlist_id.to_string()),
            ),
        )
        .await?;
        let entries = parse_entries(&response.body)?;
        // 实测同一响应可能混有 songInfo: null，按 ID 批量补详情，不能丢掉整批推荐。
        let missing: Vec<_> = entries
            .iter()
            .filter(|entry| entry.song_info.is_none())
            .map(|entry| entry.id)
            .collect();
        let mut details = Vec::new();
        for chunk in missing.chunks(200) {
            let ids = chunk
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let mut response =
                Self::request(client.song_detail(&Query::new().param("ids", &ids))).await?;
            details.extend(
                parse_songs(&mut response.body)
                    .map_err(|error| format!("心动歌曲详情格式无效：{error}"))?,
            );
        }
        let songs = resolve_entries(entries, details);
        if songs.is_empty() {
            return Err("暂时没有心动推荐，请稍后重试".into());
        }
        Ok(songs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_missing_details_by_id_preserving_order_and_privileges() {
        let body = json!({"data": [
            {"id": 2, "songInfo": null},
            {"id": 1, "songInfo": {"id": 1, "name": "one", "ar": [], "al": {"id": 1}, "dt": 1000,
                "privilege": {"id": 1, "pl": 320000}}},
            {"id": 2}, {"id": 3, "songInfo": null}
        ]});
        let songs = resolve_entries(
            parse_entries(&body).unwrap(),
            vec![Song {
                id: 2,
                ..Default::default()
            }],
        );
        assert_eq!(songs.iter().map(|s| s.id).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(songs[1].privilege.as_ref().unwrap().pl, Some(320000));
    }

    #[test]
    fn later_valid_duplicate_survives_a_missing_first_detail() {
        let entries = vec![
            HeartEntry {
                id: 1,
                song_info: None,
            },
            HeartEntry {
                id: 1,
                song_info: Some(Song {
                    id: 1,
                    ..Default::default()
                }),
            },
        ];
        assert_eq!(resolve_entries(entries, vec![]).len(), 1);
    }

    #[test]
    fn rejects_invalid_shapes_and_mismatched_song_ids() {
        for body in [
            json!({}),
            json!({"data": null}),
            json!({"data": [{"id": 0}]}),
            json!({"data": [{"id": 1, "songInfo": {"id": 2, "name": "x", "ar": [], "al": {"id": 1}, "dt": 1}}]}),
        ] {
            assert!(parse_entries(&body).is_err());
        }
        assert!(parse_entries(&json!({"data": []})).unwrap().is_empty());
    }
}
