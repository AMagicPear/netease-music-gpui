use super::StreamControl;
use rodio::{ChannelCount, SampleRate, Source};
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
    // 已交给 rodio 的歌曲样本位置；缓冲静音不推进它。
    pub position_us: AtomicU64,
    // 输出端确认最后一个 PCM 已消费，控制器才允许自动切歌。
    pub output_drained: AtomicBool,
    pub buffering: AtomicBool,
}

/// 网络不足与 EOF 均输出静音，EOF 由控制器接管，避免 rodio 移除仍可能被 seek 的源。
pub struct BufferedSource {
    pub(super) pcm: Arc<Pcm>,
    pub(super) channels: u16,
    pub(super) sample_rate: u32,
    pub(super) chunk: Option<Chunk>,
    pub(super) sample_index: usize,
    pub(super) channel_index: u16,
}

impl Iterator for BufferedSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.pcm.control.cancelled.load(Ordering::Acquire) {
            return None;
        }
        if self.channel_index == 0 {
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
                }
                self.sample_index = 0;
                if self.chunk.is_some() {
                    self.pcm.changed.notify_one();
                }
                let drained = self.chunk.is_none() && data.decode_finished && data.error.is_none();
                self.pcm.output_drained.store(drained, Ordering::Release);
                self.pcm
                    .buffering
                    .store(self.chunk.is_none() && !drained, Ordering::Release);
            }
        }
        self.channel_index = (self.channel_index + 1) % self.channels;
        let Some(chunk) = &self.chunk else {
            return Some(0.);
        };
        let sample = chunk.samples[self.sample_index];
        self.sample_index += 1;
        if self.channel_index == 0
            && let Ok(_data) = self.pcm.data.try_lock()
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
        Some(sample)
    }
}

impl Source for BufferedSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channels).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.sample_rate).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
