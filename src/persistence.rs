use crate::models::Song;
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
    writer: Arc<Writer>,
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
}

#[derive(Deserialize, Serialize)]
pub struct PlaybackState {
    pub playlist_id: u64,
    pub queue: Vec<Song>,
    pub song_id: u64,
    pub position: Duration,
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
        Ok(Self::at(directory))
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
        Self { directory, writer }
    }

    pub fn load_playback(&self) -> Option<PlaybackState> {
        let bytes = fs::read(self.directory.join("playback.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// 入队一份最新状态；不阻塞调用方，磁盘写入由后台线程完成。
    pub fn request_playback(&self, state: PlaybackState) {
        let mut slot = self.writer.state.lock().unwrap();
        slot.pending = Some(state);
        self.writer.changed.notify_all();
    }

    /// 等待已入队的状态全部落盘。仅在退出（Drop）时调用。
    pub fn flush(&self) {
        let mut slot = self.writer.state.lock().unwrap();
        while slot.pending.is_some() || slot.writing {
            slot = self.writer.changed.wait(slot).unwrap();
        }
    }
}

fn write_loop(directory: PathBuf, writer: Arc<Writer>) {
    loop {
        let state = {
            let mut slot = writer.state.lock().unwrap();
            while slot.pending.is_none() {
                slot = writer.changed.wait(slot).unwrap();
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
    let temporary = directory.join("playback.json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(state).map_err(io::Error::other)?,
    )?;
    fs::rename(temporary, directory.join("playback.json"))
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
        };

        store.request_playback(state);
        store.flush();
        let loaded = store.load_playback().unwrap();
        assert_eq!(loaded.playlist_id, 42);
        assert_eq!(loaded.queue[0].id, 7);
        assert_eq!(loaded.song_id, 7);
        assert_eq!(loaded.position, Duration::from_secs(13));
        fs::remove_dir_all(directory).unwrap();
    }
}
