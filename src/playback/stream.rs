use crate::models::{AudioQualityLevel, AudioSourceInfo};
use rodio::{ChannelCount, SampleRate, Source};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{CODEC_TYPE_FLAC, CODEC_TYPE_MP3, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, SeekMode, SeekTo},
    io::{MediaSource, MediaSourceStream},
    probe::Hint,
    units::{Time, TimeBase},
};

const BLOCK_BYTES: u64 = 256 * 1024;
const CACHE_BLOCKS: usize = 128; // Range 缓存最多 32 MiB，不随高音质文件大小增长。
const MAX_SEQUENTIAL_BYTES: u64 = 256 * 1024 * 1024;
const PCM_CHUNKS: usize = 8;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Demand {
    offset: u64,
    generation: u64,
}

struct Download {
    blocks: BTreeMap<u64, Vec<u8>>,
    byte_len: Option<u64>,
    range: bool,
    finished: bool,
    cancelled: bool,
    // None 代次表示顺序下载已永久失败，seek 不能清除它。
    error: Option<(Option<u64>, String)>,
}

struct Cache {
    data: Mutex<Download>,
    changed: Condvar,
    demand: tokio::sync::watch::Sender<Demand>,
    pcm: Arc<Pcm>,
}

impl Cache {
    fn insert(&self, offset: u64, bytes: Vec<u8>) {
        let mut data = self.data.lock().unwrap();
        data.blocks.insert(offset, bytes);
        if data.range && data.blocks.len() > CACHE_BLOCKS {
            // ponytail: 保留文件头，其余按字节顺序淘汰；反复跨区拖动变多再改 LRU。
            if let Some(key) = data
                .blocks
                .keys()
                .copied()
                .find(|key| *key != 0 && *key != offset)
            {
                data.blocks.remove(&key);
            }
        }
        self.changed.notify_all();
    }

    fn fail(&self, generation: u64, error: String) {
        let mut data = self.data.lock().unwrap();
        if self.pcm.generation.load(Ordering::Acquire) == generation && !data.cancelled {
            data.error = Some((Some(generation), error));
            self.changed.notify_all();
        }
    }

    async fn sequential(&self, mut response: reqwest::Response) -> Result<(), String> {
        let mut offset = 0;
        loop {
            let chunk = tokio::time::timeout(NETWORK_TIMEOUT, response.chunk())
                .await
                .map_err(|_| "音频下载超时".to_string())?
                .map_err(|error| format!("音频下载中断：{error}"))?;
            let mut data = self.data.lock().unwrap();
            if data.cancelled {
                return Ok(());
            }
            let Some(chunk) = chunk else {
                if offset == 0 || data.byte_len.is_some_and(|length| length != offset) {
                    return Err("音频文件不完整，请重试".into());
                }
                data.byte_len = Some(offset);
                data.finished = true;
                self.changed.notify_all();
                return Ok(());
            };
            if offset + chunk.len() as u64 > MAX_SEQUENTIAL_BYTES {
                return Err("服务器不支持分段下载，顺序缓存超过 256 MiB".into());
            }
            let mut remaining = chunk.as_ref();
            while !remaining.is_empty() {
                let block_offset = offset / BLOCK_BYTES * BLOCK_BYTES;
                let count = remaining
                    .len()
                    .min((BLOCK_BYTES - offset % BLOCK_BYTES) as usize);
                data.blocks
                    .entry(block_offset)
                    .or_default()
                    .extend_from_slice(&remaining[..count]);
                remaining = &remaining[count..];
                offset += count as u64;
            }
            self.changed.notify_all();
        }
    }
}

/// 校验服务器实际响应的区间，而不是仅信任 Accept-Ranges。
fn content_range(response: &reqwest::Response, offset: u64) -> Result<(u64, u64), String> {
    let range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes "))
        .ok_or_else(|| "音频分段响应缺少 Content-Range".to_string())?;
    let (span, total) = range.split_once('/').ok_or("音频分段响应无效")?;
    let (start, end) = span.split_once('-').ok_or("音频分段响应无效")?;
    let start: u64 = start.parse().map_err(|_| "音频分段起点无效")?;
    let end: u64 = end.parse().map_err(|_| "音频分段终点无效")?;
    let total: u64 = total.parse().map_err(|_| "音频文件长度无效")?;
    if total == 0
        || start != offset
        || end < start
        || end >= total
        || end != offset.saturating_add(BLOCK_BYTES - 1).min(total - 1)
    {
        return Err("音频分段范围与请求不一致".into());
    }
    Ok((end - start + 1, total))
}

async fn request_range(
    http: &reqwest::Client,
    url: &str,
    offset: u64,
    validator: Option<&str>,
) -> Result<reqwest::Response, String> {
    let mut request = http
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(
            reqwest::header::RANGE,
            format!("bytes={offset}-{}", offset.saturating_add(BLOCK_BYTES - 1)),
        );
    if let Some(validator) = validator {
        request = request.header(reqwest::header::IF_RANGE, validator);
    }
    tokio::time::timeout(NETWORK_TIMEOUT, request.send())
        .await
        .map_err(|_| "音频连接超时".to_string())?
        .map_err(|error| format!("音频连接失败：{error}"))
}

