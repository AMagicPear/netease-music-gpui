use super::{
    download::{
        BLOCK_BYTES, CACHE_BLOCKS, CacheData, CacheReader, Demand, DownloadError, content_range,
    },
    output::Chunk,
    *,
};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{Condvar, Mutex},
};
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

fn read_frame(output: &mut BufferedSource) -> Option<Vec<f32>> {
    let mut frame = vec![0.; output.channels as usize];
    output.read_frame(&mut frame).then_some(frame)
}

async fn next_audible(output: &mut BufferedSource) -> f32 {
    // 回调碰到生产者持锁会输出一帧静音，这是设计行为；测试等待真正消费 PCM。
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let sample = read_frame(output).expect("播放源不应在取消前退出")[0];
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
    }
}

#[tokio::test]
async fn cache_is_bounded_and_seek_cannot_hide_terminal_download_errors() {
    let pcm = Arc::new(Pcm::default());
    let control = pcm.control.clone();
    let (demand, _receiver) = tokio::sync::watch::channel(Demand {
        offset: Some(0),
        generation: 0,
    });
    let cache = Arc::new(Cache {
        data: Mutex::new(CacheData {
            blocks: BTreeMap::new(),
            byte_len: Some(1024 * BLOCK_BYTES),
            range: true,
            error: None,
        }),
        changed: Condvar::new(),
        demand,
        control: control.clone(),
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
    cache.data.lock().unwrap().error = Some(DownloadError::Sequential("顺序连接已断开".into()));
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
    assert_eq!(
        read_frame(&mut source),
        Some(vec![0.; source.channels as usize])
    );
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
    assert_eq!(read_frame(&mut source), None);
}

#[tokio::test]
async fn dropping_audio_wakes_a_reader_waiting_for_a_stalled_range() {
    let mut server = serve(
        wav(12),
        true,
        None,
        Some((3 * BLOCK_BYTES, Arc::new(tokio::sync::Notify::new()))),
    )
    .await;
    let (audio, mut output) = open(&server).await;
    let cache = audio.cache.clone();
    let reader = tokio::task::spawn_blocking(move || {
        let mut reader = CacheReader {
            cache,
            position: 3 * BLOCK_BYTES,
            generation: 0,
            eof: Arc::new(AtomicBool::new(false)),
        };
        reader.read(&mut [0; 1])
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while server.requests.recv().await.unwrap() != 3 * BLOCK_BYTES {}
    })
    .await
    .unwrap();
    drop(audio);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), reader)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(read_frame(&mut output), None);
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
            assert!(read_frame(&mut source).is_some());
            std::thread::yield_now();
        }
        assert!(audio.error().is_none());
        assert_eq!(
            read_frame(&mut source),
            Some(vec![0.; source.channels as usize])
        );
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
async fn seeking_to_end_while_paused_finishes_on_output_and_can_seek_back() {
    for bytes in [
        include_bytes!("../../../tests/fixtures/tone.mp3").as_slice(),
        include_bytes!("../../../tests/fixtures/tone-vbr.mp3").as_slice(),
        include_bytes!("../../../tests/fixtures/tone.flac").as_slice(),
        include_bytes!("../../../tests/fixtures/tone.m4a").as_slice(),
    ] {
        let server = serve(bytes.to_vec(), true, None, None).await;
        let (audio, mut output) = open_with_duration(&server, Some(Duration::from_secs(1))).await;
        // 进度条的最大值使用整数毫秒，可能比精确时长少不足 1 ms。
        let end = Duration::from_millis(audio.duration.unwrap().as_millis() as u64);
        audio.seek(end);
        let pcm = audio.pcm.clone();
        tokio::task::spawn_blocking(move || {
            let data = pcm.data.lock().unwrap();
            let (data, timeout) = pcm
                .changed
                .wait_timeout_while(data, Duration::from_secs(3), |data| {
                    !data.decode_finished && data.error.is_none()
                })
                .unwrap();
            let error = data.error.clone();
            let empty = data.chunks.is_empty();
            drop(data);
            assert!(!timeout.timed_out());
            assert!(error.is_none(), "{error:?}");
            assert!(empty);
        })
        .await
        .unwrap();
        assert!(!audio.finished(), "暂停时尚未消费输出，不能触发切歌");
        assert_eq!(
            read_frame(&mut output),
            Some(vec![0.; output.channels as usize])
        );
        assert!(audio.finished());
        assert!(!audio.buffering());
        assert!(audio.error().is_none());
        audio.seek(Duration::from_millis(500));
        assert!(!audio.finished());
        wait_pcm(&audio.pcm).await;
        assert!(next_audible(&mut output).await.abs() > 0.);
        assert!(audio.error().is_none());
    }
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
            read_frame(&mut source);
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
        include_bytes!("../../../tests/fixtures/tone.mp3").as_slice(),
        include_bytes!("../../../tests/fixtures/tone-vbr.mp3").as_slice(),
        include_bytes!("../../../tests/fixtures/tone.flac").as_slice(),
        include_bytes!("../../../tests/fixtures/tone.m4a").as_slice(),
    ] {
        let server = serve(bytes.to_vec(), true, None, None).await;
        let (audio, mut source) = open_with_duration(&server, Some(Duration::from_secs(1))).await;
        assert!((audio.duration.unwrap().as_secs_f64() - 1.).abs() < 0.1);
        if bytes.starts_with(b"fLaC") {
            assert_eq!(source.sample_rate, 96000);
            assert_eq!(source.channels, 2);
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
                audible |= read_frame(&mut source).unwrap()[0].abs() > 0.001;
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
    let mut bytes = include_bytes!("../../../tests/fixtures/tone.flac").to_vec();
    let mut frame = 4;
    loop {
        let last = bytes[frame] & 0x80 != 0;
        let length = u32::from_be_bytes([0, bytes[frame + 1], bytes[frame + 2], bytes[frame + 3]]);
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
                read_frame(&mut output);
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
        assert_eq!(read_frame(&mut output), Some(vec![0., 0.]));
    }
    assert_eq!(pcm.position_us.load(Ordering::Acquire), 0);
    {
        let mut data = pcm.data.lock().unwrap();
        data.queued_samples = 2;
        data.chunks.push_back(Chunk {
            samples: vec![0.25, 0.75],
            position: Duration::ZERO,
            generation: 0,
        });
    }
    assert_eq!(read_frame(&mut output), Some(vec![0.25, 0.75]));
    assert_eq!(pcm.position_us.load(Ordering::Acquire), 1000);
    output.chunk = Some(Chunk {
        samples: vec![0.5, 0.5],
        position: Duration::ZERO,
        generation: 0,
    });
    pcm.control.generation.store(1, Ordering::Release);
    pcm.position_us.store(5_000_000, Ordering::Release);
    assert_eq!(read_frame(&mut output), Some(vec![0., 0.]));
    assert_eq!(pcm.position_us.load(Ordering::Acquire), 5_000_000);
    // 解码结束不代表输出结束，必须先消费最后一个完整声道帧。
    let mut data = pcm.data.lock().unwrap();
    data.queued_samples = 2;
    data.chunks.push_back(Chunk {
        samples: vec![0.25, 0.75],
        position: Duration::from_secs(5),
        generation: 1,
    });
    data.decode_finished = true;
    drop(data);
    assert_eq!(read_frame(&mut output), Some(vec![0.25, 0.75]));
    assert!(!pcm.output_drained.load(Ordering::Acquire));
    assert_eq!(read_frame(&mut output), Some(vec![0., 0.]));
    assert!(pcm.output_drained.load(Ordering::Acquire));

    let mut data = pcm.data.lock().unwrap();
    data.error = Some("损坏音频".into());
    drop(data);
    assert_eq!(read_frame(&mut output), Some(vec![0., 0.]));
    assert!(!pcm.output_drained.load(Ordering::Acquire));
    pcm.control.cancelled.store(true, Ordering::Release);
    let mut frame = [1_f32; 2];
    assert!(!output.read_frame(&mut frame));
    assert_eq!(frame, [0., 0.]);
}

#[test]
fn buffered_output_preserves_channels_rates_volume_and_seek() {
    fn pcm() -> Arc<Pcm> {
        let pcm = Arc::new(Pcm::default());
        let samples = vec![0.2, 0.6, 0.4, 0.8, 0.6, 1.0, 0.8, 0.4];
        let mut data = pcm.data.lock().unwrap();
        data.queued_samples = samples.len();
        data.chunks.push_back(Chunk {
            samples,
            position: Duration::ZERO,
            generation: 0,
        });
        drop(data);
        pcm
    }
    let shared = pcm();
    let mut output = source(shared.clone());
    let mut buffer = [1_f32; 4];
    output.write(&mut buffer, 0.5);
    assert_eq!(buffer, [0.1, 0.3, 0.2, 0.4]);
    assert_eq!(shared.position_us.load(Ordering::Acquire), 2000);

    for channels in [1, 2] {
        let mut output = source(pcm());
        output.channels = channels;
        let mut frames = [0_f32; 8];
        output.write(&mut frames, 1.);
        assert_eq!(frames, [0.2, 0.6, 0.4, 0.8, 0.6, 1.0, 0.8, 0.4]);
    }

    for rate in [44100, 48000, 96000] {
        let shared = pcm();
        let mut output = source(shared.clone());
        output.sample_rate = rate;
        let mut frames = [0_f32; 8];
        output.write(&mut frames, 1.);
        assert_eq!(frames, [0.2, 0.6, 0.4, 0.8, 0.6, 1.0, 0.8, 0.4]);
        assert_eq!(
            shared.position_us.load(Ordering::Acquire),
            4_000_000 / u64::from(rate)
        );
        shared.control.generation.store(1, Ordering::Release);
        let mut data = shared.data.lock().unwrap();
        data.queued_samples = 2;
        data.chunks.push_back(Chunk {
            samples: vec![-0.5, -0.5],
            position: Duration::from_secs(5),
            generation: 1,
        });
        drop(data);
        let mut frame = [0_f32; 2];
        output.write(&mut frame, 1.);
        assert_eq!(frame, [-0.5, -0.5], "seek must discard old PCM");
    }
}
