use crate::models::{AudioQualityLevel, PlayMode, Song};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

/// 播放状态的读写入口。读在启动时同步完成；写交给一个后台线程，
/// 避免磁盘 I/O 落在 GPUI 主线程上拖慢界面或音频调度。
#[derive(Clone)]
pub struct Persistence {
    directory: PathBuf,
    cache_directory: PathBuf,
    writer: Arc<WriterOwner>,
}

/// 只有 Persistence 持有 owner；线程持有 worker，最后一个 owner 释放后线程可退出。
struct WriterOwner {
    worker: Arc<Writer>,
}

impl Drop for WriterOwner {
    fn drop(&mut self) {
        self.worker.state.lock().unwrap().stopping = true;
        self.worker.changed.notify_all();
    }
}

/// 写入线程与调用方共享的槽位：只保留最新一份待写状态。
#[derive(Default)]
struct Writer {
    state: Mutex<WriterState>,
    changed: Condvar,
}

#[derive(Default)]
struct WriterState {
    pending: Option<PlaybackState>,
    writing: bool,
    stopping: bool,
}

#[derive(Deserialize, Serialize)]
pub struct PlaybackState {
    pub playlist_id: u64,
    pub queue: Vec<Song>,
    pub song_id: u64,
    pub position: Duration,
    /// 上次的播放方式；早期缓存没有这个字段，缺失时回落到顺序播放。
    #[serde(default)]
    pub mode: PlayMode,
    /// 选择的音质；早期缓存没有这个字段，缺失时回落到默认档位。
    #[serde(default)]
    pub quality: AudioQualityLevel,
}

impl Default for Persistence {
    fn default() -> Self {
        Self::new().expect("无法确定应用持久化目录")
    }
}

