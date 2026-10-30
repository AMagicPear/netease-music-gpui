//! HWND 绑定的 Windows SMTC。事件只发回控制器，本地封面同步打开为只读流，
//! 不在 GPUI 的 Entity 更新期间等待 WinRT 异步操作（可能泵消息并重入 UI）。

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use souvlaki::{
    MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection,
};
use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
        SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsDisplayUpdater, SystemMediaTransportControlsTimelineProperties,
    },
    Storage::Streams::{IRandomAccessStream, RandomAccessStreamReference},
    Win32::{
        Foundation::HWND,
        System::WinRT::{CreateRandomAccessStreamOnFile, ISystemMediaTransportControlsInterop},
    },
    core::{HSTRING, PCWSTR},
};

pub(super) struct MediaControls {
    controls: SystemMediaTransportControls,
    display: SystemMediaTransportControlsDisplayUpdater,
    timeline: SystemMediaTransportControlsTimelineProperties,
    button_token: Option<i64>,
    position_token: Option<i64>,
}

impl MediaControls {
    pub(super) fn new(config: PlatformConfig) -> Result<Self> {
        let interop: ISystemMediaTransportControlsInterop = windows::core::factory::<
            SystemMediaTransportControls,
            ISystemMediaTransportControlsInterop,
        >()?;
        let hwnd = config.hwnd.context("Windows 媒体控件缺少 HWND")?;
        // HWND 来自当前仍存活的 GPUI 窗口。
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd)) }?;
        Ok(Self {
            display: controls.DisplayUpdater()?,
            timeline: SystemMediaTransportControlsTimelineProperties::new()?,
            controls,
            button_token: None,
            position_token: None,
        })
    }

    pub(super) fn attach(
        &mut self,
        handler: impl Fn(MediaControlEvent) + Send + Sync + 'static,
    ) -> Result<()> {
        self.controls.SetIsEnabled(true)?;
        self.controls.SetIsPlayEnabled(true)?;
        self.controls.SetIsPauseEnabled(true)?;
        self.controls.SetIsStopEnabled(true)?;
        self.controls.SetIsNextEnabled(true)?;
        self.controls.SetIsPreviousEnabled(true)?;
        self.controls.SetIsFastForwardEnabled(true)?;
        self.controls.SetIsRewindEnabled(true)?;
        self.display.SetType(MediaPlaybackType::Music)?;
        let handler = Arc::new(handler);
        let button_handler = handler.clone();
        self.button_token =
            Some(self.controls.ButtonPressed(&TypedEventHandler::new(
                move |_,
                      args: windows::core::Ref<
                    SystemMediaTransportControlsButtonPressedEventArgs,
                >| {
                    let Some(args) = args.as_ref() else {
                        return Ok(());
                    };
                    let event = match args.Button()? {
                        SystemMediaTransportControlsButton::Play => MediaControlEvent::Play,
                        SystemMediaTransportControlsButton::Pause => MediaControlEvent::Pause,
                        SystemMediaTransportControlsButton::Stop => MediaControlEvent::Stop,
                        SystemMediaTransportControlsButton::Next => MediaControlEvent::Next,
                        SystemMediaTransportControlsButton::Previous => MediaControlEvent::Previous,
                        SystemMediaTransportControlsButton::FastForward => {
                            MediaControlEvent::Seek(SeekDirection::Forward)
                        }
                        SystemMediaTransportControlsButton::Rewind => {
                            MediaControlEvent::Seek(SeekDirection::Backward)
                        }
                        _ => return Ok(()),
                    };
                    button_handler(event);
                    Ok(())
                },
            ))?);
        self.position_token = Some(self.controls.PlaybackPositionChangeRequested(
            &TypedEventHandler::new(
                move |_, args: windows::core::Ref<PlaybackPositionChangeRequestedEventArgs>| {
                    if let Some(args) = args.as_ref() {
                        let ticks = args.RequestedPlaybackPosition()?.Duration.max(0) as u64;
                        handler(MediaControlEvent::SetPosition(MediaPosition(
                            Duration::from_nanos(ticks.saturating_mul(100)),
                        )));
                    }
                    Ok(())
                },
            ),
        )?);
        Ok(())
    }

    pub(super) fn set_metadata(&mut self, metadata: MediaMetadata<'_>) -> Result<()> {
        self.display.ClearAll()?;
        self.display.SetType(MediaPlaybackType::Music)?;
        let properties = self.display.MusicProperties()?;
        properties.SetTitle(&HSTRING::from(metadata.title.unwrap_or_default()))?;
        properties.SetArtist(&HSTRING::from(metadata.artist.unwrap_or_default()))?;
        properties.SetAlbumTitle(&HSTRING::from(metadata.album.unwrap_or_default()))?;
        if let Some(url) = metadata.cover_url {
            // 封面失败不应阻止标题、时间轴与播放按钮发布，也不在每个进度 tick 重试。
            let result = thumbnail(url)
                .and_then(|stream| self.display.SetThumbnail(&stream).map_err(Into::into));
            if let Err(error) = result {
                log::warn!("系统媒体封面不可用：{error:#}");
            }
        }
        self.timeline.SetStartTime(TimeSpan::default())?;
        self.timeline.SetMinSeekTime(TimeSpan::default())?;
        let end = timespan(metadata.duration.unwrap_or_default());
        self.timeline.SetEndTime(end)?;
        self.timeline.SetMaxSeekTime(end)?;
        // 切到更短的歌曲时不能沿用上一首的位置；共享层紧接着会发布新位置。
        self.timeline.SetPosition(TimeSpan::default())?;
        self.controls.UpdateTimelineProperties(&self.timeline)?;
        self.display.Update()?;
        Ok(())
    }

    pub(super) fn set_playback(&mut self, playback: MediaPlayback) -> Result<()> {
        let (status, position) = match playback {
            MediaPlayback::Playing { progress } => (MediaPlaybackStatus::Playing, progress),
            MediaPlayback::Paused { progress } => (MediaPlaybackStatus::Paused, progress),
            MediaPlayback::Stopped => (MediaPlaybackStatus::Stopped, None),
        };
        self.timeline
            .SetPosition(timespan(position.map(|value| value.0).unwrap_or_default()))?;
        self.controls.UpdateTimelineProperties(&self.timeline)?;
        self.controls.SetPlaybackStatus(status)?;
        Ok(())
    }
}

