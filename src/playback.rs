mod controller;
mod engine;
mod stream;

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
    pub is_playing: bool,
    pub loading: bool,
    pub buffering: bool,
    pub quality: AudioQualityLevel,
    pub actual_quality: Option<AudioQualityLevel>,
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
