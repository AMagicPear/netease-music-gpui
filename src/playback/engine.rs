use super::stream::{BufferedSource, StreamingAudio};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

struct OutputControl {
    playing: AtomicBool,
    volume: AtomicU32,
    error: Mutex<Option<String>>,
    source: Mutex<Option<BufferedSource>>,
}

impl Default for OutputControl {
    fn default() -> Self {
        Self {
            playing: AtomicBool::new(false),
            volume: AtomicU32::new(1_f32.to_bits()),
            error: Mutex::new(None),
            source: Mutex::new(None),
        }
    }
}

/// 按歌曲采样率和声道数提交 f32 PCM，由系统共享输出适配设备格式。
#[derive(Default)]
pub(super) struct PlayerEngine {
    device: Option<(cpal::Stream, cpal::StreamConfig)>,
    audio: Option<StreamingAudio>,
    control: Arc<OutputControl>,
}

impl PlayerEngine {
    pub fn load(&mut self, audio: StreamingAudio, source: BufferedSource) -> Result<(), String> {
        self.stop();
        self.ensure_device(source.sample_rate, source.channels)?;
        *self.control.source.lock().unwrap() = Some(source);
        self.audio = Some(audio);
        Ok(())
    }

    fn ensure_device(&mut self, sample_rate: u32, channels: u16) -> Result<(), String> {
        if self.device.as_ref().is_some_and(|(_, config)| {
            config.sample_rate == sample_rate && config.channels == channels
        }) {
            return Ok(());
        }
        self.device = None;
        *self.control.error.lock().unwrap() = None;
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("没有可用的音频输出设备")?;
        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        // ponytail: 依赖共享后端适配格式；其他后端拒绝时报告错误，支持它们时再加适配。
        let control = self.control.clone();
        let errors = self.control.clone();
        let stream = device
            .build_output_stream(
                &stream_config,
                move |buffer: &mut [f32], _| write_output(buffer, &control),
                move |error| {
                    errors.playing.store(false, Ordering::Release);
                    *errors.error.lock().unwrap() = Some(format!("音频输出失败：{error}"));
                },
                None,
            )
            .map_err(|error| format!("无法创建音频输出流：{error}"))?;
        // 保持流运行，暂停时回调只写静音，不消费 PCM，也不推进位置。
        stream
            .play()
            .map_err(|error| format!("无法启动音频输出：{error}"))?;
        self.device = Some((stream, stream_config));
        Ok(())
    }

    pub fn stop(&mut self) {
        self.pause();
        *self.control.source.lock().unwrap() = None;
        self.audio = None;
        // 正常停止保留输出设备；设备报错时释放，下次播放重新打开。
        let device_failed = self.control.error.lock().unwrap().is_some();
        if device_failed {
            self.device = None;
            *self.control.error.lock().unwrap() = None;
        }
    }

    pub fn has_source(&self) -> bool {
        self.audio.is_some()
    }

    pub fn finished(&self) -> bool {
        self.audio.as_ref().is_some_and(StreamingAudio::finished)
    }

    pub fn buffering(&self) -> bool {
        self.audio.as_ref().is_some_and(StreamingAudio::buffering)
    }

    pub fn pause(&self) {
        self.control.playing.store(false, Ordering::Release);
    }

    pub fn resume(&self) -> bool {
        let playing = self.has_source() && !self.finished() && self.error().is_none();
        self.control.playing.store(playing, Ordering::Release);
        playing
    }

    pub fn position(&self) -> Duration {
        self.audio
            .as_ref()
            .map_or(Duration::ZERO, StreamingAudio::position)
    }

    pub fn seek_to(&self, position: Duration) -> bool {
        if let Some(audio) = &self.audio {
            audio.seek(position);
            true
        } else {
            false
        }
    }

    pub fn error(&self) -> Option<String> {
        self.control
            .error
            .lock()
            .unwrap()
            .clone()
            .or_else(|| self.audio.as_ref().and_then(StreamingAudio::error))
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.control.volume.load(Ordering::Acquire))
    }

    pub fn set_volume(&mut self, volume: f32) -> bool {
        if !volume.is_finite() {
            return false;
        }
        self.control
            .volume
            .store(volume.clamp(0., 1.).to_bits(), Ordering::Release);
        true
    }
}

/// 回调只尝试获取当前音源；切歌持锁或暂停时写静音，不阻塞设备线程。
fn write_output(buffer: &mut [f32], control: &OutputControl) {
    if control.playing.load(Ordering::Acquire)
        && let Ok(mut source) = control.source.try_lock()
        && let Some(source) = source.as_mut()
    {
        source.write(
            buffer,
            f32::from_bits(control.volume.load(Ordering::Acquire)),
        );
    } else {
        buffer.fill(0.);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_silences_missing_paused_or_busy_source_without_blocking() {
        let control = OutputControl::default();
        let mut buffer = [1_f32; 4];
        write_output(&mut buffer, &control);
        assert_eq!(buffer, [0.; 4]);
        control.playing.store(true, Ordering::Release);
        buffer.fill(1.);
        write_output(&mut buffer, &control);
        assert_eq!(buffer, [0.; 4]);
        let _busy = control.source.lock().unwrap();
        buffer.fill(1.);
        write_output(&mut buffer, &control);
        assert_eq!(buffer, [0.; 4]);
    }

    #[test]
    fn stop_clears_failed_device_for_retry_and_retains_volume() {
        let mut engine = PlayerEngine::default();
        engine.set_volume(0.4);
        *engine.control.error.lock().unwrap() = Some("设备已断开".into());
        assert!(engine.error().is_some());
        engine.stop();
        assert!(engine.error().is_none());
        assert!(engine.device.is_none());
        assert!(!engine.has_source());
        assert!(!engine.resume());
        assert_eq!(engine.volume(), 0.4);
    }

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