impl Persistence {
    pub fn new() -> io::Result<Self> {
        let directory =
            if cfg!(target_os = "macos") {
                PathBuf::from(std::env::var_os("HOME").ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "HOME 环境变量不存在")
                })?)
                .join("Library/Application Support/NeteaseMusicGpui")
            } else if cfg!(target_os = "windows") {
                PathBuf::from(std::env::var_os("APPDATA").ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "APPDATA 环境变量不存在")
                })?)
                .join("NeteaseMusicGpui")
            } else {
                PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                        .join(".local/share")
                        .into_os_string()
                }))
                .join("netease-music-gpui")
            };
        let cache_directory = if cfg!(target_os = "macos") {
            PathBuf::from(std::env::var_os("HOME").unwrap()).join("Library/Caches/NeteaseMusicGpui")
        } else if cfg!(target_os = "windows") {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|path| path.join("NeteaseMusicGpui/Cache"))
                .unwrap_or_else(|| directory.join("cache"))
        } else {
            PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join(".cache")
                    .into_os_string()
            }))
            .join("netease-music-gpui")
        };
        let mut persistence = Self::at(directory);
        persistence.cache_directory = cache_directory;
        Ok(persistence)
    }

    /// 在 `directory` 下建立存储，并启动唯一的写入线程。所有 clone 共用同一线程。
    pub fn at(directory: PathBuf) -> Self {
        let writer = Arc::new(Writer::default());
        let thread_writer = writer.clone();
        let thread_directory = directory.clone();
        std::thread::Builder::new()
            .name("playback-persistence".into())
            .spawn(move || write_loop(thread_directory, thread_writer))
            .expect("无法启动播放缓存写入线程");
        Self {
            cache_directory: directory.join("cache"),
            directory,
            writer: Arc::new(WriterOwner { worker: writer }),
        }
    }

    /// 可淘汰媒体与播放状态分开存放；系统清理缓存不影响队列和进度。
    pub fn audio_cache_directory(&self) -> PathBuf {
        self.cache_directory.join("audio-v1")
    }

    pub fn load_playback(&self) -> Option<PlaybackState> {
        let bytes = fs::read(self.directory.join("playback.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// 入队一份最新状态；不阻塞调用方，磁盘写入由后台线程完成。
    pub fn request_playback(&self, state: PlaybackState) {
        let mut slot = self.writer.worker.state.lock().unwrap();
        slot.pending = Some(state);
        self.writer.worker.changed.notify_all();
    }

    /// 等待已入队的状态全部落盘。仅在退出（Drop）时调用。
    pub fn flush(&self) {
        let mut slot = self.writer.worker.state.lock().unwrap();
        while slot.pending.is_some() || slot.writing {
            slot = self.writer.worker.changed.wait(slot).unwrap();
        }
    }
}

fn write_loop(directory: PathBuf, writer: Arc<Writer>) {
    loop {
        let state = {
            let mut slot = writer.state.lock().unwrap();
            while slot.pending.is_none() && !slot.stopping {
                slot = writer.changed.wait(slot).unwrap();
            }
            if slot.pending.is_none() {
                return;
            }
            slot.writing = true;
            slot.pending.take().unwrap()
        };
        if let Err(error) = write_playback(&directory, &state) {
            eprintln!("保存播放缓存失败：{error}");
        }
        let mut slot = writer.state.lock().unwrap();
        slot.writing = false;
        writer.changed.notify_all();
    }
}

fn write_playback(directory: &Path, state: &PlaybackState) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut temporary, state).map_err(io::Error::other)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(directory.join("playback.json"))
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_state_round_trips_through_its_store() {
        let directory = std::env::temp_dir().join(format!(
            "netease-music-gpui-persistence-{}",
            std::process::id()
        ));
        let store = Persistence::at(directory.clone());
        let state = PlaybackState {
            playlist_id: 42,
            queue: vec![Song {
                id: 7,
                name: "测试歌曲".into(),
                ..Default::default()
            }],
            song_id: 7,
            position: Duration::from_secs(13),
            mode: PlayMode::Shuffle,
            quality: AudioQualityLevel::Lossless,
        };

        store.request_playback(state);
        store.flush();
        let loaded = store.load_playback().unwrap();
        assert_eq!(loaded.playlist_id, 42);
        assert_eq!(loaded.queue[0].id, 7);
        assert_eq!(loaded.song_id, 7);
        assert_eq!(loaded.position, Duration::from_secs(13));
        assert_eq!(loaded.mode, PlayMode::Shuffle);
        assert_eq!(loaded.quality, AudioQualityLevel::Lossless);
        fs::remove_dir_all(directory).unwrap();
    }

    /// 旧版缓存没有 mode 字段，不能被整份丢弃——队列和进度仍然要恢复。
    #[test]
    fn legacy_cache_without_play_mode_still_loads() {
        let directory = std::env::temp_dir().join(format!(
            "netease-music-gpui-legacy-cache-{}",
            std::process::id()
        ));
        let store = Persistence::at(directory.clone());
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("playback.json"),
            serde_json::json!({
                "playlist_id": 42,
                "queue": [],
                "song_id": 7,
                "position": {"secs": 5, "nanos": 0},
            })
            .to_string(),
        )
        .unwrap();

        let loaded = store.load_playback().unwrap();
        assert_eq!(loaded.song_id, 7);
        assert_eq!(loaded.mode, PlayMode::default());
        assert_eq!(loaded.quality, AudioQualityLevel::default());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn last_store_clone_stops_writer_after_draining_pending_state() {
        let directory = tempfile::tempdir().unwrap();
        let store = Persistence::at(directory.path().to_owned());
        let worker = store.writer.worker.clone();
        let remaining = store.clone();
        drop(store);
        assert!(!worker.state.lock().unwrap().stopping);
        remaining.request_playback(PlaybackState {
            playlist_id: 1,
            queue: Vec::new(),
            song_id: 2,
            position: Duration::ZERO,
            mode: PlayMode::Sequential,
            quality: AudioQualityLevel::Standard,
        });
        drop(remaining);
        let state = worker.state.lock().unwrap();
        assert!(state.stopping);
        let (_state, timeout) = worker
            .changed
            .wait_timeout_while(state, Duration::from_secs(3), |state| {
                state.pending.is_some() || state.writing
            })
            .unwrap();
        assert!(!timeout.timed_out());
        assert!(directory.path().join("playback.json").is_file());
    }
}