fn cover_path(url: &str) -> Result<PathBuf> {
    gpui::http_client::Url::parse(url)?
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("系统媒体封面必须为本地 file URL"))
}

fn thumbnail(url: &str) -> Result<RandomAccessStreamReference> {
    let path = cover_path(url)?;
    let path = HSTRING::from(path.as_os_str());
    // FileAccessMode::Read = 0。同步打开文件，无 StorageFile::GetFileFromPathAsync().get()。
    let stream: IRandomAccessStream =
        unsafe { CreateRandomAccessStreamOnFile(PCWSTR(path.as_ptr()), 0) }
            .context("打开本地媒体封面")?;
    Ok(RandomAccessStreamReference::CreateFromStream(&stream)?)
}

fn timespan(duration: Duration) -> TimeSpan {
    TimeSpan {
        Duration: (duration.as_nanos() / 100).min(i64::MAX as u128) as i64,
    }
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        if let Some(token) = self.button_token.take() {
            let _ = self.controls.RemoveButtonPressed(token);
        }
        if let Some(token) = self.position_token.take() {
            let _ = self.controls.RemovePlaybackPositionChangeRequested(token);
        }
        let _ = self.controls.SetIsEnabled(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_decode_drive_unicode_spaces_and_reserved_characters() {
        let path = PathBuf::from(r"C:\音乐 应用\封面 #1%.png");
        let url = gpui::http_client::Url::from_file_path(&path).unwrap();
        assert_eq!(cover_path(url.as_str()).unwrap(), path);
        assert!(cover_path("https://example.com/cover.png").is_err());
    }

    #[test]
    fn timeline_uses_100ns_ticks_and_saturates() {
        assert_eq!(timespan(Duration::from_millis(1234)).Duration, 12_340_000);
        assert_eq!(timespan(Duration::MAX).Duration, i64::MAX);
    }
}
