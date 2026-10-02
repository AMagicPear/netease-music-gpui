use crate::models::AudioSourceInfo;
use rodio::{ChannelCount, Decoder, SampleRate, Source};
use std::{
    collections::VecDeque,
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

const MAX_AUDIO_BYTES: usize = 100 * 1024 * 1024;
const PCM_CHUNKS: usize = 8;

struct AudioResponse {
    body: reqwest::Response,
    byte_len: Option<u64>,
    duration: Option<Duration>,
}

#[derive(Default)]
struct Download {
    bytes: Vec<u8>,
    finished: bool,
    cancelled: bool,
    error: Option<String>,
}

struct Cache {
    data: Mutex<Download>,
    changed: Condvar,
    byte_len: Option<u64>,
}

impl Cache {
    fn finish(&self, result: Result<(), String>) {
        let mut data = self.data.lock().unwrap();
        data.finished = true;
        data.error = result.err();
        self.changed.notify_all();
    }

    async fn download(&self, mut body: reqwest::Response) -> Result<(), String> {
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(20), body.chunk())
                .await
                .map_err(|_| "音频下载超时".to_string())?
                .map_err(|_| "音频下载中断，请重试".to_string())?;
            let mut data = self.data.lock().unwrap();
            if data.cancelled {
                return Ok(());
            }
            let Some(chunk) = chunk else {
                if data.bytes.is_empty()
                    || self
                        .byte_len
                        .is_some_and(|len| data.bytes.len() as u64 != len)
                {
                    return Err("音频文件不完整，请重试".into());
                }
                return Ok(());
            };
            if data.bytes.len() + chunk.len() > MAX_AUDIO_BYTES {
                return Err("音频文件过大，暂不支持播放".into());
            }
            data.bytes.extend_from_slice(&chunk);
            self.changed.notify_all();
        }
    }
}

/// 仅在解码线程等待数据；不能把这个读取器直接交给声卡输出线程。
struct CacheReader {
    cache: Arc<Cache>,
    position: u64,
}

impl Read for CacheReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut data = self.cache.data.lock().unwrap();
        loop {
            if data.cancelled {
                return Err(io::Error::other("播放已取消"));
            }
            if let Some(error) = &data.error {
                return Err(io::Error::other(error.clone()));
            }
            if self.position < data.bytes.len() as u64 {
                let start = self.position as usize;
                let count = buffer.len().min(data.bytes.len() - start);
                buffer[..count].copy_from_slice(&data.bytes[start..start + count]);
                self.position += count as u64;
                return Ok(count);
            }
            if data.finished || self.cache.byte_len.is_some_and(|len| self.position >= len) {
                return Ok(0);
            }
            data = self.cache.changed.wait(data).unwrap();
        }
    }
}

impl Seek for CacheReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => {
                let length = if let Some(length) = self.cache.byte_len {
                    length
                } else {
                    let mut data = self.cache.data.lock().unwrap();
                    while !data.finished && !data.cancelled && data.error.is_none() {
                        data = self.cache.changed.wait(data).unwrap();
                    }
                    if data.cancelled || data.error.is_some() {
                        return Err(io::Error::other("无法读取音频长度"));
                    }
                    data.bytes.len() as u64
                };
                i128::from(length) + i128::from(offset)
            }
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "无效音频位置"))?;
        Ok(self.position)
    }
}

struct Chunk {
    samples: Vec<f32>,
    position: Duration,
    generation: u64,
}

#[derive(Default)]
struct PcmData {
    chunks: VecDeque<Chunk>,
    seek: Option<Duration>,
    finished: bool,
    error: Option<String>,
}

#[derive(Default)]
struct Pcm {
    data: Mutex<PcmData>,
    changed: Condvar,
    generation: AtomicU64,
    position_us: AtomicU64,
    cancelled: AtomicBool,
}

