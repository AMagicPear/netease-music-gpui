use super::{
    PlaybackSnapshot,
    audio_cache::{AudioCacheStore, DEFAULT_CAPACITY, DEFAULT_MAX_FILE_BYTES},
    cover_art::CoverArtCache,
    engine::PlayerEngine,
    stream::{BufferedSource, StreamingAudio},
    system_media::SystemMedia,
};
use crate::{
    api::MusicApi,
    models::{AudioQualityLevel, PlayMode, Song},
    persistence::{Persistence, PlaybackState},
};
use gpui::{App, Context, ReadGlobal, Window};
use rand::{Rng, seq::SliceRandom};
use souvlaki::{MediaControlEvent, SeekDirection};
use std::{sync::Arc, time::Duration};

type AudioRequest = tokio::task::JoinHandle<Result<(StreamingAudio, BufferedSource), String>>;

struct CancelAudioOnDrop(AudioRequest);

impl Drop for CancelAudioOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Preload {
    song: Song,
    quality: AudioQualityLevel,
    request: AudioRequest,
}

/// 共享 GPUI Entity：统一接收 UI 命令，管理播放列表、异步请求和状态通知。
pub struct PlaybackController {
    state: PlaybackSnapshot,
    engine: PlayerEngine,
    /// 当前播放列表。它是一份自己的顺序，和任何页面上的歌单互不影响：
    /// 换来源、洗牌、插入歌曲都只改这里，页面歌单的排序或过滤不会带走播放。
    queue: Vec<Song>,
    /// 当前歌曲在 `queue` 中的位置；有它就不必按歌曲 id 反查，列表里有重复歌曲也没问题。
    queue_cursor: Option<usize>,
    /// 列表的来源歌单，只用来显示「来自哪个歌单」。
    queue_source: Option<u64>,
    /// 队列内容每变一次（换列表、洗牌、插歌、移游标）就自增。UI 靠它判断
    /// 播放列表面板是否需要重绘：不能只盯 `state.revision`，因为重播同一首
    /// 已加载的歌时列表可以整体换掉而不重新加载。
    queue_revision: u64,
    play_when_ready: bool,
    request: Option<tokio::task::AbortHandle>,
    pending_position: Duration,
    system_media: Option<SystemMedia>,
    persistence: Persistence,
    audio_cache: Arc<tokio::sync::OnceCell<Arc<AudioCacheStore>>>,
    preload: Option<Preload>,
    /// tick 每 100 ms 运行一次；攒够 5 秒才写一次状态，关键事件仍立即写。
    ticks_since_save: u32,
    /// 最后释放：测试临时目录必须覆盖控制器 Drop 中的保存和 flush。
    #[cfg(test)]
    test_directory: Option<tempfile::TempDir>,
}

/// 播放中周期性落盘的间隔（tick 数）。暂停时 tick 不再写盘。
const SAVE_INTERVAL_TICKS: u32 = 50;
const PRELOAD_REMAINING: Duration = Duration::from_secs(60);

