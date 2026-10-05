use super::stream::{BufferedSource, StreamingAudio};
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use std::time::Duration;

/// 音频设备和当前播放源。这个模块不依赖 GPUI 或网易云 SDK。
pub(super) struct PlayerEngine {
    player: Option<Player>,
    audio: Option<StreamingAudio>,
    // 保留设备句柄，且让当前播放源先析构。
    device: Option<MixerDeviceSink>,
    volume: f32,
}

impl Default for PlayerEngine {
    fn default() -> Self {
        Self {
            player: None,
            audio: None,
            device: None,
            volume: 1.,
        }
    }
}

impl PlayerEngine {
    pub fn ensure_device(&mut self) -> Result<(), String> {
        if self.device.is_none() {
            let mut device = DeviceSinkBuilder::open_default_sink()
                .map_err(|error| format!("无法打开音频输出设备：{error}"))?;
            device.log_on_drop(false);
            self.device = Some(device);
        }
        Ok(())
    }

    pub fn load(&mut self, audio: StreamingAudio, source: BufferedSource) -> Result<(), String> {
        self.stop();
        self.ensure_device()?;
        let player = Player::connect_new(self.device.as_ref().unwrap().mixer());
        player.pause();
        player.set_volume(self.volume);
        player.append(source);
        self.player = Some(player);
        self.audio = Some(audio);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.player = None;
        self.audio = None;
    }

    pub fn has_source(&self) -> bool {
        self.player.as_ref().is_some_and(|player| !player.empty())
    }

    pub fn finished(&self) -> bool {
        self.audio.as_ref().is_some_and(StreamingAudio::finished)
    }

    pub fn buffering(&self) -> bool {
        self.audio.as_ref().is_some_and(StreamingAudio::buffering)
    }

    pub fn pause(&self) {
        if let Some(player) = &self.player {
            player.pause();
        }
    }

    pub fn resume(&self) -> bool {
        if let Some(player) = self
            .player
            .as_ref()
            .filter(|player| !player.empty() && !self.finished())
        {
            player.play();
            true
        } else {
            false
        }
    }

    pub fn position(&self) -> Duration {
        self.audio
            .as_ref()
            .map_or(Duration::ZERO, StreamingAudio::position)
    }

    pub fn seek_to(&self, position: Duration) -> bool {
        if self.has_source()
            && let Some(audio) = &self.audio
        {
            audio.seek(position);
            true
        } else {
            false
        }
    }

    pub fn error(&self) -> Option<String> {
        self.audio.as_ref().and_then(StreamingAudio::error)
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_volume(&mut self, volume: f32) -> bool {
        if !volume.is_finite() {
            return false;
        }
        self.volume = volume.clamp(0., 1.);
        if let Some(player) = &self.player {
            player.set_volume(self.volume);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_is_retained_without_a_device_and_ignores_non_finite_values() {
        let mut engine = PlayerEngine::default();
        assert_eq!(engine.volume(), 1.);
        assert!(engine.set_volume(0.4));
        assert_eq!(engine.volume(), 0.4);
        assert!(engine.set_volume(2.));
        assert_eq!(engine.volume(), 1.);
        assert!(engine.set_volume(-1.));
        assert_eq!(engine.volume(), 0.);
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!engine.set_volume(invalid));
            assert_eq!(engine.volume(), 0.);
        }
        engine.stop();
        assert_eq!(engine.volume(), 0.);
    }
}
