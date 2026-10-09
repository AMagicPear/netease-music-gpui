use super::{
    PlaybackSnapshot,
    engine::PlayerEngine,
    stream::{BufferedSource, StreamingAudio},
    system_media::SystemMedia,
};
use crate::{
    api::MusicApi,
    models::{AudioQualityLevel, Song},
    persistence::{Persistence, PlaybackState},
};
use gpui::{Context, ReadGlobal, Window};
use souvlaki::{MediaControlEvent, SeekDirection};
use std::time::Duration;

/// 共享 GPUI Entity：统一接收 UI 命令，管理队列、异步请求和状态通知。
#[derive(Default)]
pub struct PlaybackController {
    state: PlaybackSnapshot,
    engine: PlayerEngine,
    queue: Vec<Song>,
    playlist_id: Option<u64>,
    play_when_ready: bool,
    request: Option<tokio::task::AbortHandle>,
    pending_position: Duration,
    system_media: Option<SystemMedia>,
    persistence: Persistence,
}

impl PlaybackController {
    pub fn new(window: &Window, persistence: Persistence, cx: &mut Context<Self>) -> Self {
        let mut this = Self::default();
        this.persistence = persistence;
        if let Some(cache) = this.persistence.load_playback() {
            this.queue = cache.queue;
            this.playlist_id = Some(cache.playlist_id);
            if let Some(song) = this
                .queue
                .iter()
                .find(|song| song.id == cache.song_id)
                .cloned()
            {
                this.load_song(song, cache.position, cache.was_playing, cx);
            }
        }
        match SystemMedia::new(window) {
            Ok((media, mut events)) => {
                this.system_media = Some(media);
                // 原生回调只发送命令，Entity 的修改始终回到 GPUI 线程。
                cx.spawn(async move |this, cx| {
                    while let Some(event) = events.recv().await {
                        if this
                            .update(cx, |this, cx| this.handle_media_event(event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
                cx.observe_self(|this, _| {
                    if let Some(media) = &mut this.system_media {
                        if let Err(error) = media.sync(&this.state, this.engine.has_source()) {
                            eprintln!("同步系统媒体控件失败：{error}");
                        }
                    }
                })
                .detach();
            }
            Err(error) => eprintln!("系统媒体控件初始化失败：{error}"),
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(1000))
                    .await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn handle_media_event(&mut self, event: MediaControlEvent, cx: &mut Context<Self>) {
        match event {
            MediaControlEvent::Play => self.resume(cx),
            MediaControlEvent::Pause => self.pause(cx),
            MediaControlEvent::Toggle => self.toggle(cx),
            MediaControlEvent::Previous => self.previous(cx),
            MediaControlEvent::Next => self.next(cx),
            MediaControlEvent::Stop => {
                self.stop();
                cx.notify();
            }
            MediaControlEvent::SetPosition(position) => self.seek_to(position.0, cx),
            MediaControlEvent::Seek(direction) => {
                self.seek_by(direction, Duration::from_secs(10), cx)
            }
            MediaControlEvent::SeekBy(direction, offset) => self.seek_by(direction, offset, cx),
            MediaControlEvent::SetVolume(volume) if volume.is_finite() => {
                self.set_volume(volume.clamp(0., 1.) as f32, cx)
            }
            MediaControlEvent::Raise => crate::desktop::show_window(cx),
            MediaControlEvent::Quit => cx.quit(),
            _ => {}
        }
    }

    fn seek_by(&mut self, direction: SeekDirection, offset: Duration, cx: &mut Context<Self>) {
        let position = if self.engine.has_source() {
            self.engine.position()
        } else {
            self.state.position
        };
        let position = match direction {
            SeekDirection::Forward => position.saturating_add(offset),
            SeekDirection::Backward => position.saturating_sub(offset),
        };
        self.seek_to(position, cx);
    }

    fn stop(&mut self) {
        // 使停止之前的加载结果失效，防止异步完成后重新开始播放。
        self.state.revision = self.state.revision.wrapping_add(1);
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.play_when_ready = false;
        self.pending_position = Duration::ZERO;
        self.engine.stop();
        self.state.position = Duration::ZERO;
        self.state.is_playing = false;
        self.state.loading = false;
        self.state.buffering = false;
        self.state.error = None;
    }

    pub fn snapshot(&self) -> &PlaybackSnapshot {
        &self.state
    }

    pub fn play_from_queue(
        &mut self,
        playlist_id: u64,
        songs: Vec<Song>,
        song_id: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(song) = songs.iter().find(|song| song.id == song_id).cloned() else {
            return;
        };
        self.queue = songs;
        self.playlist_id = Some(playlist_id);
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
        self.state.revision = self.state.revision.wrapping_add(1);
        let load_revision = self.state.revision;
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
                if this.finish(load_revision, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn finish(
        &mut self,
        load_revision: u64,
        result: Result<(StreamingAudio, BufferedSource), String>,
    ) -> bool {
        // 同一首歌也可能重新请求；旧结果在这里丢弃，资源会随之取消。
        if self.state.revision != load_revision {
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

    fn tick(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = self.engine.error() {
            self.fail(error);
            cx.notify();
            return;
        }
        if !self.state.is_playing {
            self.save_cache();
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
        self.save_cache();
        cx.notify();
    }

    fn save_cache(&self) {
        let (Some(playlist_id), Some(song)) = (self.playlist_id, self.state.current_song.as_ref())
        else {
            return;
        };
        let cache = PlaybackState {
            playlist_id,
            queue: self.queue.clone(),
            song_id: song.id,
            position: self.state.position,
            was_playing: self.is_play_requested(),
        };
        if let Err(error) = self.persistence.save_playback(&cache) {
            eprintln!("保存播放缓存失败：{error}");
        }
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
        self.save_cache();
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
    fn system_stop_invalidates_pending_load_and_resets_position() {
        let mut controller = PlaybackController::default();
        controller.state.revision = 3;
        controller.play_when_ready = true;
        controller.state.loading = true;
        controller.state.position = Duration::from_secs(12);
        controller.pending_position = controller.state.position;
        controller.stop();
        assert!(!controller.finish(3, Err("停止前的请求".into())));
        assert!(!controller.is_play_requested());
        assert!(!controller.state.loading);
        assert!(!controller.state.buffering);
        assert_eq!(controller.state.position, Duration::ZERO);
        assert_eq!(controller.pending_position, Duration::ZERO);
        assert!(controller.state.error.is_none());
        assert_eq!(controller.state.revision, 4);
    }

    #[test]
    fn queue_keeps_boundaries() {
        let mut controller = PlaybackController::default();
        controller.queue = [1, 2, 3]
            .map(|id| Song {
                id,
                ..Default::default()
            })
            .into();
        controller.state.current_song = Some(controller.queue[0].clone());
        assert!(controller.queued_song(-1).is_none());
        assert_eq!(controller.queued_song(1).unwrap().id, 2);
        controller.state.current_song = Some(controller.queue[2].clone());
        assert!(controller.queued_song(1).is_none());
        assert_eq!(controller.queued_song(-1).unwrap().id, 2);
        assert!(!controller.can_seek());
    }

    #[test]
    fn obsolete_load_cannot_replace_current_snapshot() {
        let mut controller = PlaybackController::default();
        controller.state.revision = 3;
        controller.state.loading = true;
        assert!(!controller.finish(1, Err("旧请求失败".into())));
        assert!(controller.snapshot().loading);
        assert!(controller.snapshot().error.is_none());
        assert!(controller.finish(3, Err("当前请求失败".into())));
        assert!(!controller.snapshot().loading);
        assert_eq!(controller.snapshot().error.as_deref(), Some("当前请求失败"));
    }
}
