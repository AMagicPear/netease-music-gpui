use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::AudioQualityLevel;

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

    /// 这首歌曲自带资源的最高音质档位；只有标准音质时返回 `None`。
    ///
    /// 依据是歌曲自带的音质变体（`hr` / `sq` / `h`），不是播放时实际选用的档位，
    /// 所以它描述的是「这首歌能到什么音质」，供歌单列表决定要不要显示徽章。
    pub fn best_quality_level(&self) -> Option<AudioQualityLevel> {
        if self.hr.is_some() {
            Some(AudioQualityLevel::HiRes)
        } else if self.sq.is_some() {
            Some(AudioQualityLevel::Lossless)
        } else if self.h.as_ref().is_some_and(|quality| quality.br >= 320_000) {
            Some(AudioQualityLevel::ExHigh)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(br: u64) -> AudioQuality {
        AudioQuality {
            br,
            size: 0,
            sr: None,
            vd: None,
        }
    }

    #[test]
    fn best_quality_level_follows_available_variants() {
        let mut song = Song::default();
        assert_eq!(song.best_quality_level(), None);

        // 达到 320 kbps 的 h 变体才算「极高」，128 kbps 不算。
        song.h = Some(variant(320_000));
        assert_eq!(song.best_quality_level(), Some(AudioQualityLevel::ExHigh));
        song.h = Some(variant(128_000));
        assert_eq!(song.best_quality_level(), None);

        // 更高的变体存在时优先取更高的那档。
        song.sq = Some(variant(999_000));
        assert_eq!(song.best_quality_level(), Some(AudioQualityLevel::Lossless));
        song.hr = Some(variant(1_500_000));
        assert_eq!(song.best_quality_level(), Some(AudioQualityLevel::HiRes));
    }
}