async fn range_bytes(
    mut response: reqwest::Response,
    offset: u64,
    total: u64,
    cache: &Cache,
) -> Result<Vec<u8>, String> {
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!(
            "音频分段下载失败（{}），请重新播放",
            response.status()
        ));
    }
    let (length, actual_total) = content_range(&response, offset)?;
    if actual_total != total {
        return Err("播放资源在下载期间发生变化，请重新播放".into());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("音频下载中断：{error}"))?
    {
        if bytes.len() + chunk.len() > length as usize {
            return Err("音频分段数据超过声明长度".into());
        }
        bytes.extend_from_slice(&chunk);
        // 块尚未下载完也可解码，避免低速网络必须先攒满 256 KiB 才出声。
        cache.insert(offset, bytes.clone());
    }
    if bytes.len() as u64 != length {
        return Err("音频分段数据不完整，请重试".into());
    }
    Ok(bytes)
}

async fn download_ranges(
    cache: Arc<Cache>,
    http: reqwest::Client,
    url: String,
    total: u64,
    initial: reqwest::Response,
    validator: Option<String>,
    mut demand: tokio::sync::watch::Receiver<Demand>,
) {
    let mut initial = Some(initial);
    loop {
        let requested = *demand.borrow_and_update();
        let cached = cache
            .data
            .lock()
            .unwrap()
            .blocks
            .get(&requested.offset)
            .is_some_and(|bytes| {
                bytes.len() as u64 == BLOCK_BYTES.min(total.saturating_sub(requested.offset))
            });
        if !cached && requested.offset < total {
            let fetch = async {
                let response = if requested.offset == 0
                    && let Some(response) = initial.take()
                {
                    response
                } else {
                    request_range(&http, &url, requested.offset, validator.as_deref()).await?
                };
                range_bytes(response, requested.offset, total, &cache).await
            };
            let result = tokio::select! {
                biased;
                changed = demand.changed() => {
                    if changed.is_err() { return; }
                    continue;
                }
                result = tokio::time::timeout(NETWORK_TIMEOUT, fetch) =>
                    result.unwrap_or_else(|_| Err("音频下载超时".into())),
            };
            match result {
                Ok(bytes) => cache.insert(requested.offset, bytes),
                Err(error) => cache.fail(requested.generation, error),
            }
        }
        if demand.changed().await.is_err() {
            return;
        }
    }
}

/// 仅解码线程等待数据；每个读者绑定一个 seek 代次，旧读者会立即退出等待。
struct CacheReader {
    cache: Arc<Cache>,
    position: u64,
    generation: u64,
    eof: Arc<AtomicBool>,
}

impl CacheReader {
    fn check(&self, data: &Download) -> io::Result<()> {
        if data.cancelled || self.cache.pcm.generation.load(Ordering::Acquire) != self.generation {
            // 不能用 Interrupted：read_exact 会自动重试，导致旧请求无法退出。
            return Err(io::Error::other("播放或跳转已更新"));
        }
        if let Some((generation, error)) = &data.error
            && generation.is_none_or(|generation| generation == self.generation)
        {
            return Err(io::Error::other(error.clone()));
        }
        Ok(())
    }
}

impl Read for CacheReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut data = self.cache.data.lock().unwrap();
        loop {
            self.check(&data)?;
            if data.byte_len.is_some_and(|length| self.position >= length) {
                self.eof.store(true, Ordering::Release);
                return Ok(0);
            }
            let offset = self.position / BLOCK_BYTES * BLOCK_BYTES;
            let start = (self.position - offset) as usize;
            if let Some(bytes) = data.blocks.get(&offset)
                && start < bytes.len()
            {
                let count = buffer.len().min(bytes.len() - start);
                buffer[..count].copy_from_slice(&bytes[start..start + count]);
                self.position += count as u64;
                return Ok(count);
            }
            let requested = Demand {
                offset,
                generation: self.generation,
            };
            self.cache.demand.send_if_modified(|current| {
                if *current == requested {
                    false
                } else {
                    *current = requested;
                    true
                }
            });
            data = self.cache.changed.wait(data).unwrap();
        }
    }
}

impl Seek for CacheReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let mut data = self.cache.data.lock().unwrap();
        self.check(&data)?;
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => {
                while data.byte_len.is_none() {
                    self.check(&data)?;
                    data = self.cache.changed.wait(data).unwrap();
                }
                i128::from(data.byte_len.unwrap()) + i128::from(offset)
            }
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "无效音频位置"))?;
        self.eof.store(false, Ordering::Release);
        Ok(self.position)
    }
}

impl MediaSource for CacheReader {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        self.cache.data.lock().unwrap().byte_len
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
    queued_samples: usize,
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
    drained: AtomicBool,
    buffering: AtomicBool,
}

