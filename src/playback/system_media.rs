use std::time::Duration;

use gpui::Window;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::PlaybackSnapshot;

/// 仅桥接系统媒体会话，音频和队列仍由 PlaybackController 管理。
pub(super) struct SystemMedia {
    controls: MediaControls,
    metadata_key: Option<(u64, Duration)>,
    playback: Option<MediaPlayback>,
    #[cfg(target_os = "linux")]
    volume: Option<f32>,
}

impl SystemMedia {
    pub fn new(
        _window: &Window,
    ) -> Result<(Self, UnboundedReceiver<MediaControlEvent>), Box<dyn std::error::Error>> {
        #[cfg(target_os = "windows")]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match HasWindowHandle::window_handle(_window)?.as_raw() {
                RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut std::ffi::c_void),
                _ => return Err("系统媒体控件需要 Win32 窗口句柄".into()),
            }
        };
        #[cfg(not(target_os = "windows"))]
        let hwnd = None;
        let mut controls = MediaControls::new(PlatformConfig {
            dbus_name: "netease_music_gpui",
            display_name: "网易云音乐",
            hwnd,
        })?;
        let (sender, receiver) = unbounded_channel();
        controls.attach(move |event| {
            let _ = sender.send(event);
        })?;
        Ok((
            Self {
                controls,
                metadata_key: None,
                playback: None,
                #[cfg(target_os = "linux")]
                volume: None,
            },
            receiver,
        ))
    }

    pub fn sync(
        &mut self,
        state: &PlaybackSnapshot,
        has_source: bool,
    ) -> Result<(), souvlaki::Error> {
        // 没有歌曲时不发布空会话；进度通知不会反复加载封面。
        if let Some(song) = &state.current_song {
            let key = (song.id, state.duration);
            if self.metadata_key != Some(key) {
                let artist = song
                    .ar
                    .iter()
                    .filter_map(|artist| artist.name.as_deref())
                    .collect::<Vec<_>>()
                    .join(" / ");
                // 复用封面 URL 归一化：网易 CDN 升级到 HTTPS，并避免下载原图。
                let cover_url = song
                    .al
                    .pic_url
                    .as_deref()
                    .map(|url| crate::ui::assets::thumbnail_url(url, 512));
                self.controls.set_metadata(MediaMetadata {
                    title: Some(&song.name),
                    artist: Some(&artist),
                    album: song.al.name.as_deref(),
                    cover_url: cover_url.as_deref(),
                    duration: Some(state.duration),
                })?;
                self.metadata_key = Some(key);
                // metadata 可能重置原生时间轴，即使位置没变也要重新同步。
                self.playback = None;
            }
            let playback = playback_status(state, has_source);
            if self.playback.as_ref() != Some(&playback) {
                self.controls.set_playback(playback.clone())?;
                self.playback = Some(playback);
            }
        }
        #[cfg(target_os = "linux")]
        if self.volume != Some(state.volume) {
            self.controls.set_volume(state.volume as f64)?;
            self.volume = Some(state.volume);
        }
        Ok(())
    }
}

fn playback_status(state: &PlaybackSnapshot, has_source: bool) -> MediaPlayback {
    if !has_source
        || state.current_song.is_none()
        || state.loading
        || state.error.is_some()
        || (!state.is_playing && state.position >= state.duration)
    {
        MediaPlayback::Stopped
    } else if state.is_playing && !state.buffering {
        MediaPlayback::Playing {
            progress: Some(MediaPosition(state.position)),
        }
    } else {
        // 缓冲期间真实播放进度不前进，系统时间轴也应暂停。
        MediaPlayback::Paused {
            progress: Some(MediaPosition(state.position)),
        }
    }
}

impl Drop for SystemMedia {
    fn drop(&mut self) {
        if self.metadata_key.is_some() {
            let _ = self.controls.set_playback(MediaPlayback::Stopped);
            let _ = self.controls.set_metadata(MediaMetadata::default());
        }
        // MediaControls::drop 自动移除命令处理器。
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Song;

    #[test]
    fn timeline_follows_audio_including_buffering_and_failure() {
        let mut state = PlaybackSnapshot::default();
        assert_eq!(playback_status(&state, true), MediaPlayback::Stopped);
        state.current_song = Some(Song::default());
        state.duration = Duration::from_secs(60);
        state.position = Duration::from_secs(12);
        state.is_playing = true;
        assert_eq!(playback_status(&state, false), MediaPlayback::Stopped);
        let progress = Some(MediaPosition(state.position));
        assert_eq!(
            playback_status(&state, true),
            MediaPlayback::Playing { progress }
        );
        state.buffering = true;
        assert_eq!(
            playback_status(&state, true),
            MediaPlayback::Paused { progress }
        );
        state.buffering = false;
        state.is_playing = false;
        assert_eq!(
            playback_status(&state, true),
            MediaPlayback::Paused { progress }
        );
        state.loading = true;
        assert_eq!(playback_status(&state, true), MediaPlayback::Stopped);
        state.loading = false;
        state.error = Some("解码失败".into());
        assert_eq!(playback_status(&state, true), MediaPlayback::Stopped);
        state.error = None;
        state.position = state.duration;
        assert_eq!(playback_status(&state, true), MediaPlayback::Stopped);
    }
}
