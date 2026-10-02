use super::audio::StreamingAudio;
use super::song::Song;
use crate::api::MusicApi;
use gpui::{Context, ReadGlobal};
use rodio::{OutputStream, OutputStreamBuilder, Sink};
use std::time::Duration;

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
    audio: Option<StreamingAudio>,
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
        self.audio = None;
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
                    self.fail(format!("无法打开音频输出设备：{error}"));
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
            let response = MusicApi::song_stream(client, http, song.id).await?;
            StreamingAudio::start(response).await
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
                    Ok((audio, source)) => {
                        this.audio_duration = audio.duration;
                        let sink = Sink::connect_new(this.stream.as_ref().unwrap().mixer());
                        sink.pause();
                        sink.append(source);
                        this.is_playing = this.play_when_ready;
                        if this.is_playing {
                            sink.play();
                        }
                        this.sink = Some(sink);
                        this.audio = Some(audio);
                    }
                    Err(error) => this.fail(error),
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
        if let Some(error) = self.audio.as_ref().and_then(StreamingAudio::error) {
            self.fail(error);
            cx.notify();
            return;
        }
        if !self.is_playing {
            return;
        }
        let Some(sink) = &self.sink else { return };
        if sink.empty() {
            self.position = self.duration();
            self.is_playing = false;
            self.sink = None;
            self.audio = None;
            self.change_song(1, cx);
        } else {
            // 只计已输出的歌曲样本，缓冲期间的静音不计入歌曲进度。
            self.position = self
                .audio
                .as_ref()
                .map_or(Duration::ZERO, StreamingAudio::position)
                .min(self.duration());
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
        if let Some(audio) = &self.audio {
            // 只提交目标，解码线程负责等待下载和 seek，界面线程立即返回。
            audio.seek(position);
            self.position = position;
            self.error = None;
        }
    }

    fn fail(&mut self, error: String) {
        eprintln!("{error}");
        self.error = Some(error);
        self.loading = false;
        self.is_playing = false;
        self.sink = None;
        self.audio = None;
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

impl Drop for PlaybackState {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.sink = None;
        self.audio = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
