use std::time::Duration;

use serde::{Deserialize, Serialize};

/// 歌曲详情、歌单 tracks 和每日推荐歌曲使用的字段。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub id: u64,
    pub name: String,
    pub ar: Vec<Artist>,
    pub al: Album,
    /// API 原始时长，单位为毫秒。
    pub dt: u64,
    #[serde(default)]
    pub alia: Vec<String>,
    #[serde(default)]
    pub tns: Vec<String>,
    pub fee: Option<u32>,
    pub mv: Option<u64>,
    pub publish_time: Option<i64>,
    pub h: Option<AudioQuality>,
    pub m: Option<AudioQuality>,
    pub l: Option<AudioQuality>,
    pub sq: Option<AudioQuality>,
    pub hr: Option<AudioQuality>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Artist {
    pub id: u64,
    pub name: Option<String>,
    #[serde(default)]
    pub alias: Vec<String>,
    #[serde(default)]
    pub tns: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: u64,
    pub name: Option<String>,
    pub pic_url: Option<String>,
    #[serde(default)]
    pub tns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioQuality {
    pub br: u64,
    pub size: u64,
    pub sr: Option<u32>,
    pub vd: Option<f64>,
}

impl Song {
    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.dt)
    }
}
