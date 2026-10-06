mod controller;
mod engine;
mod stream;
mod system_media;

pub use controller::PlaybackController;

use crate::models::{AudioQualityLevel, Song};
use std::time::Duration;

/// 控制器的只读视图；UI 通过 snapshot() 借用，不能直接修改播放状态。
pub struct PlaybackSnapshot {
    /// 每次重新加载递增，UI 用来隔离旧歌曲或旧音质的拖动事件。
    pub revision: u64,
    pub current_song: Option<Song>,
    pub position: Duration,
    pub duration: Duration,
    /// 已请求播放且音源已加载；缓冲时也可能为 true，不等于正在发出声音。
    pub is_playing: bool,
    /// 正在获取播放地址或等待解码器准备首批样本。
    pub loading: bool,
    /// 播放期间输出端暂时取不到 PCM；暂停和加载时由控制器置为 false。
    pub buffering: bool,
    pub quality: AudioQualityLevel,
    pub actual_quality: Option<AudioQualityLevel>,
    /// 下载、解码或设备错误；失败后停止，可从保留的位置重试。
    pub error: Option<String>,
    /// 0..=1，默认 1。音量 UI 尚未实现。
    pub volume: f32,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            current_song: None,
            position: Duration::ZERO,
            duration: Duration::ZERO,
            is_playing: false,
            loading: false,
            buffering: false,
            quality: AudioQualityLevel::default(),
            actual_quality: None,
            error: None,
            volume: 1.,
        }
    }
}
