use std::{collections::HashMap, time::Duration};

use ncm_api_rs::Query;

use super::{ApiClient, MusicApi};
use crate::models::{
    AudioQualityLevel, AudioSourceInfo, LyricLine, Song, SongComments, attach_privileges,
    parse_comment_page,
};

impl MusicApi {
    pub async fn song_counts(client: ApiClient, song_id: u64) -> [Result<u64, String>; 2] {
        let query = Query::new()
            .param("id", &song_id.to_string())
            .param("limit", "0");
        let (likes, comments) = tokio::join!(
            Self::request(client.song_red_count(&query)),
            Self::request(client.comment_music(&query)),
        );
        [
            likes.and_then(|response| {
                response.body["data"]["count"]
                    .as_u64()
                    .ok_or_else(|| "红心计数格式无效".into())
            }),
            comments.and_then(|response| {
                response.body["total"]
                    .as_u64()
                    .ok_or_else(|| "评论计数格式无效".into())
            }),
        ]
    }

    pub async fn song_detail(client: ApiClient, song_id: u64) -> Result<Song, String> {
        let mut response =
            Self::request(client.song_detail(&Query::new().param("ids", &song_id.to_string())))
                .await?;
        let songs = parse_songs(&mut response.body)
            .map_err(|error| format!("歌曲详情格式无效：{error}"))?;
        songs
            .into_iter()
            .next()
            .ok_or_else(|| format!("歌曲 {song_id} 不存在"))
    }

    pub async fn song_lyrics(client: ApiClient, song_id: u64) -> Result<Vec<LyricLine>, String> {
        let response =
            Self::request(client.lyric(&Query::new().param("id", &song_id.to_string()))).await?;
        Ok(song_lyric_lines(&response.body))
    }

    /// offset 仅计算普通评论，热评只合并到第一页。
    pub async fn song_comments(
        client: ApiClient,
        song_id: u64,
        offset: usize,
    ) -> Result<SongComments, String> {
        let response = Self::request(
            client.comment_music(
                &Query::new()
                    .param("id", &song_id.to_string())
                    .param("limit", "20")
                    .param("offset", &offset.to_string()),
            ),
        )
        .await?;
        parse_comment_page(response.body, offset)
    }

    pub async fn song_source(
        client: ApiClient,
        song_id: u64,
        quality: AudioQualityLevel,
    ) -> Result<AudioSourceInfo, String> {
        let response = Self::request(
            client.song_url_v1(
                &Query::new()
                    .param("id", &song_id.to_string())
                    .param("level", quality.api_level()),
            ),
        )
        .await?;
        song_source_info(&response.body, song_id)
    }

    /// 逐首提交，返回已成功的 ID 与首个错误，供调用方回滚未生效部分。
    pub async fn set_songs_liked(
        client: ApiClient,
        song_ids: Vec<u64>,
        liked: bool,
    ) -> (Vec<u64>, Option<String>) {
        let mut succeeded = Vec::with_capacity(song_ids.len());
        for song_id in song_ids {
            match Self::set_song_liked(client.clone(), song_id, liked).await {
                Ok(()) => succeeded.push(song_id),
                Err(message) => return (succeeded, Some(message)),
            }
        }
        (succeeded, None)
    }

    pub async fn set_song_liked(
        client: ApiClient,
        song_id: u64,
        liked: bool,
    ) -> Result<(), String> {
        Self::request(
            client.like(
                &Query::new()
                    .param("id", &song_id.to_string())
                    .param("like", if liked { "true" } else { "false" }),
            ),
        )
        .await?;
        Ok(())
    }

    /// cloudsearch 返回 ar / al / dt 字段，可直接解析为 Song。
    pub async fn search_songs(client: ApiClient, keywords: &str) -> Result<Vec<Song>, String> {
        let mut response = Self::request(
            client.cloudsearch(
                &Query::new()
                    .param("keywords", keywords)
                    .param("type", "1")
                    .param("limit", "30"),
            ),
        )
        .await?;
        parse_songs(&mut response.body["result"])
            .map_err(|error| format!("搜索结果格式无效：{error}"))
    }
}

