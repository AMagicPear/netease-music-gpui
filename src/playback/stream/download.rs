use super::StreamControl;
use crate::models::AudioSourceInfo;
use std::{
    collections::BTreeMap,
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use symphonia::core::io::MediaSource;

pub(super) const BLOCK_BYTES: u64 = 256 * 1024;
pub(super) const CACHE_BLOCKS: usize = 128; // Range 缓存最多 32 MiB。
const MAX_SEQUENTIAL_BYTES: u64 = 256 * 1024 * 1024;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Demand {
    // None 只中断旧请求；新读取器遇到缺失数据后再提交真实区间。
    pub offset: Option<u64>,
    pub generation: u64,
}

pub(super) enum DownloadError {
    Range { generation: u64, message: String },
    // 顺序下载不可重新定位，失败后不能由 seek 清除。
    Sequential(String),
}

impl DownloadError {
    fn message(&self) -> &str {
        match self {
            Self::Range { message, .. } | Self::Sequential(message) => message,
        }
    }
}

pub(super) struct CacheData {
    pub blocks: BTreeMap<u64, Vec<u8>>,
    pub byte_len: Option<u64>,
    pub range: bool,
    pub error: Option<DownloadError>,
}

/// 当前播放音源的临时字节缓存；Range/顺序读取都不保存歌曲文件。
/// 不依赖 PCM 队列、音频输出或未来独立的歌曲下载模块。
pub(super) struct Cache {
    pub data: Mutex<CacheData>,
    pub changed: Condvar,
    pub demand: tokio::sync::watch::Sender<Demand>,
    pub control: Arc<StreamControl>,
}

impl Cache {
    pub async fn open(
        http: reqwest::Client,
        source: &AudioSourceInfo,
        control: Arc<StreamControl>,
    ) -> Result<(Arc<Self>, tokio::task::AbortHandle), String> {
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
        let (demand, receiver) = tokio::sync::watch::channel(Demand {
            offset: Some(0),
            generation: 0,
        });
        let cache = Arc::new(Self {
            data: Mutex::new(CacheData {
                blocks: BTreeMap::new(),
                byte_len,
                range,
                error: None,
            }),
            changed: Condvar::new(),
            demand,
            control,
        });
        let download_cache = cache.clone();
        let url = source.url.clone();
        let task = tokio::spawn(async move {
            if range {
                download_ranges(
                    download_cache,
                    http,
                    url,
                    byte_len.unwrap(),
                    response,
                    validator,
                    receiver,
                )
                .await;
            } else if let Err(error) = download_cache.sequential(response).await {
                let mut data = download_cache.data.lock().unwrap();
                data.error = Some(DownloadError::Sequential(error));
                download_cache.changed.notify_all();
            }
        });
        Ok((cache, task.abort_handle()))
    }

    pub fn insert(&self, offset: u64, bytes: Vec<u8>) {
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

    pub fn fail(&self, generation: u64, message: String) {
        let mut data = self.data.lock().unwrap();
        if self.control.is_current(generation) {
            data.error = Some(DownloadError::Range {
                generation,
                message,
            });
            self.changed.notify_all();
        }
    }

    pub fn interrupt(&self, generation: u64) {
        let mut data = self.data.lock().unwrap();
        if matches!(data.error, Some(DownloadError::Range { .. })) {
            data.error = None;
        }
        self.demand.send_replace(Demand {
            offset: None,
            generation,
        });
        self.changed.notify_all();
    }

    pub fn error(&self) -> Option<String> {
        self.data
            .try_lock()
            .ok()
            .and_then(|data| data.error.as_ref().map(|error| error.message().to_owned()))
    }

    async fn sequential(&self, mut response: reqwest::Response) -> Result<(), String> {
        let mut offset = 0;
        loop {
            let chunk = tokio::time::timeout(NETWORK_TIMEOUT, response.chunk())
                .await
                .map_err(|_| "音频下载超时".to_string())?
                .map_err(|error| format!("音频下载中断：{error}"))?;
            let mut data = self.data.lock().unwrap();
            if self.control.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            let Some(chunk) = chunk else {
                if offset == 0 || data.byte_len.is_some_and(|length| length != offset) {
                    return Err("音频文件不完整，请重试".into());
                }
                data.byte_len = Some(offset);
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
pub(super) fn content_range(
    response: &reqwest::Response,
    offset: u64,
) -> Result<(u64, u64), String> {
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
        if let Some(offset) = requested.offset {
            let cached = cache
                .data
                .lock()
                .unwrap()
                .blocks
                .get(&offset)
                .is_some_and(|bytes| {
                    bytes.len() as u64 == BLOCK_BYTES.min(total.saturating_sub(offset))
                });
            if !cached && offset < total {
                let fetch = async {
                    let response = if offset == 0
                        && let Some(response) = initial.take()
                    {
                        response
                    } else {
                        request_range(&http, &url, offset, validator.as_deref()).await?
                    };
                    range_bytes(response, offset, total, &cache).await
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
                    Ok(bytes) => cache.insert(offset, bytes),
                    Err(error) => cache.fail(requested.generation, error),
                }
            }
        }
        if demand.changed().await.is_err() {
            return;
        }
    }
}

/// 仅解码线程等待数据；每个读者绑定一个 seek 代次，旧读者会立即退出等待。
pub(super) struct CacheReader {
    pub cache: Arc<Cache>,
    pub position: u64,
    pub generation: u64,
    pub eof: Arc<AtomicBool>,
}

impl CacheReader {
    fn check(&self, data: &CacheData) -> io::Result<()> {
        if !self.cache.control.is_current(self.generation) {
            // 不能用 Interrupted：read_exact 会自动重试，导致旧请求无法退出。
            return Err(io::Error::other("播放或跳转已更新"));
        }
        if let Some(error) = &data.error {
            match error {
                DownloadError::Range {
                    generation,
                    message,
                } if *generation == self.generation => {
                    return Err(io::Error::other(message.clone()));
                }
                DownloadError::Sequential(message) => {
                    return Err(io::Error::other(message.clone()));
                }
                _ => {}
            }
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
                offset: Some(offset),
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