/// 拥有下载与解码的生命周期；切歌或关闭窗口会唤醒所有等待者。
pub struct StreamingAudio {
    cache: Arc<Cache>,
    pcm: Arc<Pcm>,
    download: tokio::task::AbortHandle,
    pub duration: Option<Duration>,
}

impl StreamingAudio {
    pub async fn open(
        http: reqwest::Client,
        source: AudioSourceInfo,
    ) -> Result<(Self, BufferedSource), String> {
        let response = tokio::time::timeout(Duration::from_secs(20), http.get(source.url).send())
            .await
            .map_err(|_| "音频连接超时".to_string())?
            .map_err(|_| "音频下载失败，请检查网络后重试".to_string())?;
        if !response.status().is_success() {
            return Err(format!("音频服务器返回错误：{}", response.status()));
        }
        let byte_len = response.content_length().or(source.byte_len);
        Self::start(AudioResponse {
            body: response,
            byte_len,
            duration: source.duration,
        })
        .await
    }

    async fn start(response: AudioResponse) -> Result<(Self, BufferedSource), String> {
        if response
            .byte_len
            .is_some_and(|length| length > MAX_AUDIO_BYTES as u64)
        {
            return Err("音频文件过大，暂不支持播放".into());
        }
        // ponytail: 单首缓存上限 100 MiB，顺序下载；远端即时 seek 再接入 Range 区间缓存。
        let cache = Arc::new(Cache {
            data: Mutex::new(Download::default()),
            changed: Condvar::new(),
            byte_len: response.byte_len,
        });
        let pcm = Arc::new(Pcm::default());
        let download_cache = cache.clone();
        let download = tokio::spawn(async move {
            let result = download_cache.download(response.body).await;
            download_cache.finish(result);
        });
        let mut audio = Self {
            cache: cache.clone(),
            pcm: pcm.clone(),
            download: download.abort_handle(),
            duration: response.duration,
        };
        let (ready, receiver) = tokio::sync::oneshot::channel();
        let worker_pcm = pcm.clone();
        std::thread::Builder::new()
            .name("music-decoder".into())
            .spawn(move || {
                if let Err(error) = decode(cache, worker_pcm.clone(), ready) {
                    let mut data = worker_pcm.data.lock().unwrap();
                    data.error = Some(error);
                    data.finished = true;
                    worker_pcm.changed.notify_all();
                }
            })
            .map_err(|error| format!("无法启动解码线程：{error}"))?;
        let (source, duration) = receiver
            .await
            .map_err(|_| audio.error().unwrap_or_else(|| "音频解码失败".into()))?;
        audio.duration = duration.or(audio.duration);
        Ok((audio, source))
    }

    pub fn position(&self) -> Duration {
        Duration::from_micros(self.pcm.position_us.load(Ordering::Acquire))
    }

    pub fn seek(&self, position: Duration) {
        let mut data = self.pcm.data.lock().unwrap();
        data.chunks.clear();
        data.seek = Some(position);
        data.finished = false;
        data.error = None;
        self.pcm
            .position_us
            .store(position.as_micros() as u64, Ordering::Release);
        self.pcm.generation.fetch_add(1, Ordering::AcqRel);
        self.pcm.changed.notify_all();
    }

    pub fn error(&self) -> Option<String> {
        self.cache
            .data
            .try_lock()
            .ok()
            .and_then(|data| data.error.clone())
            .or_else(|| {
                self.pcm
                    .data
                    .try_lock()
                    .ok()
                    .and_then(|data| data.error.clone())
            })
    }
}

impl Drop for StreamingAudio {
    fn drop(&mut self) {
        self.download.abort();
        self.cache.data.lock().unwrap().cancelled = true;
        self.cache.changed.notify_all();
        // 与 Condvar 等待使用同一把锁，避免取消通知在开始等待前丢失。
        let _data = self.pcm.data.lock().unwrap();
        self.pcm.cancelled.store(true, Ordering::Release);
        self.pcm.changed.notify_all();
    }
}