impl PlaybackController {
    /// 仅构造空控制器；存储由调用方明确注入，不启动 UI、网络或播放任务。
    pub(crate) fn with_persistence(persistence: Persistence) -> Self {
        Self {
            state: PlaybackSnapshot::default(),
            engine: PlayerEngine::default(),
            queue: Vec::new(),
            queue_cursor: None,
            queue_source: None,
            queue_revision: 0,
            play_when_ready: false,
            request: None,
            pending_position: Duration::ZERO,
            system_media: None,
            persistence,
            audio_cache: Arc::default(),
            preload: None,
            ticks_since_save: 0,
            #[cfg(test)]
            test_directory: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        let directory = tempfile::tempdir().expect("无法创建测试存储目录");
        let mut this = Self::with_persistence(Persistence::at(directory.path().to_owned()));
        this.test_directory = Some(directory);
        this
    }

    pub fn new(window: &Window, persistence: Persistence, cx: &mut Context<Self>) -> Self {
        let mut this = Self::with_persistence(persistence);
        if let Some(cache) = this.persistence.load_playback() {
            let position = cache.position;
            this.restore_list(cache);
            if let Some(song) = this.current_list_song() {
                // 启动时只恢复列表与进度，不自动播放；等用户手动按下播放。
                this.load_song(song, position, false, cx);
            }
        }
        let covers = Arc::new(CoverArtCache::new(this.persistence.cover_cache_directory()));
        match SystemMedia::new(window, covers) {
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
                cx.observe_self(|this, cx| {
                    if let Some(media) = &mut this.system_media {
                        if let Err(error) = media.sync(&this.state, this.engine.has_source()) {
                            eprintln!("同步系统媒体控件失败：{error}");
                        }
                    }
                    this.sync_media_cover(cx);
                })
                .detach();
            }
            Err(error) => eprintln!("系统媒体控件初始化失败：{error}"),
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
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
        self.cancel_preload();
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

    /// 来源歌单，只用于显示；播放列表本身已经和页面脱钩。
    pub(crate) fn playlist_id(&self) -> Option<u64> {
        self.queue_source
    }

    /// 已实际提交音频预加载的候选；UI 自行决定封面尺寸及图片预取。
    pub(crate) fn preloading_song(&self) -> Option<Song> {
        self.preload.as_ref().map(|preload| preload.song.clone())
    }

    /// 播放列表当前的顺序。界面直接读这份数据展示，不再复制一份。
    pub fn queue(&self) -> &[Song] {
        &self.queue
    }

    /// 当前播放的是哪个队列项；重复歌曲也只对应一个位置。
    pub fn queue_cursor(&self) -> Option<usize> {
        self.queue_cursor
    }

    /// 队列内容的版本号：换列表、洗牌、插歌、移游标都会让它变化。
    /// 界面把它并进「需要重绘」的比对里，队列一变面板就刷新。
    pub fn queue_revision(&self) -> u64 {
        self.queue_revision
    }

    /// 播放列表面板双击某一项：游标移到这一项并立即播放。
    /// 队列项是按位置引用的，不按歌曲 id 反查，列表里有重复歌曲也不会选错。
    pub fn play_queue_index(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(song) = self.queue.get(index).cloned() else {
            return;
        };
        self.queue_cursor = Some(index);
        self.queue_revision = self.queue_revision.wrapping_add(1);
        self.select_song(song, cx);
    }

    /// 把页面上选中的歌曲换成一份新的播放列表并开始播放。
    /// 列表进来之后就是独立数据：页面再排序、切走或刷新都不影响它。
    pub fn play_list(
        &mut self,
        source: u64,
        songs: Vec<Song>,
        song_id: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(song) = self.set_list(Some(source), songs, Some(song_id)) else {
            cx.notify();
            return;
        };
        // 新列表也要服从当前的播放方式：随机模式下当场洗一遍。
        self.apply_mode_to_list(self.state.mode);
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

    /// 播放整张歌单；随机模式先洗牌，再从洗牌后的第一首开始。
    pub fn play_list_from_start(&mut self, source: u64, songs: Vec<Song>, cx: &mut Context<Self>) {
        self.set_list(Some(source), songs, None);
        self.apply_mode_to_list(self.state.mode);
        let Some(song) = self.queue.first().cloned() else {
            cx.notify();
            return;
        };
        self.queue_cursor = Some(0);
        self.queue_revision = self.queue_revision.wrapping_add(1);
        self.select_song(song, cx);
    }

    /// 换列表并把游标指到 `song_id`；不做任何其他判断，方便离线构造与测试。
    fn set_list(
        &mut self,
        source: Option<u64>,
        songs: Vec<Song>,
        song_id: Option<u64>,
    ) -> Option<Song> {
        self.cancel_preload();
        self.queue_source = source;
        self.queue = songs;
        self.queue_cursor =
            song_id.and_then(|song_id| self.queue.iter().position(|song| song.id == song_id));
        self.queue_revision = self.queue_revision.wrapping_add(1);
        self.current_list_song()
    }

    fn restore_list(&mut self, cache: PlaybackState) {
        // 保存的队列已经是当时的播放顺序，无需再洗牌。
        let cursor = cache.queue_cursor.filter(|&cursor| {
            cache
                .queue
                .get(cursor)
                .is_some_and(|song| song.id == cache.song_id)
        });
        self.set_list(Some(cache.playlist_id), cache.queue, Some(cache.song_id));
        if cursor.is_some() {
            self.queue_cursor = cursor;
        }
        self.state.mode = cache.mode;
        self.state.quality = cache.quality;
    }

    /// 游标所指的那首歌。
    fn current_list_song(&self) -> Option<Song> {
        let cursor = self.queue_cursor?;
        self.queue.get(cursor).cloned()
    }

    /// 与自动续播使用同一游标规则；单曲循环复用当前音源，不另加载一份。
    fn next_song(&self) -> Option<Song> {
        if self.state.mode.repeats_current() {
            return None;
        }
        let next = self.queue_cursor? + 1;
        self.queue
            .get(next)
            .or_else(|| {
                self.state
                    .mode
                    .wraps()
                    .then(|| self.queue.first())
                    .flatten()
            })
            .cloned()
    }

    fn cancel_preload(&mut self) {
        if let Some(preload) = self.preload.take() {
            preload.request.abort();
        }
    }

    fn audio_request(&self, song_id: u64, pending: Option<AudioRequest>, cx: &App) -> AudioRequest {
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let http = api.audio_http();
        let quality = self.state.quality;
        let cache = self.audio_cache.clone();
        let directory = self.persistence.audio_cache_directory();
        let pending = pending.map(CancelAudioOnDrop);
        api.runtime.spawn(async move {
            if let Some(mut pending) = pending
                && let Ok(Ok(audio)) = (&mut pending.0).await
            {
                return Ok(audio);
            }
            // 预加载失败只丢弃预加载结果；正式切到这首时按正常流程重试一次。
            // 每次新加载先获取当前权限/试听信息；不保存或复用会过期的播放 URL。
            let source = MusicApi::song_source(client, song_id, quality).await?;
            let store = cache
                .get_or_try_init(|| async move {
                    tokio::task::spawn_blocking(move || {
                        AudioCacheStore::new(directory, DEFAULT_CAPACITY, DEFAULT_MAX_FILE_BYTES)
                    })
                    .await
                    .map_err(|error| error.to_string())?
                })
                .await?
                .clone();
            StreamingAudio::open_cached(http, source, store, song_id).await
        })
    }

    /// 系统媒体控件的封面必须是本地文件：先下载，落盘后再通知一次让元数据带上它。
    fn sync_media_cover(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self
            .system_media
            .as_ref()
            .and_then(|media| media.cover_request(&self.state))
        else {
            return;
        };
        let api = MusicApi::global(cx);
        let http = api.audio_http();
        let download = api
            .runtime
            .spawn(async move { request.cache.download(request.url, &http).await });
        cx.spawn(async move |this, cx| {
            let downloaded = download.await.is_ok_and(|path| path.is_some());
            let _ = this.update(cx, |_, cx| {
                if downloaded {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn maybe_preload(&mut self, cx: &mut Context<Self>) {
        let next = self.next_song();
        if self.preload.as_ref().is_some_and(|preload| {
            next.as_ref().is_none_or(|song| song.id != preload.song.id)
                || preload.quality != self.state.quality
        }) {
            self.cancel_preload();
        }
        // 当前歌曲先下载完，避免下一首抢占当前播放的网络带宽。
        if self.preload.is_some()
            || !self.engine.download_complete()
            || self.state.duration.is_zero()
            || self.state.duration.saturating_sub(self.state.position) > PRELOAD_REMAINING
        {
            return;
        }
        let Some(next) = next else {
            return;
        };
        if self
            .state
            .current_song
            .as_ref()
            .is_some_and(|song| song.id == next.id)
        {
            return;
        }
        self.preload = Some(Preload {
            quality: self.state.quality,
            request: self.audio_request(next.id, None, cx),
            song: next,
        });
    }

    /// 手动切歌：`direction` 只取符号，`mode.wraps()` 决定能否越过两端。
    fn step_list(&mut self, direction: isize) -> Option<Song> {
        let mode = self.state.mode;
        let len = self.queue.len();
        if len == 0 {
            return None;
        }
        let cursor = self.queue_cursor? as isize + direction.signum();
        let cursor = if mode.wraps() {
            cursor.rem_euclid(len as isize)
        } else if (0..len as isize).contains(&cursor) {
            cursor
        } else {
            // 顺序播放和单曲循环下，手动切歌走到两端就没有去处了。
            return None;
        };
        self.queue_cursor = Some(cursor as usize);
        self.queue_revision = self.queue_revision.wrapping_add(1);
        self.current_list_song()
    }

    /// 播完一首后前进一格；返回 None 表示整个列表播完。
    /// 单曲循环不在这里处理：那是「不前进」，由控制器决定。
    fn advance_list(&mut self) -> Option<Song> {
        self.step_list(1)
    }

    /// 就地洗牌：列表此刻的顺序就是之后的播放顺序，不需要另存一份原始顺序。
    fn apply_mode_to_list(&mut self, mode: PlayMode) {
        if !mode.shuffles() {
            return;
        }
        let len = self.queue.len();
        if len < 2 {
            return;
        }
        let mut rng = rand::rng();
        if let Some(current) = self.queue_cursor.filter(|&cursor| cursor < len) {
            // 先单独保留当前具体队列项，再随机放回，重复 id 的元数据也不会串位。
            self.queue.swap(0, current);
            self.queue[1..].shuffle(&mut rng);
            let cursor = rng.random_range(0..len);
            self.queue.swap(0, cursor);
            self.queue_cursor = Some(cursor);
        } else {
            self.queue.shuffle(&mut rng);
            self.queue_cursor = None;
        }
        self.queue_revision = self.queue_revision.wrapping_add(1);
    }

    /// 下一首插入：插到当前歌曲之后。「播完这首就听它」和心动模式穿插推荐都走这里。
    #[allow(dead_code, reason = "下一首插入与心动模式尚未接入 UI")]
    pub fn insert_next(&mut self, song: Song, cx: &mut Context<Self>) {
        self.cancel_preload();
        match self.queue_cursor {
            Some(cursor) => self.queue.insert(cursor + 1, song),
            None => self.queue.insert(0, song),
        }
        self.queue_revision = self.queue_revision.wrapping_add(1);
        self.save_cache();
        cx.notify();
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
        let request = if let Some(preload) = self.preload.take() {
            if preload.song.id == song.id && preload.quality == self.state.quality {
                self.audio_request(song.id, Some(preload.request), cx)
            } else {
                preload.request.abort();
                self.audio_request(song.id, None, cx)
            }
        } else {
            self.audio_request(song.id, None, cx)
        };
        self.request = Some(request.abort_handle());
        // 切歌/切歌单立即落盘一次，避免等下一个 tick 才记录新的歌曲。
        self.save_cache();
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
                        self.state.is_playing = self.play_when_ready && self.engine.start();
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
        self.save_cache();
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
        self.save_cache();
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.is_play_requested() {
            self.pause(cx);
        } else {
            self.resume(cx);
        }
    }

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
            // load_song 自己会落盘，音质跟着这次重载一起写进缓存。
            self.load_song(song, position, play, cx);
        } else {
            // 没有正在播放的歌曲也要记住选择，下次启动沿用。
            self.save_cache();
            cx.notify();
        }
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        self.change_song(-1, cx);
    }
    pub fn next(&mut self, cx: &mut Context<Self>) {
        self.change_song(1, cx);
    }

    fn change_song(&mut self, direction: isize, cx: &mut Context<Self>) {
        if let Some(song) = self.step_list(direction) {
            self.select_song(song, cx);
        }
    }

    /// 一首歌播完后按当前播放方式决定去向。
    fn advance_on_finished(&mut self, cx: &mut Context<Self>) {
        if self.state.mode.repeats_current() {
            self.replay(cx);
            return;
        }
        let Some(song) = self.advance_list() else {
            // 顺序播放到队尾：停在末尾不动，位置保留，再按播放会重播当前歌曲。
            self.engine.stop();
            self.save_cache();
            cx.notify();
            return;
        };
        self.select_song(song, cx);
    }

    /// 回到开头重播当前歌曲：音源还在，回绕即可，不用再向接口要一次播放地址。
    fn replay(&mut self, cx: &mut Context<Self>) {
        if self.engine.seek_to(Duration::ZERO) && self.engine.start() {
            self.state.position = Duration::ZERO;
            self.state.is_playing = true;
            self.state.buffering = self.engine.buffering();
        } else if let Some(song) = self.state.current_song.clone() {
            // 音源已释放或设备出错时退回重新加载，行为与普通切歌一致。
            self.select_song(song, cx);
            return;
        }
        self.save_cache();
        cx.notify();
    }

    /// 播放栏按钮切换到下一种播放方式。
    pub fn cycle_mode(&mut self, cx: &mut Context<Self>) {
        self.set_mode(self.state.mode.next(), cx);
    }

    pub fn set_mode(&mut self, mode: PlayMode, cx: &mut Context<Self>) {
        if self.state.mode == mode {
            return;
        }
        self.state.mode = mode;
        self.cancel_preload();
        // 只在进入随机模式的那一刻洗牌；离开随机不再还原，洗好的顺序就是列表顺序。
        self.apply_mode_to_list(mode);
        self.save_cache();
        cx.notify();
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
            self.save_cache();
            cx.notify();
        }
    }

    /// 0..=1；修改当前播放音量，并在后续切歌或重试时保留。
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
            // 暂停淡出还会消费约 80 ms，完成后补记真实位置，只更新一次。
            if self.engine.has_source() {
                let position = self.engine.position().min(self.state.duration);
                if position != self.state.position {
                    self.state.position = position;
                    self.save_cache();
                    cx.notify();
                }
            }
            // 暂停时状态没有变化，不写盘也不重绘。
            return;
        }
        self.state.buffering = self.engine.buffering();
        if self.engine.finished() {
            self.state.position = self.state.duration;
            self.state.is_playing = false;
            self.state.buffering = false;
            self.advance_on_finished(cx);
        } else {
            // 从真实 PCM 消费量取进度，等待数据的静音不计入歌曲时间。
            self.state.position = self.engine.position().min(self.state.duration);
            self.maybe_preload(cx);
        }
        self.ticks_since_save += 1;
        if self.ticks_since_save >= SAVE_INTERVAL_TICKS {
            self.save_cache();
        }
        cx.notify();
    }

    /// 组装最新快照并交给后台写入线程；本方法自身不再做磁盘 I/O。
    ///
    /// 存的就是列表现在的样子：随机模式洗好的顺序跟着一起落盘，重启后继续顺着它播。
    fn save_cache(&mut self) {
        self.ticks_since_save = 0;
        let (Some(playlist_id), Some(song)) = (self.queue_source, self.current_list_song()) else {
            return;
        };
        self.persistence.request_playback(PlaybackState {
            playlist_id,
            queue: self.queue.clone(),
            song_id: song.id,
            queue_cursor: self.queue_cursor,
            position: self.state.position,
            mode: self.state.mode,
            quality: self.state.quality,
        });
    }

    fn fail(&mut self, error: String) {
        self.cancel_preload();
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
        self.cancel_preload();
        // 退出时等最后一次缓存真正落盘，确保下次启动能恢复到最后的位置。
        self.save_cache();
        self.persistence.flush();
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
    fn test_storage_outlives_controller_drop_and_is_then_removed() {
        let mut controller = controller_with(&[1], 1, PlayMode::Sequential);
        let path = controller
            .test_directory
            .as_ref()
            .unwrap()
            .path()
            .to_owned();
        assert!(path.exists());
        controller.save_cache();
        drop(controller);
        assert!(!path.exists());
    }

    #[test]
    fn system_stop_invalidates_pending_load_and_resets_position() {
        let mut controller = PlaybackController::for_test();
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

    /// 造一份播放列表，选中 `song_id`，并把当前歌曲同步到快照上。
    fn controller_with(ids: &[u64], song_id: u64, mode: PlayMode) -> PlaybackController {
        let mut controller = PlaybackController::for_test();
        let songs: Vec<Song> = ids
            .iter()
            .map(|&id| Song {
                id,
                ..Default::default()
            })
            .collect();
        let song = controller.set_list(Some(7), songs, Some(song_id));
        controller.state.mode = mode;
        controller.state.current_song = song;
        controller
    }

    fn ids(controller: &PlaybackController) -> Vec<u64> {
        controller.queue.iter().map(|song| song.id).collect()
    }

    #[test]
    fn advancing_uses_the_cursor_not_the_song_id() {
        // 同一首歌在列表里出现两次：游标按位置走，不会被 id 反查带回第一次出现的地方。
        let mut controller = controller_with(&[1, 2, 1, 3], 1, PlayMode::Sequential);
        assert_eq!(controller.queue_cursor, Some(0));
        assert_eq!(controller.advance_list().unwrap().id, 2);
        assert_eq!(controller.advance_list().unwrap().id, 1);
        assert_eq!(controller.queue_cursor, Some(2));
    }

    #[test]
    fn queue_revision_tracks_replacement_cursor_and_shuffle() {
        let mut controller = controller_with(&[1, 2, 1, 3], 1, PlayMode::Sequential);
        let revision = controller.queue_revision();
        controller.step_list(1);
        controller.step_list(1);
        assert_eq!(controller.queue_cursor(), Some(2));
        assert_eq!(controller.queue_revision(), revision + 2);
        controller.apply_mode_to_list(PlayMode::Shuffle);
        assert_eq!(controller.queue_revision(), revision + 3);
        assert_eq!(controller.queue()[controller.queue_cursor().unwrap()].id, 1);
        controller.set_list(Some(7), vec![], None);
        assert!(controller.queue().is_empty());
        assert_eq!(controller.queue_cursor(), None);
        assert_eq!(controller.queue_revision(), revision + 4);
        assert!(controller.step_list(1).is_none());
        assert_eq!(controller.queue_revision(), revision + 4);
    }

    #[test]
    fn manual_switching_follows_the_selected_mode() {
        // 顺序播放走到两端就没有去处。
        let mut sequential = controller_with(&[1, 2, 3], 1, PlayMode::Sequential);
        assert!(sequential.step_list(-1).is_none());
        assert_eq!(sequential.step_list(1).unwrap().id, 2);
        let mut last = controller_with(&[1, 2, 3], 3, PlayMode::Sequential);
        assert!(last.step_list(1).is_none());

        // 列表循环把两端接起来。
        let mut last = controller_with(&[1, 2, 3], 3, PlayMode::RepeatAll);
        assert_eq!(last.step_list(1).unwrap().id, 1);
        let mut first = controller_with(&[1, 2, 3], 1, PlayMode::RepeatAll);
        assert_eq!(first.step_list(-1).unwrap().id, 3);

        // 单曲循环只管自动续播，手动切歌仍沿列表走。
        let mut repeat_one = controller_with(&[1, 2, 3], 2, PlayMode::RepeatOne);
        assert_eq!(repeat_one.step_list(1).unwrap().id, 3);
    }

    #[test]
    fn finishing_a_song_follows_the_list_and_the_mode() {
        let mut sequential = controller_with(&[1, 2, 3], 1, PlayMode::Sequential);
        assert_eq!(sequential.advance_list().unwrap().id, 2);
        assert_eq!(sequential.advance_list().unwrap().id, 3);
        assert!(sequential.advance_list().is_none());

        let mut repeat_all = controller_with(&[1, 2, 3], 3, PlayMode::RepeatAll);
        assert_eq!(repeat_all.advance_list().unwrap().id, 1);
    }

    #[test]
    fn shuffle_rewrites_the_list_order_and_keeps_the_current_song() {
        // 借列表循环走完整条列表：洗牌后每首歌仍然恰好排在播放序列上一次。
        let mut controller = controller_with(&[1, 2, 3, 4, 5], 3, PlayMode::RepeatAll);
        let before = ids(&controller);
        controller.apply_mode_to_list(PlayMode::Shuffle);
        assert_eq!(controller.current_list_song().unwrap().id, 3);

        let mut played = vec![controller.current_list_song().unwrap().id];
        while played.len() < controller.queue.len() {
            played.push(controller.advance_list().unwrap().id);
        }
        played.sort_unstable();
        assert_eq!(played, before);
    }

    #[test]
    fn shuffle_preserves_the_specific_duplicate_queue_entry() {
        let mut controller = controller_with(&[1; 8], 1, PlayMode::Shuffle);
        for (index, song) in controller.queue.iter_mut().enumerate() {
            song.name = format!("版本 {index}");
            song.al.pic_url = Some(format!("https://example.com/{index}.png"));
        }
        controller.queue_cursor = Some(5);
        let current = serde_json::to_value(controller.current_list_song()).unwrap();
        for _ in 0..32 {
            controller.apply_mode_to_list(PlayMode::Shuffle);
            assert_eq!(
                serde_json::to_value(controller.current_list_song()).unwrap(),
                current
            );
            let mut names: Vec<_> = controller
                .queue
                .iter()
                .map(|song| song.name.clone())
                .collect();
            names.sort();
            assert_eq!(
                names,
                (0..8)
                    .map(|index| format!("版本 {index}"))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn restoring_duplicate_songs_validates_cursor_and_falls_back_to_id() {
        let mut controller = controller_with(&[1, 2, 1, 3], 1, PlayMode::Shuffle);
        controller.queue[0].name = "第一个版本".into();
        controller.queue[2].name = "第二个版本".into();
        controller.queue_cursor = Some(2);
        controller.state.position = Duration::from_secs(9);
        controller.save_cache();
        controller.persistence.flush();
        for (saved_cursor, expected_cursor) in [
            (Some(2), Some(2)),
            (None, Some(0)),
            (Some(1), Some(0)),
            (Some(99), Some(0)),
        ] {
            let mut cache = controller.persistence.load_playback().unwrap();
            assert_eq!(cache.queue_cursor, Some(2));
            assert_eq!(cache.position, Duration::from_secs(9));
            cache.queue_cursor = saved_cursor;
            let mut restored = PlaybackController::for_test();
            restored.restore_list(cache);
            assert_eq!(restored.queue_cursor, expected_cursor);
            assert_eq!(restored.state.mode, PlayMode::Shuffle);
            if saved_cursor == Some(2) {
                assert_eq!(restored.current_list_song().unwrap().name, "第二个版本");
                assert_eq!(restored.advance_list().unwrap().id, 3);
            } else {
                assert_eq!(restored.current_list_song().unwrap().name, "第一个版本");
            }
        }
        let mut cache = controller.persistence.load_playback().unwrap();
        cache.song_id = 99;
        let mut restored = PlaybackController::for_test();
        restored.restore_list(cache);
        assert!(restored.current_list_song().is_none());
    }

    #[test]
    fn obsolete_load_cannot_replace_current_snapshot() {
        let mut controller = PlaybackController::for_test();
        controller.state.revision = 3;
        controller.state.loading = true;
        assert!(!controller.finish(1, Err("旧请求失败".into())));
        assert!(controller.snapshot().loading);
        assert!(controller.snapshot().error.is_none());
        assert!(controller.finish(3, Err("当前请求失败".into())));
        assert!(!controller.snapshot().loading);
        assert_eq!(controller.snapshot().error.as_deref(), Some("当前请求失败"));
    }

    #[test]
    fn cache_round_trips_through_the_background_writer() {
        let mut controller = PlaybackController::for_test();
        controller.set_list(
            Some(5),
            vec![Song {
                id: 9,
                ..Default::default()
            }],
            Some(9),
        );
        controller.state.current_song = controller.current_list_song();
        controller.state.position = Duration::from_secs(7);
        controller.state.mode = PlayMode::RepeatAll;
        controller.state.quality = AudioQualityLevel::Lossless;
        controller.save_cache();
        controller.persistence.flush();

        let loaded = controller.persistence.load_playback().unwrap();
        assert_eq!(loaded.playlist_id, 5);
        assert_eq!(loaded.song_id, 9);
        assert_eq!(loaded.queue_cursor, Some(0));
        assert_eq!(loaded.position, Duration::from_secs(7));
        assert_eq!(loaded.mode, PlayMode::RepeatAll);
        assert_eq!(loaded.quality, AudioQualityLevel::Lossless);
    }

    #[test]
    fn preload_candidate_matches_automatic_queue_advance_without_moving_cursor() {
        for mode in [
            PlayMode::Sequential,
            PlayMode::RepeatOne,
            PlayMode::RepeatAll,
            PlayMode::Shuffle,
        ] {
            let mut controller = controller_with(&[1, 2, 3], 1, mode);
            assert_eq!(
                controller.next_song().map(|song| song.id),
                if mode.repeats_current() {
                    None
                } else {
                    Some(2)
                }
            );
            assert_eq!(controller.queue_cursor, Some(0));
            controller.queue_cursor = Some(2);
            assert_eq!(
                controller.next_song().map(|song| song.id),
                if !mode.repeats_current() && mode.wraps() {
                    Some(1)
                } else {
                    None
                }
            );
            assert_eq!(controller.queue_cursor, Some(2));
        }
    }

    #[tokio::test]
    async fn replacing_queue_and_stopping_cancel_speculative_audio() {
        let mut controller = controller_with(&[1, 2], 1, PlayMode::Sequential);
        assert!(controller.preloading_song().is_none());
        let request = tokio::spawn(std::future::pending());
        let aborted = request.abort_handle();
        controller.preload = Some(Preload {
            song: Song {
                id: 2,
                name: "真正预加载的版本".into(),
                ..Default::default()
            },
            quality: AudioQualityLevel::Standard,
            request,
        });
        assert_eq!(
            controller.preloading_song().unwrap().name,
            "真正预加载的版本"
        );
        assert_eq!(controller.queue_cursor, Some(0));
        controller.set_list(
            None,
            vec![Song {
                id: 3,
                ..Default::default()
            }],
            Some(3),
        );
        assert!(controller.preload.is_none());
        assert!(controller.preloading_song().is_none());
        tokio::task::yield_now().await;
        assert!(aborted.is_finished());
        let request = tokio::spawn(std::future::pending());
        let aborted = request.abort_handle();
        controller.preload = Some(Preload {
            song: Song {
                id: 4,
                ..Default::default()
            },
            quality: AudioQualityLevel::Standard,
            request,
        });
        controller.stop();
        assert!(controller.preload.is_none());
        assert!(controller.preloading_song().is_none());
        tokio::task::yield_now().await;
        assert!(aborted.is_finished());
    }
}
