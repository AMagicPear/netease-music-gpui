use super::{
    PlaybackSnapshot,
    engine::PlayerEngine,
    stream::{BufferedSource, StreamingAudio},
};
use crate::{
    api::MusicApi,
    models::{AudioQualityLevel, Song},
};
use gpui::{Context, ReadGlobal};
use std::time::Duration;

/// 共享 GPUI Entity：统一接收 UI 命令，管理队列、异步请求和状态通知。
#[derive(Default)]
pub struct PlaybackController {
    state: PlaybackSnapshot,
    engine: PlayerEngine,
    queue: Vec<Song>,
    generation: u64,
    play_when_ready: bool,
    request: Option<tokio::task::AbortHandle>,
    pending_position: Duration,
}

impl PlaybackController {
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

    pub fn snapshot(&self) -> &PlaybackSnapshot {
        &self.state
    }

    pub fn play_from_queue(&mut self, songs: Vec<Song>, song_id: u64, cx: &mut Context<Self>) {
        let Some(song) = songs.iter().find(|song| song.id == song_id).cloned() else {
            return;
        };
        self.queue = songs;
        if self
            .state
            .current_song
            .as_ref()
            .is_some_and(|current| current.id == song_id)
            && (self.state.loading || self.engine.has_source())
        {
            self.resume(cx);
        } else {
            self.select_song(song, cx);
        }
    }

    fn select_song(&mut self, song: Song, cx: &mut Context<Self>) {
        self.load_song(song, Duration::ZERO, true, cx);
    }

    fn load_song(&mut self, song: Song, position: Duration, play: bool, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.state.revision = generation;
        self.pending_position = position;
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.engine.stop();
        self.state.duration = song.duration();
        self.state.current_song = Some(song.clone());
        self.state.position = position.min(self.state.duration);
        self.state.is_playing = false;
        self.state.loading = false;
        self.state.buffering = false;
        self.state.actual_quality = None;
        self.state.error = None;
        self.play_when_ready = play;
        if let Err(error) = self.engine.ensure_device() {
            self.fail(error);
            cx.notify();
            return;
        }
        self.state.loading = true;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let http = api.audio_http();
        let quality = self.state.quality;
        let request = api.runtime.spawn(async move {
            let source = MusicApi::song_source(client, song.id, quality).await?;
            StreamingAudio::open(http, source).await
        });
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("音频加载任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.finish(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn finish(
        &mut self,
        generation: u64,
        result: Result<(StreamingAudio, BufferedSource), String>,
    ) -> bool {
        // 同一首歌也可能重新请求；旧结果在这里丢弃，资源会随之取消。
        if self.generation != generation {
            return false;
        }
        self.request = None;
        self.state.loading = false;
        match result {
            Ok((audio, source)) => {
                self.state.actual_quality = audio.quality;
                if let Some(duration) = audio.duration {
                    self.state.duration = duration;
                }
                match self.engine.load(audio, source) {
                    Ok(()) => {
                        let position = self.pending_position.min(self.state.duration);
                        if !position.is_zero() {
                            self.engine.seek_to(position);
                        }
                        self.state.position = position;
                        self.state.is_playing = self.play_when_ready && self.engine.resume();
                        self.state.buffering = self.state.is_playing && self.engine.buffering();
                    }
                    Err(error) => self.fail(error),
                }
            }
            Err(error) => self.fail(error),
        }
        true
    }

    pub fn is_play_requested(&self) -> bool {
        if self.state.loading {
            self.play_when_ready
        } else {
            self.state.is_playing
        }
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.play_when_ready = false;
        self.engine.pause();
        if self.engine.has_source() {
            self.state.position = self.engine.position().min(self.state.duration);
        }
        self.state.is_playing = false;
        self.state.buffering = false;
        cx.notify();
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        self.play_when_ready = true;
        if !self.state.loading {
            if self.engine.resume() {
                self.state.is_playing = true;
                self.state.buffering = self.engine.buffering();
            } else if let Some(song) = self.state.current_song.clone() {
                let position = if self.state.error.is_some() {
                    self.state.position
                } else {
                    Duration::ZERO
                };
                self.load_song(song, position, true, cx);
                return;
            }
        }
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.is_play_requested() {
            self.pause(cx);
        } else {
            self.resume(cx);
        }
    }

    #[allow(dead_code, reason = "音质菜单已按要求回退，保留播放控制接口")]
    pub fn set_quality(&mut self, quality: AudioQualityLevel, cx: &mut Context<Self>) {
        if self.state.quality == quality {
            return;
        }
        let play = self.is_play_requested();
        let position = if self.engine.has_source() {
            self.engine.position()
        } else {
            self.state.position
        };
        self.state.quality = quality;
        if let Some(song) = self.state.current_song.clone() {
            self.load_song(song, position, play, cx);
        } else {
            cx.notify();
        }
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        self.change_song(-1, cx);
    }
    pub fn next(&mut self, cx: &mut Context<Self>) {
        self.change_song(1, cx);
    }

    fn queued_song(&self, direction: isize) -> Option<Song> {
        let id = self.state.current_song.as_ref()?.id;
        let index = self.queue.iter().position(|song| song.id == id)?;
        self.queue
            .get(index.checked_add_signed(direction)?)
            .cloned()
    }

    fn change_song(&mut self, direction: isize, cx: &mut Context<Self>) {
        if let Some(song) = self.queued_song(direction) {
            self.select_song(song, cx);
        }
    }

    pub fn can_seek(&self) -> bool {
        self.engine.has_source() && !self.state.duration.is_zero()
    }

    pub fn seek_to(&mut self, position: Duration, cx: &mut Context<Self>) {
        let position = position.min(self.state.duration);
        if self.can_seek() && self.engine.seek_to(position) {
            self.state.position = position;
            self.state.error = None;
            self.state.buffering = self.state.is_playing;
            cx.notify();
        }
    }

    /// 0..=1；修改当前播放音量，并在后续切歌或重试时保留。
    #[allow(dead_code, reason = "本轮提供音量接口，后续音量 UI 接入时移除此标记")]
    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        if self.engine.set_volume(volume) {
            self.state.volume = self.engine.volume();
            cx.notify();
        }
    }

