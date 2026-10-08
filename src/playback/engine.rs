use super::stream::{BufferedSource, DeviceOutput, StreamingAudio};
use cpal::{
    FromSample, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
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
    source: Mutex<Option<DeviceOutput>>,
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

/// 输出端固定使用设备默认配置；切歌只替换音源，复用同一个 CPAL 流。
#[derive(Default)]
pub(super) struct PlayerEngine {
    device: Option<(cpal::Stream, cpal::SupportedStreamConfig)>,
    audio: Option<StreamingAudio>,
    control: Arc<OutputControl>,
}

impl PlayerEngine {
    pub fn load(&mut self, audio: StreamingAudio, source: BufferedSource) -> Result<(), String> {
        self.stop();
        self.ensure_device()?;
        let config = &self.device.as_ref().unwrap().1;
        let output = DeviceOutput::new(source, config.sample_rate());
        *self.control.source.lock().unwrap() = Some(output);
        self.audio = Some(audio);
        Ok(())
    }

    fn ensure_device(&mut self) -> Result<(), String> {
        if self.device.is_some() {
            return Ok(());
        }
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("没有可用的音频输出设备")?;
        let config = device
            .default_output_config()
            .map_err(|error| format!("无法读取音频输出配置：{error}"))?;
        let stream_config = config.config();
        let control = self.control.clone();
        let stream = match config.sample_format() {
            cpal::SampleFormat::I8 => build_stream::<i8>(&device, &stream_config, control),
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, &stream_config, control),
            cpal::SampleFormat::I24 => build_stream::<cpal::I24>(&device, &stream_config, control),
            cpal::SampleFormat::I32 => build_stream::<i32>(&device, &stream_config, control),
            cpal::SampleFormat::I64 => build_stream::<i64>(&device, &stream_config, control),
            cpal::SampleFormat::U8 => build_stream::<u8>(&device, &stream_config, control),
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, &stream_config, control),
            cpal::SampleFormat::U24 => build_stream::<cpal::U24>(&device, &stream_config, control),
            cpal::SampleFormat::U32 => build_stream::<u32>(&device, &stream_config, control),
            cpal::SampleFormat::U64 => build_stream::<u64>(&device, &stream_config, control),
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, &stream_config, control),
            cpal::SampleFormat::F64 => build_stream::<f64>(&device, &stream_config, control),
            format => return Err(format!("不支持的音频输出格式：{format}")),
        }
        .map_err(|error| format!("无法创建音频输出流：{error}"))?;
        // 保持流运行，暂停时回调只写静音，不消费 PCM，也不推进位置。
        stream
            .play()
            .map_err(|error| format!("无法启动音频输出：{error}"))?;
        self.device = Some((stream, config));
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

fn build_stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    control: Arc<OutputControl>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let errors = control.clone();
    device.build_output_stream(
        config,
        move |buffer: &mut [T], _| write_output(buffer, channels, &control),
        move |error| {
            errors.playing.store(false, Ordering::Release);
            *errors.error.lock().unwrap() = Some(format!("音频输出失败：{error}"));
        },
        None,
    )
}

/// 回调只尝试获取当前音源；切歌持锁或暂停时写静音，不阻塞设备线程。
fn write_output<T: SizedSample + FromSample<f32>>(
    buffer: &mut [T],
    channels: usize,
    control: &OutputControl,
) {
    if control.playing.load(Ordering::Acquire)
        && let Ok(mut source) = control.source.try_lock()
        && let Some(source) = source.as_mut()
    {
        source.write(
            buffer,
            channels,
            true,
            f32::from_bits(control.volume.load(Ordering::Acquire)),
        );
    } else {
        buffer.fill(T::from_sample(0.));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_silences_missing_paused_or_busy_source_without_blocking() {
        let control = OutputControl::default();
        let mut buffer = [0_u16; 4];
        write_output(&mut buffer, 2, &control);
        assert_eq!(buffer, [32768; 4]);
        control.playing.store(true, Ordering::Release);
        buffer.fill(0);
        write_output(&mut buffer, 2, &control);
        assert_eq!(buffer, [32768; 4]);
        let _busy = control.source.lock().unwrap();
        buffer.fill(0);
        write_output(&mut buffer, 2, &control);
        assert_eq!(buffer, [32768; 4]);
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