/// 拥有网络与解码生命周期，切歌时取消请求并唤醒全部等待者。
pub struct StreamingAudio {
    cache: Arc<Cache>,
    pcm: Arc<Pcm>,
    download: tokio::task::AbortHandle,
    pub duration: Option<Duration>,
    pub quality: Option<AudioQualityLevel>,
}

impl StreamingAudio {
    pub async fn open(
        http: reqwest::Client,
        source: AudioSourceInfo,
    ) -> Result<(Self, BufferedSource), String> {
        let response = request_range(&http, &source.url, 0, None).await?;
        if !response.status().is_success() {
            return Err(format!("音频服务器返回错误：{}", response.status()));
        }
        let range = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let byte_len = if range {
            Some(content_range(&response, 0)?.1)
        } else {
            response.content_length().or(source.byte_len)
        };
        let validator = response
            .headers()
            .get(reqwest::header::ETAG)
            .filter(|value| !value.as_bytes().starts_with(b"W/"))
            .or_else(|| response.headers().get(reqwest::header::LAST_MODIFIED))
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let pcm = Arc::new(Pcm::default());
        pcm.buffering.store(true, Ordering::Release);
        let (sender, receiver) = tokio::sync::watch::channel(Demand {
            offset: 0,
            generation: 0,
        });
        let cache = Arc::new(Cache {
            data: Mutex::new(Download {
                blocks: BTreeMap::new(),
                byte_len,
                range,
                finished: false,
                cancelled: false,
                error: None,
            }),
            changed: Condvar::new(),
            demand: sender,
            pcm: pcm.clone(),
        });
        let download_cache = cache.clone();
        let download = tokio::spawn(async move {
            if range {
                download_ranges(
                    download_cache,
                    http,
                    source.url,
                    byte_len.unwrap(),
                    response,
                    validator,
                    receiver,
                )
                .await;
            } else if let Err(error) = download_cache.sequential(response).await {
                let mut data = download_cache.data.lock().unwrap();
                data.error = Some((None, error));
                download_cache.changed.notify_all();
            }
        });
        let mut audio = Self {
            cache: cache.clone(),
            pcm: pcm.clone(),
            download: download.abort_handle(),
            duration: source.duration,
            quality: source.quality,
        };
        let (ready, receiver) = tokio::sync::oneshot::channel();
        let source_duration = source.duration;
        std::thread::Builder::new()
            .name("music-decoder".into())
            .spawn(move || decode(cache, pcm, ready, source_duration))
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
        data.finished = false;
        data.error = None;
        self.pcm.drained.store(false, Ordering::Release);
        self.pcm.buffering.store(true, Ordering::Release);
        self.pcm
            .position_us
            .store(position.as_micros() as u64, Ordering::Release);
        let generation = self.pcm.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.pcm.changed.notify_all();
        // 同锁唤醒正在等网络的旧读者，同时抢占旧 Range 请求。
        let mut download = self.cache.data.lock().unwrap();
        if download
            .error
            .as_ref()
            .is_some_and(|(generation, _)| generation.is_some())
        {
            download.error = None;
        }
        self.cache.demand.send_replace(Demand {
            // 只取消旧下载。新读者复用文件头，缺数据时再提交真实区间。
            offset: u64::MAX,
            generation,
        });
        self.cache.changed.notify_all();
    }

    pub fn finished(&self) -> bool {
        self.pcm.drained.load(Ordering::Acquire)
    }
    pub fn buffering(&self) -> bool {
        self.pcm.buffering.load(Ordering::Acquire)
    }

    pub fn error(&self) -> Option<String> {
        self.pcm
            .data
            .try_lock()
            .ok()
            .and_then(|data| data.error.clone())
            .or_else(|| {
                self.cache
                    .data
                    .try_lock()
                    .ok()
                    .and_then(|data| data.error.as_ref().map(|(_, error)| error.clone()))
            })
    }
}

impl Drop for StreamingAudio {
    fn drop(&mut self) {
        self.download.abort();
        {
            let mut data = self.cache.data.lock().unwrap();
            data.cancelled = true;
            self.cache.changed.notify_all();
        }
        let _data = self.pcm.data.lock().unwrap();
        self.pcm.cancelled.store(true, Ordering::Release);
        self.pcm.changed.notify_all();
    }
}

type Ready = tokio::sync::oneshot::Sender<(BufferedSource, Option<Duration>)>;

