use std::time::Duration;

/// 平台返回的播放资源信息；不持有 HTTP 响应、音频设备或界面状态。
pub struct AudioSourceInfo {
    pub url: String,
    pub byte_len: Option<u64>,
    pub duration: Option<Duration>,
}
