use super::StreamControl;
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
    pub(crate) channels: u16,
    pub(crate) sample_rate: u32,
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

    /// 直接填充 CPAL 的交错 f32 缓冲，并应用音量。
    pub fn write(&mut self, output: &mut [f32], volume: f32) {
        for frame in output.chunks_mut(self.channels as usize) {
            self.read_frame(frame);
            for sample in frame {
                *sample = (*sample * volume).clamp(-1., 1.);
            }
        }
    }
}