fn decode(
    cache: Arc<Cache>,
    pcm: Arc<Pcm>,
    ready: tokio::sync::oneshot::Sender<(BufferedSource, Option<Duration>)>,
) -> Result<(), String> {
    let mut builder = Decoder::builder()
        .with_data(CacheReader {
            cache: cache.clone(),
            position: 0,
        })
        .with_seekable(true);
    if let Some(length) = cache.byte_len {
        builder = builder.with_byte_len(length);
    }
    let mut decoder = builder
        .build()
        .map_err(|error| format!("音频解码失败：{error}"))?;
    let channels = decoder.channels().get();
    let sample_rate = decoder.sample_rate().get();
    let duration = decoder.total_duration();
    let source = BufferedSource {
        pcm: pcm.clone(),
        channels,
        sample_rate,
        chunk: None,
        sample_index: 0,
        channel_index: 0,
    };
    if ready.send((source, duration)).is_err() {
        return Ok(());
    }
    let chunk_samples = (sample_rate as usize / 20).max(1) * channels as usize;
    let mut start = Duration::ZERO;
    let mut decoded_samples = 0_u64;
    loop {
        let mut data = pcm.data.lock().unwrap();
        while !pcm.cancelled.load(Ordering::Acquire)
            && data.seek.is_none()
            && (data.chunks.len() >= PCM_CHUNKS || data.finished)
        {
            data = pcm.changed.wait(data).unwrap();
        }
        if pcm.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let generation = pcm.generation.load(Ordering::Acquire);
        let seek = data.seek.take();
        drop(data);
        if let Some(position) = seek {
            // 某些格式不接受恰好 EOF 的 seek；最后一个采样帧仍会自然播放结束。
            start = duration.map_or(position, |duration| {
                position
                    .min(duration.saturating_sub(Duration::from_secs_f64(1. / sample_rate as f64)))
            });
            if let Err(error) = decoder.try_seek(start) {
                if pcm.generation.load(Ordering::Acquire) != generation {
                    continue;
                }
                return Err(format!("无法跳转播放位置：{error}"));
            }
            decoded_samples = 0;
        }
        let position = start
            + Duration::from_secs_f64(
                decoded_samples as f64 / channels as f64 / sample_rate as f64,
            );
        let mut samples: Vec<_> = decoder.by_ref().take(chunk_samples).collect();
        samples.truncate(samples.len() / channels as usize * channels as usize);
        decoded_samples += samples.len() as u64;
        let mut data = pcm.data.lock().unwrap();
        if pcm.generation.load(Ordering::Acquire) != generation {
            continue;
        }
        if samples.is_empty() {
            data.finished = true;
            data.error = cache.data.lock().unwrap().error.clone();
        } else {
            data.chunks.push_back(Chunk {
                samples,
                position,
                generation,
            });
        }
        pcm.changed.notify_all();
    }
}

/// 音频回调只取已解码样本。缺数据时输出整帧静音，不等待网络、不推进歌曲进度。
pub struct BufferedSource {
    pcm: Arc<Pcm>,
    channels: u16,
    sample_rate: u32,
    chunk: Option<Chunk>,
    sample_index: usize,
    channel_index: u16,
}