pub(super) fn parse_songs(body: &mut serde_json::Value) -> Result<Vec<Song>, serde_json::Error> {
    let mut songs = serde_json::from_value::<Vec<Song>>(body["songs"].take())?;
    // 权益缺失不影响歌曲解析，音质判断回退到歌曲自带字段。
    let privileges = serde_json::from_value(body["privileges"].take()).unwrap_or_default();
    attach_privileges(&mut songs, privileges);
    Ok(songs)
}

fn song_lyric_lines(body: &serde_json::Value) -> Vec<LyricLine> {
    if body["nolyric"].as_bool() == Some(true) {
        return Vec::new();
    }
    let translations: HashMap<_, _> =
        parse_lrc(body["tlyric"]["lyric"].as_str().unwrap_or_default())
            .into_iter()
            .filter(|(_, text)| !text.is_empty())
            .collect();
    parse_lrc(body["lrc"]["lyric"].as_str().unwrap_or_default())
        .into_iter()
        .map(|(time, text)| LyricLine {
            time,
            text,
            translation: translations.get(&time).cloned(),
        })
        .collect()
}

fn parse_lrc(lrc: &str) -> Vec<(Duration, String)> {
    let mut lines = Vec::new();
    for line in lrc.lines() {
        let mut text = line.trim().trim_start_matches('\u{feff}');
        let mut times = Vec::new();
        while let Some(tagged) = text.strip_prefix('[') {
            let Some((tag, rest)) = tagged.split_once(']') else {
                break;
            };
            if let Some(time) = lrc_time(tag) {
                times.push(time);
            }
            text = rest;
        }
        // metadata 没有时间标签；空白时间行保留，用于清空当前歌词。
        for time in times {
            lines.push((time, text.trim().to_string()));
        }
    }
    lines.sort_by_key(|(time, _)| *time);
    lines
}

fn lrc_time(tag: &str) -> Option<Duration> {
    let (minutes, seconds) = tag.split_once(':')?;
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    if minutes.is_empty()
        || seconds.is_empty()
        || !minutes.bytes().all(|byte| byte.is_ascii_digit())
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 9
    {
        return None;
    }
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: u64 = seconds.parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let nanos = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u32>().ok()? * 10u32.pow(9 - fraction.len() as u32)
    };
    Some(Duration::new(
        minutes.checked_mul(60)?.checked_add(seconds)?,
        nanos,
    ))
}

