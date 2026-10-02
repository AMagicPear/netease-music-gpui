mod controller;
mod engine;
mod stream;

pub use controller::PlaybackController;

use crate::models::Song;
use std::time::Duration;

/// 控制器的只读视图；UI 通过 snapshot() 借用，不能直接修改播放状态。
pub struct PlaybackSnapshot {
    pub current_song: Option<Song>,
    pub position: Duration,
    pub duration: Duration,
    pub is_playing: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// 0..=1，默认 1。音量 UI 尚未实现。
    pub volume: f32,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            current_song: None,
            position: Duration::ZERO,
            duration: Duration::ZERO,
            is_playing: false,
            loading: false,
            error: None,
            volume: 1.,
        }
    }
}