impl Iterator for BufferedSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pcm.cancelled.load(Ordering::Acquire) {
            return None;
        }
        if self.channel_index == 0 {
            let generation = self.pcm.generation.load(Ordering::Acquire);
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
                self.sample_index = 0;
                if self.chunk.is_some() {
                    self.pcm.changed.notify_one();
                }
                if self.chunk.is_none() && data.finished && data.error.is_none() {
                    return None;
                }
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
            && self.pcm.generation.load(Ordering::Acquire) == chunk.generation
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
    // 包含静音等待的输出时长不等于歌曲时长，避免 rodio 在缓冲时提前裁掉结尾。
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_starts_before_eof_without_network_or_sound_device() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 两秒单声道 WAV，只先传输前 64 KiB；后半段保持未到达。
            let data_len = 192000_u32;
            let mut wav = b"RIFF".to_vec();
            wav.extend_from_slice(&(36 + data_len).to_le_bytes());
            wav.extend_from_slice(b"WAVEfmt ");
            wav.extend_from_slice(&16_u32.to_le_bytes());
            wav.extend_from_slice(&1_u16.to_le_bytes());
            wav.extend_from_slice(&1_u16.to_le_bytes());
            wav.extend_from_slice(&48000_u32.to_le_bytes());
            wav.extend_from_slice(&96000_u32.to_le_bytes());
            wav.extend_from_slice(&2_u16.to_le_bytes());
            wav.extend_from_slice(&16_u16.to_le_bytes());
            wav.extend_from_slice(b"data");
            wav.extend_from_slice(&data_len.to_le_bytes());
            wav.extend_from_slice(&[0, 32].repeat(data_len as usize / 2));
            let (sender, receiver) = futures::channel::mpsc::unbounded::<io::Result<Vec<u8>>>();
            sender.unbounded_send(Ok(wav[..65536].to_vec())).unwrap();
            let response = AudioResponse {
                body: gpui::http_client::Response::builder()
                    .body(reqwest::Body::wrap_stream(receiver))
                    .unwrap()
                    .into(),
                byte_len: Some(wav.len() as u64),
                duration: Some(Duration::from_secs(2)),
            };
            let (audio, mut source) =
                tokio::time::timeout(Duration::from_secs(2), StreamingAudio::start(response))
                    .await
                    .unwrap()
                    .unwrap();
            let pcm = audio.pcm.clone();
            tokio::task::spawn_blocking(move || {
                let data = pcm.data.lock().unwrap();
                let (data, _) = pcm
                    .changed
                    .wait_timeout_while(data, Duration::from_secs(2), |data| {
                        data.chunks.is_empty() && data.error.is_none()
                    })
                    .unwrap();
                assert!(!data.chunks.is_empty());
            })
            .await
            .unwrap();
            assert_eq!(source.next(), Some(0.25));
            assert!(!audio.cache.data.lock().unwrap().finished);
            assert!(audio.cache.data.lock().unwrap().bytes.len() < wav.len());
            // 跳到未下载的 1.5 秒处立即返回；旧 PCM 不得继续输出或覆盖进度。
            audio.seek(Duration::from_millis(1500));
            assert_eq!(source.next(), Some(0.));
            assert_eq!(audio.position(), Duration::from_millis(1500));
            sender.unbounded_send(Ok(wav[65536..].to_vec())).unwrap();
            drop(sender);
            let pcm = audio.pcm.clone();
            tokio::task::spawn_blocking(move || {
                let data = pcm.data.lock().unwrap();
                let (data, _) = pcm
                    .changed
                    .wait_timeout_while(data, Duration::from_secs(2), |data| {
                        data.chunks.is_empty() && data.error.is_none()
                    })
                    .unwrap();
                assert!(!data.chunks.is_empty(), "seek 失败：{:?}", data.error);
            })
            .await
            .unwrap();
            assert_eq!(source.next(), Some(0.25));
            assert!(audio.position() >= Duration::from_millis(1500));
            // 取消必须让下载/解码任务退出，并立即停止输出。
            drop(audio);
            assert_eq!(source.next(), None);
        });
    }

    fn cache(length: Option<u64>) -> Arc<Cache> {
        Arc::new(Cache {
            data: Mutex::new(Download::default()),
            changed: Condvar::new(),
            byte_len: length,
        })
    }

    fn source(pcm: Arc<Pcm>) -> BufferedSource {
        BufferedSource {
            pcm,
            channels: 2,
            sample_rate: 1000,
            chunk: None,
            sample_index: 0,
            channel_index: 0,
        }
    }

    #[test]
    fn cache_reads_before_download_finishes_and_can_seek_backwards() {
        let cache = cache(Some(8));
        cache.data.lock().unwrap().bytes.extend_from_slice(b"abcd");
        let mut reader = CacheReader {
            cache: cache.clone(),
            position: 0,
        };
        let mut buffer = [0; 4];
        assert_eq!(reader.read(&mut buffer).unwrap(), 4);
        assert_eq!(&buffer, b"abcd");
        assert!(!cache.data.lock().unwrap().finished);
        assert_eq!(reader.seek(SeekFrom::Current(-2)).unwrap(), 2);
        assert_eq!(reader.read(&mut buffer).unwrap(), 2);
        assert_eq!(&buffer[..2], b"cd");
        assert_eq!(reader.seek(SeekFrom::End(0)).unwrap(), 8);
        assert_eq!(reader.read(&mut buffer).unwrap(), 0);
        assert!(reader.seek(SeekFrom::Start(0)).is_ok());
        assert!(reader.seek(SeekFrom::Current(-1)).is_err());
    }

    #[test]
    fn waiting_reader_resumes_with_new_bytes_and_exits_on_cancel() {
        let cache = cache(Some(8));
        let worker_cache = cache.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut reader = CacheReader {
                cache: worker_cache,
                position: 0,
            };
            let mut buffer = [0; 4];
            sender.send(reader.read(&mut buffer).unwrap()).unwrap();
            sender
                .send(usize::from(reader.read(&mut buffer).is_err()))
                .unwrap();
        });
        {
            let mut data = cache.data.lock().unwrap();
            data.bytes.extend_from_slice(b"abcd");
            cache.changed.notify_all();
        }
        assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), 4);
        cache.data.lock().unwrap().cancelled = true;
        cache.changed.notify_all();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        worker.join().unwrap();
    }

    #[test]
    fn output_silence_preserves_progress_and_stereo_frame_alignment() {
        let pcm = Arc::new(Pcm::default());
        let mut source = source(pcm.clone());
        assert_eq!(source.next(), Some(0.));
        pcm.data.lock().unwrap().chunks.push_back(Chunk {
            samples: vec![0.25, 0.75],
            position: Duration::ZERO,
            generation: 0,
        });
        // 数据在静音帧中途到达，必须等下一帧，避免左右声道错位。
        assert_eq!(source.next(), Some(0.));
        assert_eq!(pcm.position_us.load(Ordering::Acquire), 0);
        assert_eq!(source.next(), Some(0.25));
        assert_eq!(source.next(), Some(0.75));
        assert_eq!(pcm.position_us.load(Ordering::Acquire), 1000);
        assert_eq!(source.next(), Some(0.));
        assert_eq!(source.next(), Some(0.));
        assert_eq!(pcm.position_us.load(Ordering::Acquire), 1000);
        pcm.data.lock().unwrap().finished = true;
        assert_eq!(source.next(), None);
    }

    #[test]
    fn output_does_not_block_on_busy_producer_and_discards_old_seek_samples() {
        let pcm = Arc::new(Pcm::default());
        let mut source = source(pcm.clone());
        {
            let _producer_lock = pcm.data.lock().unwrap();
            assert_eq!(source.next(), Some(0.));
            assert_eq!(source.next(), Some(0.));
        }
        source.chunk = Some(Chunk {
            samples: vec![0.25, 0.75],
            position: Duration::ZERO,
            generation: 0,
        });
        pcm.generation.store(1, Ordering::Release);
        pcm.position_us.store(5_000_000, Ordering::Release);
        assert_eq!(source.next(), Some(0.));
        assert_eq!(source.next(), Some(0.));
        assert_eq!(pcm.position_us.load(Ordering::Acquire), 5_000_000);
        pcm.data.lock().unwrap().error = Some("下载失败".into());
        pcm.data.lock().unwrap().finished = true;
        // 错误不能被当成正常 EOF，从而意外自动播放下一首。
        assert_eq!(source.next(), Some(0.));
    }
}
