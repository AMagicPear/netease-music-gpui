use crate::models::Song;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf, time::Duration};

#[derive(Clone)]
pub struct Persistence {
    directory: PathBuf,
}

impl Default for Persistence {
    fn default() -> Self {
        Self::new().expect("无法确定应用持久化目录")
    }
}

#[derive(Deserialize, Serialize)]
pub struct PlaybackState {
    pub playlist_id: u64,
    pub queue: Vec<Song>,
    pub song_id: u64,
    pub position: Duration,
    pub was_playing: bool,
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
        Ok(Self { directory })
    }

    pub fn load_playback(&self) -> Option<PlaybackState> {
        let bytes = fs::read(self.directory.join("playback.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save_playback(&self, state: &PlaybackState) -> io::Result<()> {
        fs::create_dir_all(&self.directory)?;
        let path = self.directory.join("playback.json");
        let temporary = self.directory.join("playback.json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec(state).map_err(io::Error::other)?,
        )?;
        fs::rename(temporary, path)
    }
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
        let store = Persistence { directory };
        let state = PlaybackState {
            playlist_id: 42,
            queue: vec![Song {
                id: 7,
                name: "测试歌曲".into(),
                ..Default::default()
            }],
            song_id: 7,
            position: Duration::from_secs(13),
            was_playing: true,
        };

        store.save_playback(&state).unwrap();
        let loaded = store.load_playback().unwrap();
        assert_eq!(loaded.playlist_id, state.playlist_id);
        assert_eq!(loaded.queue[0].id, state.queue[0].id);
        assert_eq!(loaded.song_id, state.song_id);
        assert_eq!(loaded.position, state.position);
        assert_eq!(loaded.was_playing, state.was_playing);
        fs::remove_dir_all(store.directory).unwrap();
    }
}
