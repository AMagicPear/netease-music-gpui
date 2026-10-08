use super::StreamControl;
use cpal::{FromSample, SizedSample};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

pub(super) struct Chunk {
    pub samples: Vec<f32>,
    pub position: Duration,
    pub generation: u64,
}

#[derive(Default)]
pub(super) struct PcmData {
    pub chunks: VecDeque<Chunk>,
    pub queued_samples: usize,
    pub seek: Option<Duration>,
    // 解码已正常结束，但队列和输出源仍可能持有未消费的样本。
    pub decode_finished: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub(super) struct Pcm {
    pub data: Mutex<PcmData>,
    pub changed: Condvar,
    pub control: Arc<StreamControl>,
    // 已交给输出回调的歌曲样本位置；缓冲静音不推进它。
    pub position_us: AtomicU64,
    // 输出端确认最后一个 PCM 已消费，控制器才允许自动切歌。
    pub output_drained: AtomicBool,
    pub buffering: AtomicBool,
}

/// 网络不足与 EOF 均输出静音；播放源保持可 seek，自动切歌由控制器接管。
pub struct BufferedSource {
    pub(super) pcm: Arc<Pcm>,
    pub(super) channels: u16,
    pub(super) sample_rate: u32,
    pub(super) chunk: Option<Chunk>,
    pub(super) sample_index: usize,
}

impl BufferedSource {
    /// 读取一个完整声道帧；缺数据时写静音，取消后返回 false。
    pub fn read_frame(&mut self, frame: &mut [f32]) -> bool {
        assert_eq!(frame.len(), self.channels as usize);
        frame.fill(0.);
        if self.pcm.control.cancelled.load(Ordering::Acquire) {
            return false;
        }
        let generation = self.pcm.control.generation.load(Ordering::Acquire);
        if self
            .chunk
            .as_ref()
            .is_some_and(|chunk| chunk.generation != generation)
        {
            self.chunk = None;
        }
        if self.chunk.is_none()
            && let Ok(mut data) = self.pcm.data.try_lock()
        {
            self.chunk = data.chunks.pop_front();
            if let Some(chunk) = &self.chunk {
                data.queued_samples -= chunk.samples.len();
                self.pcm.changed.notify_one();
            }
            self.sample_index = 0;
            let drained = self.chunk.is_none() && data.decode_finished && data.error.is_none();
            self.pcm.output_drained.store(drained, Ordering::Release);
            self.pcm
                .buffering
                .store(self.chunk.is_none() && !drained, Ordering::Release);
        }
        let Some(chunk) = &self.chunk else {
            return true;
        };
        frame.copy_from_slice(&chunk.samples[self.sample_index..self.sample_index + frame.len()]);
        self.sample_index += frame.len();
        if let Ok(_data) = self.pcm.data.try_lock()
            && self.pcm.control.is_current(chunk.generation)
        {
            let elapsed_us = self.sample_index as u64 / u64::from(self.channels) * 1_000_000
                / u64::from(self.sample_rate);
            self.pcm.position_us.store(
                chunk.position.as_micros() as u64 + elapsed_us,
                Ordering::Release,
            );
        }
        if self.sample_index == chunk.samples.len() {
            self.chunk = None;
        }
        true
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// CPAL 回调拥有输出端；缓冲在创建时分配，回调不等待网络或 PCM 锁。
pub struct DeviceOutput {
    source: BufferedSource,
    current: Vec<f32>,
    next: Vec<f32>,
    phase: f64,
    ratio: f64,
    generation: u64,
    primed: bool,
}

impl DeviceOutput {
    pub fn new(source: BufferedSource, sample_rate: u32) -> Self {
        let channels = source.channels() as usize;
        let ratio = f64::from(source.sample_rate()) / f64::from(sample_rate);
        Self {
            source,
            current: vec![0.; channels],
            next: vec![0.; channels],
            phase: 0.,
            ratio,
            generation: 0,
            primed: false,
        }
    }

    pub fn write<T: SizedSample + FromSample<f32>>(
        &mut self,
        output: &mut [T],
        channels: usize,
        playing: bool,
        volume: f32,
    ) {
        if !playing {
            output.fill(T::from_sample(0.));
            return;
        }
        for frame in output.chunks_mut(channels) {
            let generation = self.source.pcm.control.generation.load(Ordering::Acquire);
            if generation != self.generation {
                self.generation = generation;
                self.primed = false;
                self.phase = 0.;
            }
            if self.ratio == 1. {
                self.source.read_frame(&mut self.current);
            } else {
                if !self.primed {
                    self.source.read_frame(&mut self.current);
                    self.source.read_frame(&mut self.next);
                    self.primed = true;
                }
                while self.phase >= 1. {
                    std::mem::swap(&mut self.current, &mut self.next);
                    self.source.read_frame(&mut self.next);
                    self.phase -= 1.;
                }
            }
            // ponytail: 设备不支持原采样率时线性插值；需要高质量降采样时换带限重采样。
            let sample_at = |index: usize| {
                if self.ratio == 1. {
                    self.current[index]
                } else {
                    self.current[index]
                        + (self.next[index] - self.current[index]) * self.phase as f32
                }
            };
            for (channel, sample) in frame.iter_mut().enumerate() {
                let value = if channels == 1 {
                    (0..self.current.len()).map(sample_at).sum::<f32>() / self.current.len() as f32
                } else if self.current.len() == 1 {
                    sample_at(0)
                } else if channel < self.current.len() {
                    sample_at(channel)
                } else {
                    0.
                };
                *sample = T::from_sample((value * volume).clamp(-1., 1.));
            }
            if self.ratio != 1. {
                self.phase += self.ratio;
            }
        }
    }
}
