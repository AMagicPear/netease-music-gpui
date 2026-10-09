use super::StreamControl;
use crate::models::AudioSourceInfo;
use crate::playback::audio_cache::CachedFile;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use symphonia::core::io::MediaSource;

pub(super) const BLOCK_BYTES: u64 = 256 * 1024;
const RANGE_BYTES: u64 = 4 * 1024 * 1024;
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
    // 只保存各块已经落盘的连续字节数，音频数据不常驻应用内存。
    pub blocks: BTreeMap<u64, usize>,
    pub byte_len: Option<u64>,
    pub error: Option<DownloadError>,
    pub complete: bool,
}

/// 下载与解码共享的磁盘文件；只发布写入成功的区间，缺失区间优先于后台整曲下载。
pub(super) struct Cache {
    pub data: Mutex<CacheData>,
    pub changed: Condvar,
    pub demand: tokio::sync::watch::Sender<Demand>,
    pub control: Arc<StreamControl>,
    // 字段按声明顺序释放，文件句柄先关闭，再释放磁盘租约（Windows 也能清理）。
    file: Mutex<File>,
    disk: Arc<CachedFile>,
}

impl Cache {
    pub async fn open(
        http: reqwest::Client,
        source: &AudioSourceInfo,
        control: Arc<StreamControl>,
        disk: Arc<CachedFile>,
    ) -> Result<(Arc<Self>, tokio::task::AbortHandle), String> {
        let reader = disk.clone();
        let (file, completed_size) = tokio::task::spawn_blocking(move || reader.open_reader())
            .await
            .map_err(|error| format!("无法打开音频缓存：{error}"))??;
        if let Some(total) = completed_size {
            let (demand, _) = tokio::sync::watch::channel(Demand {
                offset: Some(0),
                generation: 0,
            });
            let cache = Arc::new(Self {
                data: Mutex::new(CacheData {
                    blocks: (0..total)
                        .step_by(BLOCK_BYTES as usize)
                        .map(|offset| (offset, (total - offset).min(BLOCK_BYTES) as usize))
                        .collect(),
                    byte_len: Some(total),
                    error: None,
                    complete: true,
                }),
                changed: Condvar::new(),
                demand,
                control,
                file: Mutex::new(file),
                disk,
            });
            return Ok((cache, tokio::spawn(async {}).abort_handle()));
        }
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
        if byte_len.is_some_and(|total| total > disk.limit()) {
            return Err("音频文件超过单曲缓存容量上限".into());
        }
        if let (Some(actual), Some(expected)) = (byte_len, source.byte_len)
            && actual > expected
        {
            return Err("音频文件长度超过播放接口声明的大小".into());
        }
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
                error: None,
                complete: false,
            }),
            changed: Condvar::new(),
            demand,
            control,
            file: Mutex::new(file),
            disk,
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
            } else if let Err(error) = download_cache.clone().sequential(response).await {
                let mut data = download_cache.data.lock().unwrap();
                data.error = Some(DownloadError::Sequential(error));
                download_cache.changed.notify_all();
            }
        });
        Ok((cache, task.abort_handle()))
    }

    pub fn insert(&self, offset: u64, bytes: Vec<u8>) -> Result<(), String> {
        if offset.saturating_add(bytes.len() as u64) > self.disk.limit() {
            return Err("音频文件超过单曲缓存容量上限".into());
        }
        {
            let mut file = self.file.lock().unwrap();
            file.seek(SeekFrom::Start(offset))
                .and_then(|_| file.write_all(&bytes))
                .map_err(|error| format!("写入音频缓存失败：{error}"))?;
        }
        let mut data = self.data.lock().unwrap();
        let mut position = offset;
        let end = offset + bytes.len() as u64;
        while position < end {
            let block = position / BLOCK_BYTES * BLOCK_BYTES;
            let count = (end - position).min(BLOCK_BYTES - position % BLOCK_BYTES);
            let available = data.blocks.entry(block).or_default();
            if position - block > *available as u64 {
                return Err("音频缓存区间不连续".into());
            }
            *available = (*available).max((position - block + count) as usize);
            position += count;
        }
        self.changed.notify_all();
        Ok(())
    }

    async fn publish(self: &Arc<Self>, offset: u64, bytes: Vec<u8>) -> Result<(), String> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || cache.insert(offset, bytes))
            .await
            .map_err(|error| format!("音频缓存写入任务失败：{error}"))?
    }

    async fn finish(self: &Arc<Self>, total: u64) {
        let cache = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            cache
                .file
                .lock()
                .unwrap()
                .sync_all()
                .map_err(|error| error.to_string())?;
            cache.disk.commit(total)
        })
        .await;
        if let Err(error) = result.unwrap_or_else(|error| Err(error.to_string())) {
            // 数据已经可播放；索引写入失败只影响下一次复用。
            eprintln!("提交音频缓存失败：{error}");
        }
        self.data.lock().unwrap().complete = true;
        self.changed.notify_all();
    }

    pub fn invalidate(&self) {
        self.disk.invalidate();
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

    async fn sequential(self: Arc<Self>, mut response: reqwest::Response) -> Result<(), String> {
        let mut offset = 0;
        loop {
            let chunk = tokio::time::timeout(NETWORK_TIMEOUT, response.chunk())
                .await
                .map_err(|_| "音频下载超时".to_string())?
                .map_err(|error| format!("音频下载中断：{error}"))?;
            if self.control.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            let Some(chunk) = chunk else {
                {
                    let mut data = self.data.lock().unwrap();
                    if offset == 0 || data.byte_len.is_some_and(|length| length != offset) {
                        return Err("音频文件不完整，请重试".into());
                    }
                    data.byte_len = Some(offset);
                }
                self.changed.notify_all();
                self.finish(offset).await;
                return Ok(());
            };
            self.publish(offset, chunk.to_vec()).await?;
            offset += chunk.len() as u64;
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
        || end != offset.saturating_add(RANGE_BYTES - 1).min(total - 1)
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
            format!("bytes={offset}-{}", offset.saturating_add(RANGE_BYTES - 1)),
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
    cache: &Arc<Cache>,
) -> Result<(), String> {
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
    let mut received = 0;
    while let Some(chunk) = tokio::time::timeout(NETWORK_TIMEOUT, response.chunk())
        .await
        .map_err(|_| "音频下载超时".to_string())?
        .map_err(|error| format!("音频下载中断：{error}"))?
    {
        if received + chunk.len() as u64 > length {
            return Err("音频分段数据超过声明长度".into());
        }
        // 每批字节只复制一次、落盘一次；不用复制整个累计块，也不用等满块才解码。
        cache.publish(offset + received, chunk.to_vec()).await?;
        received += chunk.len() as u64;
    }
    if received != length {
        return Err("音频分段数据不完整，请重试".into());
    }
    Ok(())
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
    'download: loop {
        let requested = *demand.borrow_and_update();
        {
            let start = requested.offset.unwrap_or(0);
            let offset = {
                let data = cache.data.lock().unwrap();
                // 先完成读取位置之后的区间，再补齐 seek 留下的洞，一路缓存整首。
                (start..total)
                    .step_by(BLOCK_BYTES as usize)
                    .chain((0..start.min(total)).step_by(BLOCK_BYTES as usize))
                    .find(|offset| {
                        data.blocks
                            .get(offset)
                            .is_none_or(|length| *length as u64 != BLOCK_BYTES.min(total - offset))
                    })
            };
            if let Some(offset) = offset {
                let fetch = async {
                    let mut last_error = String::new();
                    for attempt in 0..3 {
                        let response = if offset == 0
                            && let Some(response) = initial.take()
                        {
                            Ok(response)
                        } else {
                            request_range(&http, &url, offset, validator.as_deref()).await
                        };
                        let result = match response {
                            Ok(response) => range_bytes(response, offset, total, &cache).await,
                            Err(error) => Err(error),
                        };
                        match result {
                            Ok(()) => return Ok(()),
                            Err(error) => last_error = error,
                        }
                        if attempt < 2 {
                            tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
                        }
                    }
                    Err(last_error)
                };
                tokio::pin!(fetch);
                // seek 后的 None 指令和文件头读取可能被 watch 合并；以实际请求起点
                // 为基准，仍能识别随后直接跳到目标区间的读取。
                let mut active_demand = Demand {
                    offset: Some(offset),
                    generation: requested.generation,
                };
                let result = loop {
                    tokio::select! {
                        biased;
                        changed = demand.changed() => {
                            if changed.is_err() { return; }
                            let next = *demand.borrow_and_update();
                            let jumped = match (active_demand.offset, next.offset) {
                                (Some(previous), Some(next)) => next < previous || next > previous.saturating_add(BLOCK_BYTES),
                                _ => false,
                            };
                            active_demand = next;
                            // 读取已下载的数据不影响后台整曲下载；缺失的当前块优先。
                            let missing = next.offset.is_some_and(|start| {
                                let data = cache.data.lock().unwrap();
                                data.blocks.get(&start).is_none_or(|length| {
                                    *length as u64 != BLOCK_BYTES.min(total.saturating_sub(start))
                                })
                            });
                            if next.generation != requested.generation
                                || missing && next.offset.is_some_and(|start| {
                                    start < offset || start >= offset.saturating_add(RANGE_BYTES)
                                        || jumped && start != offset
                                })
                            {
                                continue 'download;
                            }
                        }
                        result = &mut fetch => break result,
                    }
                };
                match result {
                    Ok(()) => continue,
                    Err(error) => {
                        // 一个请求覆盖多个块：读者可能已推进到本请求后半段。
                        // 若它正等该请求内的缺失数据，必须唤醒报错，不能双方都干等。
                        let reader = *demand.borrow();
                        let needed = reader.offset.is_some_and(|start| {
                            let data = cache.data.lock().unwrap();
                            start >= offset
                                && start < offset.saturating_add(RANGE_BYTES)
                                && data.blocks.get(&start).is_none_or(|length| {
                                    (*length as u64) < BLOCK_BYTES.min(total.saturating_sub(start))
                                })
                        });
                        if needed {
                            cache.fail(requested.generation, error);
                        }
                    }
                }
            } else {
                cache.finish(total).await;
                return;
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
            let requested = Demand {
                offset: Some(offset),
                generation: self.generation,
            };
            // 命中缓存也推进预取窗口，不能等块耗尽才通知下载线程。
            self.cache.demand.send_if_modified(|current| {
                if *current == requested {
                    false
                } else {
                    *current = requested;
                    true
                }
            });
            if let Some(length) = data.blocks.get(&offset)
                && start < *length
            {
                let count = buffer.len().min(*length - start);
                // 文件 I/O 只在解码线程进行；设备回调始终只访问 PCM。
                drop(data);
                let mut file = self.cache.file.lock().unwrap();
                file.seek(SeekFrom::Start(self.position))?;
                file.read_exact(&mut buffer[..count])?;
                self.position += count as u64;
                return Ok(count);
            }
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
