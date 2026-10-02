use super::song::Song;
use crate::api::MusicApi;
use gpui::{Context, ReadGlobal};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::{io::Cursor, time::Duration};

#[derive(Default)]
pub struct PlaybackState {
    pub current_song: Option<Song>,
    pub position: Duration,
    pub is_playing: bool,
    pub loading: bool,
    pub error: Option<String>,
    queue: Vec<Song>,
    stream: Option<OutputStream>,
    sink: Option<Sink>,
    audio_duration: Option<Duration>,
    generation: u64,
    play_when_ready: bool,
    request: Option<tokio::task::AbortHandle>,
}

impl PlaybackState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self::default()
    }

    pub fn play_from_queue(&mut self, songs: Vec<Song>, song_id: u64, cx: &mut Context<Self>) {
        let Some(song) = songs.iter().find(|song| song.id == song_id).cloned() else {
            return;
        };
        self.queue = songs;
        if self
            .current_song
            .as_ref()
            .is_some_and(|current| current.id == song_id)
            && (self.loading || self.sink.as_ref().is_some_and(|sink| !sink.empty()))
        {
            self.set_playing(true, cx);
        } else {
            self.select_song(song, cx);
        }
    }

    fn select_song(&mut self, song: Song, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        if let Some(request) = self.request.take() {
            request.abort();
        }
        // 丢弃旧 Sink 会立即停止旧音频；OutputStream 必须活到整个播放过程结束。
        self.sink = None;
        self.current_song = Some(song.clone());
        self.position = Duration::ZERO;
        self.audio_duration = None;
        self.is_playing = false;
        self.loading = false;
        self.error = None;
        self.play_when_ready = true;
        if self.stream.is_none() {
            match OutputStreamBuilder::open_default_stream() {
                Ok(mut stream) => {
                    stream.log_on_drop(false);
                    self.stream = Some(stream);
                }
                Err(error) => {
                    self.error = Some(format!("无法打开音频输出设备：{error}"));
                    cx.notify();
                    return;
                }
            }
        }
        self.loading = true;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let http = api.audio_http();
        let request = api.runtime.spawn(async move {
            let bytes = MusicApi::song_audio(client, http, song.id).await?;
            tokio::task::spawn_blocking(move || decode_audio(bytes))
                .await
                .map_err(|_| "音频解码任务失败".to_string())?
        });
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("音频加载任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                // 同一首歌也可能被重新请求，因此使用请求序号，而非只比较歌曲 ID。
                if this.generation != generation {
                    return;
                }
                this.request = None;
                this.loading = false;
                match result {
                    Ok(source) => {
                        this.audio_duration = source.total_duration();
                        let sink = Sink::connect_new(this.stream.as_ref().unwrap().mixer());
                        sink.pause();
                        sink.append(source);
                        this.is_playing = this.play_when_ready;
                        if this.is_playing {
                            sink.play();
                        }
                        this.sink = Some(sink);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn toggle_playing(&mut self, cx: &mut Context<Self>) {
        self.set_playing(!self.is_play_requested(), cx);
    }

    pub fn is_play_requested(&self) -> bool {
        if self.loading {
            self.play_when_ready
        } else {
            self.is_playing
        }
    }

    pub fn set_playing(&mut self, playing: bool, cx: &mut Context<Self>) {
        if self.loading {
            self.play_when_ready = playing;
        } else if let Some(sink) = self.sink.as_ref().filter(|sink| !sink.empty()) {
            if playing {
                sink.play();
            } else {
                sink.pause();
            }
            self.is_playing = playing;
        } else if playing && let Some(song) = self.current_song.clone() {
            self.select_song(song, cx);
        }
        cx.notify();
    }

    fn queued_song(&self, direction: isize) -> Option<Song> {
        let id = self.current_song.as_ref()?.id;
        let index = self.queue.iter().position(|song| song.id == id)?;
        self.queue
            .get(index.checked_add_signed(direction)?)
            .cloned()
    }

    pub fn change_song(&mut self, direction: isize, cx: &mut Context<Self>) {
        if let Some(song) = self.queued_song(direction) {
            self.select_song(song, cx);
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.is_playing {
            return;
        }
        let Some(sink) = &self.sink else { return };
        if sink.empty() {
            self.position = self.duration();
            self.is_playing = false;
            self.sink = None;
            self.change_song(1, cx);
        } else {
            // 从音频输出端读取时间，暂停、seek 后无需维护另一套计时器。
            self.position = sink.get_pos().min(self.duration());
        }
        cx.notify();
    }

    pub fn duration(&self) -> Duration {
        self.audio_duration.unwrap_or_else(|| {
            self.current_song
                .as_ref()
                .map_or(Duration::ZERO, Song::duration)
        })
    }

    pub fn can_seek(&self) -> bool {
        self.sink.as_ref().is_some_and(|sink| !sink.empty()) && !self.duration().is_zero()
    }

    pub fn seek_to_progress(&mut self, progress: f32) {
        if !progress.is_finite() || !self.can_seek() {
            return;
        }
        let position = self.duration().mul_f64(f64::from(progress.clamp(0., 1.)));
        match self.sink.as_ref().unwrap().try_seek(position) {
            Ok(()) => {
                self.position = position;
                self.error = None;
            }
            Err(error) => self.error = Some(format!("无法跳转播放位置：{error}")),
        }
    }

    pub fn progress(&self) -> f32 {
        let duration = self.duration().as_secs_f32();
        if duration == 0. {
            0.
        } else {
            (self.position.as_secs_f32() / duration).clamp(0., 1.)
        }
    }
}

fn decode_audio(bytes: Vec<u8>) -> Result<Decoder<Cursor<Vec<u8>>>, String> {
    // Cursor 的默认解码构造不会声明可 seek，显式提供长度才能可靠跳转和计算时长。
    let length = bytes.len() as u64;
    Decoder::builder()
        .with_data(Cursor::new(bytes))
        .with_byte_len(length)
        .build()
        .map_err(|error| format!("音频解码失败：{error}"))
}

impl Drop for PlaybackState {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.sink = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloaded_audio_can_decode_and_seek_without_an_output_device() {
        // 一秒、8 kHz、单声道 16-bit PCM 静音 WAV，不依赖网络或声卡。
        let data_len = 16000_u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&8000_u32.to_le_bytes());
        wav.extend_from_slice(&16000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.resize(44 + data_len as usize, 0);
        let mut source = decode_audio(wav).unwrap();
        assert_eq!(source.total_duration(), Some(Duration::from_secs(1)));
        source.try_seek(Duration::from_millis(500)).unwrap();
        assert!(source.next().is_some());
        assert!(decode_audio(vec![0; 32]).is_err());
    }

    #[test]
    fn progress_and_queue_respect_audio_duration_and_boundaries() {
        let mut playback = PlaybackState::default();
        assert_eq!(playback.progress(), 0.);
        playback.queue = [1, 2, 3]
            .map(|id| Song {
                id,
                dt: 10000,
                ..Default::default()
            })
            .into();
        playback.current_song = Some(playback.queue[0].clone());
        playback.position = Duration::from_secs(5);
        assert_eq!(playback.progress(), 0.5);
        assert!(playback.queued_song(-1).is_none());
        assert_eq!(playback.queued_song(1).unwrap().id, 2);
        playback.current_song = Some(playback.queue[2].clone());
        assert!(playback.queued_song(1).is_none());
        assert_eq!(playback.queued_song(-1).unwrap().id, 2);
        playback.audio_duration = Some(Duration::from_secs(5));
        assert_eq!(playback.progress(), 1.);
        // 未加载音频时不能伪造 seek 成功。
        playback.seek_to_progress(0.25);
        assert_eq!(playback.position, Duration::from_secs(5));
        playback.seek_to_progress(f32::NAN);
        assert_eq!(playback.position, Duration::from_secs(5));
    }
}
