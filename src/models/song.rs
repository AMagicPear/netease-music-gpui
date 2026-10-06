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
    /// `song_detail` 的 privileges 项；它和 songs 是两个按 id 对应的数组，
    /// 由 API 侧合并进来。缺失时音质判断退回上面的变体字段。
    #[serde(default)]
    pub privilege: Option<Privilege>,
}

/// 歌曲的播放/下载权益。实测 `playMaxBrLevel`、`maxBrLevel`、`maxbr` 不随登录态变化，
/// 描述的是平台为这首歌提供的最高档位；账号只影响 `pl` / `dl` 这类权限位。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Privilege {
    pub id: u64,
    /// 最高可播档位，官方 level 字符串。
    pub play_max_br_level: Option<String>,
    /// 最高可下载档位。
    pub download_max_br_level: Option<String>,
    /// 与上一项同源的总档位，个别响应只给这一个。
    pub max_br_level: Option<String>,
    /// 非 0 表示当前账号可播放；0 表示无权播放（例如未登录时的 VIP 歌曲）。
    pub pl: Option<u64>,
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

    /// 平台为这首歌提供的最高音质档位；低于「极高」时不显示徽章，返回 `None`。
    ///
    /// 优先取 privileges 的 `playMaxBrLevel`：这是平台侧的档位上限，不随账号变化，
    /// 也只有它能表达母带、臻音、沉浸声、全景声这些不在变体字段里的档位。
    /// privileges 缺失时退回歌曲自带的变体（`hr` / `sq` / `h`）。
    /// 注意它描述的是「平台有什么」，不是播放时实际选用的档位——能否播放要看 `privilege.pl`。
    pub fn best_quality_level(&self) -> Option<AudioQualityLevel> {
        let level = self
            .platform_quality_level()
            .or_else(|| self.resource_quality_level())?;
        // 「较高」及以下和没有徽章是一样的观感，沿用原有的不显示门槛。
        (level.rank() >= AudioQualityLevel::ExHigh.rank()).then_some(level)
    }

    fn platform_quality_level(&self) -> Option<AudioQualityLevel> {
        let privilege = self.privilege.as_ref()?;
        privilege
            .play_max_br_level
            .as_deref()
            .or(privilege.max_br_level.as_deref())
            .and_then(AudioQualityLevel::from_api_level)
    }

    fn resource_quality_level(&self) -> Option<AudioQualityLevel> {
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

    fn privilege(level: &str) -> Privilege {
        Privilege {
            id: 1,
            play_max_br_level: Some(level.to_string()),
            ..Default::default()
        }
    }

    /// 母带、臻音这些档位只出现在 privileges 里，本体变体字段表达不了。
    #[test]
    fn best_quality_level_prefers_privileges_over_song_variants() {
        let mut song = Song::default();
        song.sq = Some(variant(999_000));
        song.privilege = Some(privilege("jymaster"));
        assert_eq!(song.best_quality_level(), Some(AudioQualityLevel::JyMaster));

        // 「较高」及以下依旧不显示徽章，与只看变体字段时一致。
        song.privilege = Some(privilege("higher"));
        assert_eq!(song.best_quality_level(), None);

        // 权益缺失或档位认不出来时，退回本体变体。
        song.privilege = Some(privilege("unknown"));
        assert_eq!(song.best_quality_level(), Some(AudioQualityLevel::Lossless));
        song.privilege = None;
        assert_eq!(song.best_quality_level(), Some(AudioQualityLevel::Lossless));
    }
}