    pub fn progress(&self) -> f32 {
        let duration = self.state.duration.as_secs_f32();
        if duration == 0. {
            0.
        } else {
            (self.state.position.as_secs_f32() / duration).clamp(0., 1.)
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = self.engine.error() {
            self.fail(error);
            cx.notify();
            return;
        }
        if !self.state.is_playing {
            return;
        }
        self.state.buffering = self.engine.buffering();
        if self.engine.finished() {
            self.state.position = self.state.duration;
            self.state.is_playing = false;
            self.state.buffering = false;
            self.engine.stop();
            self.next(cx);
        } else {
            // 从真实 PCM 消费量取进度，等待数据的静音不计入歌曲时间。
            self.state.position = self.engine.position().min(self.state.duration);
        }
        cx.notify();
    }

    fn fail(&mut self, error: String) {
        if self.engine.has_source() {
            self.state.position = self.engine.position().min(self.state.duration);
        }
        eprintln!("{error}");
        self.state.error = Some(error);
        self.state.loading = false;
        self.state.is_playing = false;
        self.state.buffering = false;
        self.engine.stop();
    }
}

impl Drop for PlaybackController {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.engine.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_progress_and_queue_keep_boundaries() {
        let mut controller = PlaybackController::default();
        assert_eq!(controller.progress(), 0.);
        assert_eq!(controller.snapshot().volume, 1.);
        controller.queue = [1, 2, 3]
            .map(|id| Song {
                id,
                dt: 10000,
                ..Default::default()
            })
            .into();
        controller.state.current_song = Some(controller.queue[0].clone());
        controller.state.duration = Duration::from_secs(10);
        controller.state.position = Duration::from_secs(5);
        assert_eq!(controller.progress(), 0.5);
        assert!(controller.queued_song(-1).is_none());
        assert_eq!(controller.queued_song(1).unwrap().id, 2);
        controller.state.current_song = Some(controller.queue[2].clone());
        assert!(controller.queued_song(1).is_none());
        assert_eq!(controller.queued_song(-1).unwrap().id, 2);
        controller.state.duration = Duration::from_secs(5);
        assert_eq!(controller.progress(), 1.);
        assert!(!controller.can_seek());
    }

    #[test]
    fn obsolete_load_cannot_replace_current_snapshot() {
        let mut controller = PlaybackController::default();
        controller.generation = 3;
        controller.state.loading = true;
        assert!(!controller.finish(1, Err("旧请求失败".into())));
        assert!(controller.snapshot().loading);
        assert!(controller.snapshot().error.is_none());
        assert!(controller.finish(3, Err("当前请求失败".into())));
        assert!(!controller.snapshot().loading);
        assert_eq!(controller.snapshot().error.as_deref(), Some("当前请求失败"));
    }
}
