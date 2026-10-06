use std::time::Duration;

/// 音质档位，与网易云 `song_url_v1` 的 `level` 参数一一对应（共 9 档）。

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioQualityLevel {
    #[default]
    Standard,
    Higher,
    ExHigh,
    Lossless,
    HiRes,
    /// 高清臻音。
    JyEffect,
    /// 沉浸环绕声。请求时 SDK 会额外带上 `immerseType: c51`。
    Sky,
    /// 杜比全景声
    Dolby,
    /// 超清母带
    JyMaster,
}

impl AudioQualityLevel {
    pub fn quality_label(self) -> &'static str {
        match self {
            Self::Standard => "标准",
            Self::Higher => "较高",
            Self::ExHigh => "极高",
            Self::Lossless => "无损",
            Self::HiRes => "Hi-Res",
            Self::JyEffect => "高清臻音",
            Self::Sky => "沉浸声",
            Self::Dolby => "全景声",
            Self::JyMaster => "超清母带",
        }
    }

    /// 由低到高，与官方音质选项面板的顺序一致。
    pub const ALL: [Self; 9] = [
        Self::Standard,
        Self::Higher,
        Self::ExHigh,
        Self::Lossless,
        Self::HiRes,
        Self::JyEffect,
        Self::Sky,
        Self::Dolby,
        Self::JyMaster,
    ];

    pub fn api_level(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Higher => "higher",
            Self::ExHigh => "exhigh",
            Self::Lossless => "lossless",
            Self::HiRes => "hires",
            Self::JyEffect => "jyeffect",
            Self::Sky => "sky",
            Self::Dolby => "dolby",
            Self::JyMaster => "jymaster",
        }
    }

    pub fn from_api_level(level: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|quality| quality.api_level() == level)
    }

    /// 档位在 `ALL` 中的序号，用来比较高下；`ALL` 本身就是由低到高排列的。
    pub fn rank(self) -> usize {
        Self::ALL
            .iter()
            .position(|quality| *quality == self)
            .unwrap_or(0)
    }
}

/// 平台返回的播放资源信息；不持有 HTTP 响应、音频设备或界面状态。
pub struct AudioSourceInfo {
    pub url: String,
    pub byte_len: Option<u64>,
    pub duration: Option<Duration>,
    pub quality: Option<AudioQualityLevel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 官方 `song_url_v1` 支持的全部 level；新增档位时这里和 `ALL` 要一起改。
    #[test]
    fn all_covers_every_official_api_level() {
        let official = [
            (AudioQualityLevel::Standard, "standard"),
            (AudioQualityLevel::Higher, "higher"),
            (AudioQualityLevel::ExHigh, "exhigh"),
            (AudioQualityLevel::Lossless, "lossless"),
            (AudioQualityLevel::HiRes, "hires"),
            (AudioQualityLevel::JyEffect, "jyeffect"),
            (AudioQualityLevel::Sky, "sky"),
            (AudioQualityLevel::Dolby, "dolby"),
            (AudioQualityLevel::JyMaster, "jymaster"),
        ];
        assert_eq!(AudioQualityLevel::ALL.len(), official.len());
        for (quality, level) in official {
            assert_eq!(quality.api_level(), level);
            assert_eq!(AudioQualityLevel::from_api_level(level), Some(quality));
        }
        assert!(AudioQualityLevel::from_api_level("unknown").is_none());
    }
}