/// 每次 seek 重建读取器和解码器；失败和被打断的读取不会留下损坏的解析状态。
fn decode(cache: Arc<Cache>, pcm: Arc<Pcm>, ready: Ready, source_duration: Option<Duration>) {
    let mut ready = Some(ready);
    let mut spec = None;
    let mut target = Duration::ZERO;
    let mut generation = 0;
    loop {
        let result = decode_generation(
            &cache,
            &pcm,
            generation,
            target,
            &mut ready,
            &mut spec,
            source_duration,
        );
        let mut data = pcm.data.lock().unwrap();
        if pcm.generation.load(Ordering::Acquire) == generation {
            match result {
                Ok(()) => data.finished = true,
                Err(error) => data.error = Some(error),
            }
            pcm.changed.notify_all();
            if ready.is_some() {
                return;
            }
        }
        while data.seek.is_none() && !pcm.cancelled.load(Ordering::Acquire) {
            data = pcm.changed.wait(data).unwrap();
        }
        if pcm.cancelled.load(Ordering::Acquire) {
            return;
        }
        target = data.seek.take().unwrap();
        generation = pcm.generation.load(Ordering::Acquire);
    }
}

fn time_duration(time: Time) -> Duration {
    Duration::from_secs(time.seconds) + Duration::from_secs_f64(time.frac)
}

fn seek_time(position: Duration, native_target: Duration, target: Duration) -> Duration {
    if position >= native_target {
        target + (position - native_target)
    } else {
        target.saturating_sub(native_target - position)
    }
}

