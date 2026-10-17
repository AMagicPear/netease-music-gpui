mod decode;
mod download;
mod output;
#[cfg(test)]
mod tests;

pub use output::BufferedSource;

use super::audio_cache::AudioCacheStore;
use crate::models::{AudioQualityLevel, AudioSourceInfo};
use download::Cache;
use output::Pcm;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

/// 单个音源的生命周期：一次 seek 使旧读取器和旧 PCM 过期；取消则终止整个音源。
/// 与控制器用于切歌/重载音质的 snapshot revision 相互独立。
#[derive(Default)]
struct StreamControl {
    generation: AtomicU64,
    cancelled: AtomicBool,
}

impl StreamControl {
    fn is_current(&self, generation: u64) -> bool {
        !self.cancelled.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire) == generation
    }
}

/// 组装下载、解码和输出，并统一拥有跳转与取消的生命周期。
pub struct StreamingAudio {
    cache: Arc<Cache>,
    pcm: Arc<Pcm>,
    download: tokio::task::AbortHandle,
    /// 取得播放地址的时刻；地址会过期，长时间暂停后必须重新向接口取一次。
    opened: SystemTime,
    pub duration: Option<Duration>,
    pub quality: Option<AudioQualityLevel>,
}

impl StreamingAudio {
    #[cfg(test)]
    pub async fn open(
        http: reqwest::Client,
        source: AudioSourceInfo,
    ) -> Result<(Self, BufferedSource), String> {
        let store = tokio::task::spawn_blocking(AudioCacheStore::temporary)
            .await
            .map_err(|error| error.to_string())??;
        Self::open_cached(http, source, store, 0).await
    }

    pub async fn open_cached(
        http: reqwest::Client,
        source: AudioSourceInfo,
        store: Arc<AudioCacheStore>,
        song_id: u64,
    ) -> Result<(Self, BufferedSource), String> {
        let cache_source = source.clone();
        let disk = tokio::task::spawn_blocking(move || store.acquire(song_id, &cache_source))
            .await
            .map_err(|error| format!("音频缓存准备失败：{error}"))??;
        let control = Arc::new(StreamControl::default());
        let (cache, download) = Cache::open(http, &source, control.clone(), disk).await?;
        let pcm = Arc::new(Pcm {
            control,
            ..Default::default()
        });
        pcm.buffering.store(true, Ordering::Release);
        let mut audio = Self {
            cache: cache.clone(),
            pcm: pcm.clone(),
            download,
            opened: SystemTime::now(),
            duration: source.duration,
            quality: source.quality,
        };
        let (ready, receiver) = tokio::sync::oneshot::channel();
        let source_duration = source.duration;
        std::thread::Builder::new()
            .name("music-decoder".into())
            .spawn(move || decode::decode(cache, pcm, ready, source_duration))
            .map_err(|error| format!("无法启动解码线程：{error}"))?;
        let (output, duration) = receiver
            .await
            .map_err(|_| audio.error().unwrap_or_else(|| "音频解码失败".into()))?;
        audio.duration = duration.or(audio.duration);
        Ok((audio, output))
    }

    pub fn position(&self) -> Duration {
        Duration::from_micros(self.pcm.position_us.load(Ordering::Acquire))
    }

    pub fn seek(&self, position: Duration) {
        let mut data = self.pcm.data.lock().unwrap();
        data.chunks.clear();
        data.queued_samples = 0;
        data.seek = Some(position);
        data.decode_finished = false;
        data.error = None;
        self.pcm.output_drained.store(false, Ordering::Release);
        self.pcm.buffering.store(true, Ordering::Release);
        self.pcm
            .position_us
            .store(position.as_micros() as u64, Ordering::Release);
        let generation = self.pcm.control.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.pcm.changed.notify_all();
        // 持有 PCM 锁直到旧下载被中断，避免取消新代次已提交的请求。
        self.cache.interrupt(generation);
    }

    /// 解码结束且输出端已消费完 PCM，才允许控制器自动切歌。
    pub fn finished(&self) -> bool {
        self.pcm.output_drained.load(Ordering::Acquire)
    }

    pub fn buffering(&self) -> bool {
        self.pcm.buffering.load(Ordering::Acquire)
    }

    pub fn download_complete(&self) -> bool {
        self.cache.data.try_lock().is_ok_and(|data| data.complete)
    }

    /// 下载已经失败，且失败后不会再自行重试。
    pub fn download_failed(&self) -> bool {
        self.cache.failure().is_some()
    }

    /// 距最后一次成功落盘已超过 `timeout`：休眠或断网后不会再有新数据。
    /// 只要数据还在流动，即使很慢也不会被判为停摆。
    pub fn stalled(&self, timeout: Duration) -> bool {
        self.cache.stalled(timeout)
    }

    /// 播放地址的年龄。地址过期后重试多少次都拿不到数据，只能重新取地址。
    pub fn source_age(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.opened)
            .unwrap_or_default()
    }

    pub fn error(&self) -> Option<String> {
        self.pcm
            .data
            .try_lock()
            .ok()
            .and_then(|data| data.error.clone())
            .or_else(|| self.cache.error())
    }
}

impl Drop for StreamingAudio {
    fn drop(&mut self) {
        self.download.abort();
        {
            let _data = self.pcm.data.lock().unwrap();
            self.pcm.control.cancelled.store(true, Ordering::Release);
            self.pcm.changed.notify_all();
        }
        // 在各自的等待锁内通知，防止读者在检查取消后才进入等待而漏掉唤醒。
        let _data = self.cache.data.lock().unwrap();
        self.cache.changed.notify_all();
    }
}