fn song_source_info(body: &serde_json::Value, song_id: u64) -> Result<AudioSourceInfo, String> {
    let track = body["data"]
        .as_array()
        .and_then(|tracks| {
            tracks
                .iter()
                .find(|track| track["id"].as_u64() == Some(song_id))
        })
        .ok_or_else(|| "播放地址响应缺少当前歌曲".to_string())?;
    let url = track["url"]
        .as_str()
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "这首歌暂无播放权限或已下架".to_string())?;
    let url = reqwest::Url::parse(url).map_err(|_| "歌曲播放地址无效".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("歌曲播放地址协议不受支持".into());
    }
    Ok(AudioSourceInfo {
        url: url.to_string(),
        cache_id: track["md5"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        byte_len: track["size"].as_u64().filter(|size| *size > 0),
        duration: track["time"]
            .as_u64()
            .filter(|time| *time > 0)
            .map(Duration::from_millis),
        quality: track["level"]
            .as_str()
            .and_then(AudioQualityLevel::from_api_level),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_expand_tags_sort_and_match_translation_by_time() {
        let body = serde_json::json!({
            "lrc": {"lyric": "\u{feff}[ar:歌手]\n[ti:歌曲]\n[offset:100]\n[00:02.50][00:01.5] 原文 \n[00:03]末行\n[00:04.000]\n[99:99]无效\n[18446744073709551615:01]溢出"},
            "tlyric": {"lyric": "[00:01.500][00:02.500] 译文 \n[00:03.0] \n[00:09]无对应原文"}
        });
        let lines = song_lyric_lines(&body);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].time, Duration::from_millis(1500));
        assert_eq!(lines[1].time, Duration::from_millis(2500));
        assert_eq!(lines[0].text, "原文");
        assert_eq!(lines[0].translation.as_deref(), Some("译文"));
        assert_eq!(lines[1].translation.as_deref(), Some("译文"));
        assert_eq!(lines[2].translation, None);
        assert_eq!(lines[3].text, "");
        assert_eq!(lrc_time("01:02.123"), Some(Duration::from_millis(62123)));
    }

    #[test]
    fn missing_and_instrumental_lyrics_are_empty() {
        for body in [
            serde_json::json!({}),
            serde_json::json!({"lrc": {"lyric": ""}}),
            serde_json::json!({"lrc": {"lyric": "[ti:歌曲]\n无时间文本"}}),
            serde_json::json!({"nolyric": true, "lrc": {"lyric": "[00:00]纯音乐"}}),
        ] {
            assert!(song_lyric_lines(&body).is_empty());
        }
    }

    #[test]
    fn playback_url_requires_matching_song_and_http_address() {
        let body = serde_json::json!({"data": [
            {"id": 1, "url": "https://example.com/song.mp3"},
            {"id": 2, "url": null},
            {"id": 3, "url": "file:///song.mp3"}
        ]});
        assert_eq!(
            song_source_info(&body, 1).unwrap().url,
            "https://example.com/song.mp3"
        );
        for id in [2, 3, 4] {
            assert!(song_source_info(&body, id).is_err());
        }
    }

    #[test]
    fn playback_quality_uses_actual_response_and_preserves_trial_duration() {
        let body = serde_json::json!({"data": [{
            "id": 1, "url": "https://example.com/song.flac", "size": 123456,
            "level": "lossless", "time": 30000, "md5": "content-version"
        }]});
        let source = song_source_info(&body, 1).unwrap();
        assert_eq!(source.quality, Some(AudioQualityLevel::Lossless));
        assert_eq!(source.duration, Some(Duration::from_secs(30)));
        assert_eq!(source.byte_len, Some(123456));
        assert_eq!(source.cache_id.as_deref(), Some("content-version"));
        for quality in AudioQualityLevel::ALL {
            assert_eq!(
                AudioQualityLevel::from_api_level(quality.api_level()),
                Some(quality)
            );
        }
        assert!(AudioQualityLevel::from_api_level("unknown").is_none());
    }

    #[test]
    fn privileges_attach_by_song_id_not_by_position() {
        let original = vec![
            Song {
                id: 1,
                ..Default::default()
            },
            Song {
                id: 2,
                ..Default::default()
            },
        ];
        let songs = parse_songs(&mut serde_json::json!({
            "songs": original,
            "privileges": [
                {"id": 3, "playMaxBrLevel": "hires"},
                {"id": 2, "playMaxBrLevel": "lossless"},
                {"id": 1, "playMaxBrLevel": "jymaster"}
            ]
        }))
        .unwrap();
        assert_eq!(
            songs[0].best_quality_level(),
            Some(AudioQualityLevel::JyMaster)
        );
        assert_eq!(
            songs[1].best_quality_level(),
            Some(AudioQualityLevel::Lossless)
        );
        for privileges in [serde_json::Value::Null, serde_json::json!("invalid")] {
            let songs = parse_songs(&mut serde_json::json!({
                "songs": original,
                "privileges": privileges
            }))
            .unwrap();
            assert!(songs.iter().all(|song| song.privilege.is_none()));
        }
        assert!(parse_songs(&mut serde_json::json!({"songs": null})).is_err());
    }
}
