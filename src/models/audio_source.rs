use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioQualityLevel {
    #[default]
    Standard,
    Higher,
    ExHigh,
    Lossless,
    HiRes,
}

impl AudioQualityLevel {
    pub const ALL: [Self; 5] = [
        Self::Standard,
        Self::Higher,
        Self::ExHigh,
        Self::Lossless,
        Self::HiRes,
    ];

    pub fn api_level(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Higher => "higher",
            Self::ExHigh => "exhigh",
            Self::Lossless => "lossless",
            Self::HiRes => "hires",
        }
    }

    #[allow(dead_code, reason = "音质菜单已按要求回退")]
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "标准",
            Self::Higher => "较高",
            Self::ExHigh => "极高",
            Self::Lossless => "无损",
            Self::HiRes => "Hi-Res",
        }
    }

    pub fn from_api_level(level: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|quality| quality.api_level() == level)
    }
}

/// 平台返回的播放资源信息；不持有 HTTP 响应、音频设备或界面状态。
pub struct AudioSourceInfo {
    pub url: String,
    pub byte_len: Option<u64>,
    pub duration: Option<Duration>,
    pub quality: Option<AudioQualityLevel>,
}