fn decode_generation(
    cache: &Arc<Cache>,
    pcm: &Arc<Pcm>,
    generation: u64,
    mut target: Duration,
    ready: &mut Option<Ready>,
    output_spec: &mut Option<(u16, u32)>,
    source_duration: Option<Duration>,
) -> Result<(), String> {
    let eof = Arc::new(AtomicBool::new(false));
    let reader = CacheReader {
        cache: cache.clone(),
        position: 0,
        generation,
        eof: eof.clone(),
    };
    let stream = MediaSourceStream::new(Box::new(reader), Default::default());
    let options = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let mut format = symphonia::default::get_probe()
        .format(&Hint::new(), stream, &options, &Default::default())
        .map_err(|error| format!("音频格式读取失败：{error}"))?
        .format;
    let track = format.default_track().ok_or("音频中没有可播放的轨道")?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let time_base = params
        .time_base
        .or_else(|| params.sample_rate.map(|rate| TimeBase::new(1, rate)))
        .ok_or("音频缺少时间基准")?;
    let native_duration = params
        .n_frames
        .map(|frames| time_duration(time_base.calc_time(frames)));
    let duration = if params.codec == CODEC_TYPE_MP3 {
        source_duration.or(native_duration)
    } else {
        native_duration
    };
    if let Some(duration) = duration {
        target = target.min(duration.saturating_sub(Duration::from_micros(1)));
    }
    // MP3 无 Xing 时容器时长是估计。按可信 API 时长换算粗定位的比例，避免提前钳制目标。
    let native_target = if params.codec == CODEC_TYPE_MP3
        && let (Some(native), Some(duration)) = (native_duration, duration)
        && !duration.is_zero()
    {
        native.mul_f64(target.as_secs_f64() / duration.as_secs_f64())
    } else {
        target
    };
    let mut expected_position = Duration::ZERO;
    if !target.is_zero() {
        // ponytail: MP3 Coarse 按字节比例定位；VBR 精确跳转需接入 Xing/VBRI 索引。
        // MP3 的 Accurate 模式会从头扫描；FLAC/MP4 使用自身索引。
        let seeked = format
            .seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time: Time::from(native_target.as_secs_f64()),
                    track_id: Some(track_id),
                },
            )
            .map_err(|error| format!("无法跳转播放位置：{error}"))?;
        expected_position = seek_time(
            time_duration(time_base.calc_time(seeked.actual_ts)),
            native_target,
            target,
        );
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(
            &params,
            &DecoderOptions {
                verify: target.is_zero(),
            },
        )
        .map_err(|error| format!("音频解码器初始化失败：{error}"))?;
    let mut decoded_end = expected_position;
    let mut decoded_any = false;
    loop {
        if pcm.cancelled.load(Ordering::Acquire)
            || pcm.generation.load(Ordering::Acquire) != generation
        {
            return Ok(());
        }
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                // 文件真正读完且时长匹配才算 EOF；文件内部的短读属于损坏。
                let tolerance =
                    Duration::from_secs_f64(2. / params.sample_rate.unwrap_or(48000) as f64);
                let complete = decoded_any
                    && if params.codec == CODEC_TYPE_MP3 {
                        // MP3 缺少 Xing 时，n_frames 仅是码率估算，不能据此判定损坏。
                        eof.load(Ordering::Acquire)
                    } else {
                        duration.map_or_else(
                            || eof.load(Ordering::Acquire),
                            |duration| decoded_end + tolerance >= duration,
                        )
                    };
                if !complete {
                    return Err("音频内容提前结束或已损坏，请重试".into());
                }
                if decoder.finalize().verify_ok == Some(false) {
                    return Err("音频校验失败，文件可能已损坏".into());
                }
                return Ok(());
            }
            Err(error) => return Err(format!("音频读取失败：{error}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let audio = decoder
            .decode(&packet)
            .map_err(|error| format!("音频解码失败：{error}"))?;
        let spec = *audio.spec();
        let channels = u16::try_from(spec.channels.count()).map_err(|_| "音频声道过多")?;
        if channels == 0 || spec.rate == 0 {
            return Err("音频声道或采样率无效".into());
        }
        if output_spec.is_some_and(|expected| expected != (channels, spec.rate)) {
            return Err("歌曲播放期间声道或采样率发生变化".into());
        }
        *output_spec = Some((channels, spec.rate));
        let mut buffer = SampleBuffer::<f32>::new(audio.capacity() as u64, spec);
        buffer.copy_interleaved_ref(audio);
        // Symphonia 的解码器已处理 gapless trim，包时间戳对应解码后的有效样本。
        let position = seek_time(
            time_duration(time_base.calc_time(packet.ts())),
            native_target,
            target,
        );
        if params.codec == CODEC_TYPE_FLAC && position > decoded_end + Duration::from_micros(100) {
            return Err("音频帧缺失或已损坏，请重试".into());
        }
        let frames = buffer.samples().len() / channels as usize;
        decoded_end = position + Duration::from_secs_f64(frames as f64 / spec.rate as f64);
        decoded_any |= frames > 0;
        let skip = ((target.saturating_sub(position).as_secs_f64() * spec.rate as f64).ceil()
            as usize)
            .min(frames);
        let samples = &buffer.samples()[skip * channels as usize..];
        if samples.is_empty() {
            continue;
        }
        let chunk_samples = (spec.rate as usize / 20).max(1) * channels as usize;
        for (index, samples) in samples.chunks(chunk_samples).enumerate() {
            let mut data = pcm.data.lock().unwrap();
            // 按样本量限制约 400 ms，而不是按包数；高采样率的小 FLAC 包也能攒够缓冲。
            while data.queued_samples >= chunk_samples * PCM_CHUNKS
                && !pcm.cancelled.load(Ordering::Acquire)
                && pcm.generation.load(Ordering::Acquire) == generation
            {
                data = pcm.changed.wait(data).unwrap();
            }
            if pcm.cancelled.load(Ordering::Acquire)
                || pcm.generation.load(Ordering::Acquire) != generation
            {
                return Ok(());
            }
            data.queued_samples += samples.len();
            data.chunks.push_back(Chunk {
                samples: samples.to_vec(),
                generation,
                position: position
                    + Duration::from_secs_f64(
                        (skip + index * chunk_samples / channels as usize) as f64
                            / spec.rate as f64,
                    ),
            });
            pcm.changed.notify_all();
            if let Some(sender) = ready.take() {
                let source = BufferedSource {
                    pcm: pcm.clone(),
                    channels,
                    sample_rate: spec.rate,
                    chunk: None,
                    sample_index: 0,
                    channel_index: 0,
                };
                if sender.send((source, duration)).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

/// 网络不足与 EOF 均输出静音，EOF 由控制器接管，避免 rodio 移除仍可能被 seek 的源。
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
                if let Some(chunk) = &self.chunk {
                    data.queued_samples -= chunk.samples.len();
                }
                self.sample_index = 0;
                if self.chunk.is_some() {
                    self.pcm.changed.notify_one();
                }
                let drained = self.chunk.is_none() && data.finished && data.error.is_none();
                self.pcm.drained.store(drained, Ordering::Release);
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
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn wav(seconds: u32) -> Vec<u8> {
        let data_len = seconds * 48000 * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&48000_u32.to_le_bytes());
        bytes.extend_from_slice(&96000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for second in 0..seconds {
            for _ in 0..48000 {
                bytes.extend_from_slice(&((second + 1) as i16 * 1000).to_le_bytes());
            }
        }
        bytes
    }

    struct Server {
        url: String,
        requests: tokio::sync::mpsc::UnboundedReceiver<u64>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn serve(
        bytes: Vec<u8>,
        range: bool,
        broken: Option<u64>,
        stalled: Option<(u64, Arc<tokio::sync::Notify>)>,
    ) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/song.wav", listener.local_addr().unwrap());
        let bytes = Arc::new(bytes);
        let (sender, requests) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let bytes = bytes.clone();
                let sender = sender.clone();
                let stalled = stalled.clone();
                connections.spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        let Ok(count) = socket.read(&mut buffer).await else { return; };
                        if count == 0 { return; }
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let headers = String::from_utf8(request).unwrap().to_lowercase();
                    let span = headers.lines().find_map(|line| line.strip_prefix("range: bytes=")).unwrap();
                    let (start, end) = span.split_once('-').unwrap();
                    let start: u64 = start.parse().unwrap();
                    let end = end.parse::<usize>().unwrap().min(bytes.len() - 1);
                    let _ = sender.send(start);
                    if let Some((offset, notify)) = &stalled && *offset == start && start != 0 {
                        notify.notified().await;
                    }
                    let (header, body) = if range {
                        let reported = if broken == Some(start) { start + 1 } else { start };
                        (format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {reported}-{end}/{}\r\nConnection: close\r\n\r\n", end + 1 - start as usize, bytes.len()), &bytes[start as usize..=end])
                    } else {
                        (format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()), bytes.as_slice())
                    };
                    if socket.write_all(header.as_bytes()).await.is_ok() {
                        if let Some((0, notify)) = &stalled && start == 0 {
                            let count = body.len().min(65536);
                            if socket.write_all(&body[..count]).await.is_err() { return; }
                            notify.notified().await;
                            let _ = socket.write_all(&body[count..]).await;
                        } else {
                            let _ = socket.write_all(body).await;
                        }
                    }
                });
                // 清理已经完成的连接，JoinSet 析构时会取消仍在等待的连接。
                while connections.try_join_next().is_some() {}
            }
        });
        Server {
            url,
            requests,
            task,
        }
    }

    async fn open(server: &Server) -> (StreamingAudio, BufferedSource) {
        open_with_duration(server, None).await
    }

    async fn open_with_duration(
        server: &Server,
        duration: Option<Duration>,
    ) -> (StreamingAudio, BufferedSource) {
        tokio::time::timeout(
            Duration::from_secs(3),
            StreamingAudio::open(
                reqwest::Client::new(),
                AudioSourceInfo {
                    url: server.url.clone(),
                    byte_len: None,
                    duration,
                    quality: None,
                },
            ),
        )
        .await
        .unwrap()
        .unwrap()
    }

    async fn wait_pcm(pcm: &Arc<Pcm>) {
        let pcm = pcm.clone();
        tokio::task::spawn_blocking(move || {
            let data = pcm.data.lock().unwrap();
            let (data, timeout) = pcm
                .changed
                .wait_timeout_while(data, Duration::from_secs(3), |data| {
                    data.chunks.is_empty() && data.error.is_none()
                })
                .unwrap();
            assert!(!timeout.timed_out(), "PCM 等待超时");
            assert!(data.error.is_none(), "{:?}", data.error);
            assert!(!data.chunks.is_empty());
        })
        .await
        .unwrap();
    }

    async fn next_audible(output: &mut BufferedSource) -> f32 {
        // 回调碰到生产者持锁会输出一帧静音，这是设计行为；测试等待真正消费 PCM。
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let sample = output.next().expect("播放源不应在取消前退出");
                if sample != 0. {
                    return sample;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("播放源持续静音")
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

    #[tokio::test]
    async fn cache_is_bounded_and_seek_cannot_hide_terminal_download_errors() {
        let pcm = Arc::new(Pcm::default());
        let (demand, _receiver) = tokio::sync::watch::channel(Demand {
            offset: 0,
            generation: 0,
        });
        let cache = Arc::new(Cache {
            data: Mutex::new(Download {
                blocks: BTreeMap::new(),
                byte_len: Some(1024 * BLOCK_BYTES),
                range: true,
                finished: false,
                cancelled: false,
                error: None,
            }),
            changed: Condvar::new(),
            demand,
            pcm: pcm.clone(),
        });
        for block in 0..=CACHE_BLOCKS {
            cache.insert(block as u64 * BLOCK_BYTES, vec![1]);
        }
        {
            let data = cache.data.lock().unwrap();
            assert_eq!(data.blocks.len(), CACHE_BLOCKS);
            assert!(data.blocks.contains_key(&0));
            assert!(
                data.blocks
                    .contains_key(&(CACHE_BLOCKS as u64 * BLOCK_BYTES))
            );
        }
        let task = tokio::spawn(async {});
        let audio = StreamingAudio {
            cache: cache.clone(),
            pcm,
            download: task.abort_handle(),
            duration: None,
            quality: None,
        };
        cache.fail(0, "旧跳转下载失败".into());
        audio.seek(Duration::from_secs(1));
        cache.fail(0, "晚到的旧错误".into());
        assert!(audio.error().is_none());
        cache.data.lock().unwrap().error = Some((None, "顺序连接已断开".into()));
        audio.seek(Duration::from_secs(2));
        assert_eq!(audio.error().as_deref(), Some("顺序连接已断开"));
    }

    #[test]
    fn content_range_rejects_wrong_short_and_overflowing_spans() {
        for (header, valid) in [
            ("bytes 0-7/8", true),
            ("bytes 0-3/8", false),
            ("bytes 1-7/8", false),
            ("bytes 0-8/8", false),
            ("bytes 0-7/*", false),
            ("bytes 0-18446744073709551615/18446744073709551615", false),
        ] {
            let response: reqwest::Response = gpui::http_client::Response::builder()
                .status(206)
                .header("content-range", header)
                .body(reqwest::Body::from(Vec::new()))
                .unwrap()
                .into();
            assert_eq!(content_range(&response, 0).is_ok(), valid, "{header}");
        }
    }

    #[tokio::test]
    async fn playback_and_seek_do_not_wait_for_the_first_range_to_finish() {
        let mut server = serve(
            wav(12),
            true,
            None,
            Some((0, Arc::new(tokio::sync::Notify::new()))),
        )
        .await;
        let (audio, mut output) = open(&server).await;
        assert_eq!(server.requests.recv().await, Some(0));
        assert!(audio.cache.data.lock().unwrap().blocks[&0].len() < BLOCK_BYTES as usize);
        assert_eq!(next_audible(&mut output).await, 1000. / 32768.);
        audio.seek(Duration::from_secs(10));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if server.requests.recv().await.unwrap() == 3 * BLOCK_BYTES {
                    break;
                }
            }
        })
        .await
        .unwrap();
        wait_pcm(&audio.pcm).await;
        assert_eq!(next_audible(&mut output).await, 11000. / 32768.);
        assert!(audio.position() >= Duration::from_secs(10));
        assert!(audio.error().is_none());
    }

    #[tokio::test]
    async fn seek_fetches_target_range_and_cached_return_interrupts_stalled_download() {
        let mut server = serve(
            wav(12),
            true,
            None,
            Some((3 * BLOCK_BYTES, Arc::new(tokio::sync::Notify::new()))),
        )
        .await;
        let (audio, mut source) = open(&server).await;
        assert_eq!(server.requests.recv().await, Some(0));
        assert_eq!(audio.duration, Some(Duration::from_secs(12)));
        let cache = audio.cache.clone();
        tokio::task::spawn_blocking(move || {
            let data = cache.data.lock().unwrap();
            let (data, timeout) = cache
                .changed
                .wait_timeout_while(data, Duration::from_secs(3), |data| {
                    data.blocks
                        .get(&0)
                        .is_none_or(|bytes| bytes.len() < BLOCK_BYTES as usize)
                })
                .unwrap();
            assert!(!timeout.timed_out());
            assert_eq!(data.blocks[&0].len(), BLOCK_BYTES as usize);
        })
        .await
        .unwrap();
        assert_eq!(next_audible(&mut source).await, 1000. / 32768.);
        audio.seek(Duration::from_secs(10));
        assert_eq!(source.next(), Some(0.));
        // 直接访问目标区间，不读取位于中间的第 1、2 块。
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), server.requests.recv())
                .await
                .unwrap(),
            Some(3 * BLOCK_BYTES)
        );
        audio.seek(Duration::from_secs(1));
        wait_pcm(&audio.pcm).await;
        assert_eq!(next_audible(&mut source).await, 2000. / 32768.);
        assert!(audio.position() >= Duration::from_secs(1));
        assert!(server.requests.try_recv().is_err(), "回退应复用文件头缓存");
        assert!(audio.error().is_none());
        drop(audio);
        assert_eq!(source.next(), None);
    }

    #[tokio::test]
    async fn failed_seek_keeps_worker_alive_and_new_seek_recovers() {
        let server = serve(wav(12), true, Some(3 * BLOCK_BYTES), None).await;
        let (audio, mut source) = open(&server).await;
        audio.seek(Duration::from_secs(10));
        let pcm = audio.pcm.clone();
        tokio::task::spawn_blocking(move || {
            let data = pcm.data.lock().unwrap();
            let (data, timeout) = pcm
                .changed
                .wait_timeout_while(data, Duration::from_secs(3), |data| data.error.is_none())
                .unwrap();
            assert!(!timeout.timed_out());
            assert!(data.error.is_some());
        })
        .await
        .unwrap();
        audio.seek(Duration::from_secs(1));
        wait_pcm(&audio.pcm).await;
        assert!(audio.error().is_none());
        assert_eq!(next_audible(&mut source).await, 2000. / 32768.);
    }

    #[tokio::test]
    async fn ignored_range_falls_back_and_eof_source_remains_seekable() {
        let server = serve(wav(1), false, None, None).await;
        let (audio, source) = open(&server).await;
        assert!(!audio.cache.data.lock().unwrap().range);
        let (audio, mut source) = tokio::task::spawn_blocking(move || {
            let mut source = source;
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !audio.finished() {
                assert!(std::time::Instant::now() < deadline);
                assert!(source.next().is_some());
                std::thread::yield_now();
            }
            assert!(audio.error().is_none());
            assert_eq!(source.next(), Some(0.));
            (audio, source)
        })
        .await
        .unwrap();
        audio.seek(Duration::from_millis(500));
        assert!(!audio.finished());
        wait_pcm(&audio.pcm).await;
        assert_eq!(next_audible(&mut source).await, 1000. / 32768.);
    }

    #[tokio::test]
    async fn truncated_audio_is_error_instead_of_normal_eof() {
        let mut bytes = wav(2);
        bytes.truncate(44 + 48000 * 2);
        let server = serve(bytes, true, None, None).await;
        let (audio, source) = open(&server).await;
        tokio::task::spawn_blocking(move || {
            let mut source = source;
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while audio.error().is_none() {
                assert!(std::time::Instant::now() < deadline);
                source.next();
                std::thread::yield_now();
            }
            assert!(!audio.finished());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn mp3_hires_flac_and_aac_decode_seek_and_finish_without_a_device() {
        for bytes in [
            include_bytes!("../../tests/fixtures/tone.mp3").as_slice(),
            include_bytes!("../../tests/fixtures/tone-vbr.mp3").as_slice(),
            include_bytes!("../../tests/fixtures/tone.flac").as_slice(),
            include_bytes!("../../tests/fixtures/tone.m4a").as_slice(),
        ] {
            let server = serve(bytes.to_vec(), true, None, None).await;
            let (audio, mut source) =
                open_with_duration(&server, Some(Duration::from_secs(1))).await;
            assert!((audio.duration.unwrap().as_secs_f64() - 1.).abs() < 0.1);
            if bytes.starts_with(b"fLaC") {
                assert_eq!(source.sample_rate().get(), 96000);
                assert_eq!(source.channels().get(), 2);
                let pcm = audio.pcm.clone();
                tokio::task::spawn_blocking(move || {
                    let data = pcm.data.lock().unwrap();
                    let (data, timeout) = pcm
                        .changed
                        .wait_timeout_while(data, Duration::from_secs(3), |data| {
                            data.queued_samples < 96000 * 2 * 7 / 20 && data.error.is_none()
                        })
                        .unwrap();
                    assert!(!timeout.timed_out(), "Hi-Res 小包应保持至少 350 ms 缓冲");
                    assert!(data.error.is_none());
                })
                .await
                .unwrap();
            }
            audio.seek(Duration::from_millis(500));
            wait_pcm(&audio.pcm).await;
            tokio::task::spawn_blocking(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let mut audible = false;
                while !audio.finished() && audio.error().is_none() {
                    assert!(std::time::Instant::now() < deadline);
                    audible |= source.next().unwrap().abs() > 0.001;
                    std::thread::yield_now();
                }
                assert!(audible);
                assert!(audio.error().is_none(), "{:?}", audio.error());
                assert!(audio.finished());
                assert!(audio.position() >= Duration::from_millis(900));
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn corrupt_flac_reports_decoder_error() {
        let mut bytes = include_bytes!("../../tests/fixtures/tone.flac").to_vec();
        let mut frame = 4;
        loop {
            let last = bytes[frame] & 0x80 != 0;
            let length =
                u32::from_be_bytes([0, bytes[frame + 1], bytes[frame + 2], bytes[frame + 3]]);
            frame += 4 + length as usize;
            if last {
                break;
            }
        }
        bytes[frame + 20] ^= 0x80;
        let server = serve(bytes, true, None, None).await;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            StreamingAudio::open(
                reqwest::Client::new(),
                AudioSourceInfo {
                    url: server.url.clone(),
                    byte_len: None,
                    duration: None,
                    quality: None,
                },
            ),
        )
        .await
        .unwrap();
        if let Ok((audio, mut output)) = result {
            tokio::task::spawn_blocking(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                while audio.error().is_none() {
                    assert!(!audio.finished(), "损坏的 FLAC 不能作为正常 EOF");
                    assert!(
                        std::time::Instant::now() < deadline,
                        "损坏的 FLAC 必须报告错误"
                    );
                    output.next();
                    std::thread::yield_now();
                }
            })
            .await
            .unwrap();
        }
    }

    #[test]
    fn output_is_nonblocking_and_silence_preserves_stereo_alignment_and_progress() {
        let pcm = Arc::new(Pcm::default());
        let mut output = source(pcm.clone());
        {
            let _busy = pcm.data.lock().unwrap();
            assert_eq!(output.next(), Some(0.));
            assert_eq!(output.next(), Some(0.));
        }
        assert_eq!(output.next(), Some(0.));
        {
            let mut data = pcm.data.lock().unwrap();
            data.queued_samples = 2;
            data.chunks.push_back(Chunk {
                samples: vec![0.25, 0.75],
                position: Duration::ZERO,
                generation: 0,
            });
        }
        assert_eq!(output.next(), Some(0.));
        assert_eq!(pcm.position_us.load(Ordering::Acquire), 0);
        assert_eq!(output.next(), Some(0.25));
        assert_eq!(output.next(), Some(0.75));
        assert_eq!(pcm.position_us.load(Ordering::Acquire), 1000);
        output.chunk = Some(Chunk {
            samples: vec![0.5, 0.5],
            position: Duration::ZERO,
            generation: 0,
        });
        pcm.generation.store(1, Ordering::Release);
        pcm.position_us.store(5_000_000, Ordering::Release);
        assert_eq!(output.next(), Some(0.));
        assert_eq!(output.next(), Some(0.));
        assert_eq!(pcm.position_us.load(Ordering::Acquire), 5_000_000);
        let mut data = pcm.data.lock().unwrap();
        data.finished = true;
        data.error = Some("损坏音频".into());
        drop(data);
        assert_eq!(output.next(), Some(0.));
        assert!(!pcm.drained.load(Ordering::Acquire));
    }
}
