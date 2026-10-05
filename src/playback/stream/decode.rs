use super::{
    download::{Cache, CacheReader},
    output::{BufferedSource, Chunk, Pcm},
};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{CODEC_TYPE_FLAC, CODEC_TYPE_MP3, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, SeekMode, SeekTo},
    io::MediaSourceStream,
    probe::Hint,
    units::{Time, TimeBase},
};

const PCM_CHUNKS: usize = 8;
pub(super) type Ready = tokio::sync::oneshot::Sender<(BufferedSource, Option<Duration>)>;

/// 每次 seek 重建读取器和解码器；失败和被打断的读取不会留下损坏的解析状态。
pub(super) fn decode(
    cache: Arc<Cache>,
    pcm: Arc<Pcm>,
    ready: Ready,
    source_duration: Option<Duration>,
) {
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
        if pcm.control.is_current(generation) {
            match result {
                Ok(()) => data.decode_finished = true,
                Err(error) => data.error = Some(error),
            }
            pcm.changed.notify_all();
            if ready.is_some() {
                return;
            }
        }
        while data.seek.is_none() && !pcm.control.cancelled.load(Ordering::Acquire) {
            data = pcm.changed.wait(data).unwrap();
        }
        if pcm.control.cancelled.load(Ordering::Acquire) {
            return;
        }
        target = data.seek.take().unwrap();
        generation = pcm.control.generation.load(Ordering::Acquire);
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
        if !pcm.control.is_current(generation) {
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
                && pcm.control.is_current(generation)
            {
                data = pcm.changed.wait(data).unwrap();
            }
            if !pcm.control.is_current(generation) {
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
