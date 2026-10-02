use super::song::Song;
use std::time::Duration;

pub struct PlaybackState {
    pub current_song: Option<Song>,
    pub position: Duration,
    pub is_playing: bool,
}

impl PlaybackState {
    /// 页面只提交歌曲，换歌时的位置和播放状态由播放器统一重置。
    pub fn select_song(&mut self, song: Song) {
        self.current_song = Some(song);
        self.position = Duration::ZERO;
        self.is_playing = false;
    }

    /// Slider 使用 0..=1 的进度；实际播放器接入后在这里提交 seek。
    pub fn seek_to_progress(&mut self, progress: f32) {
        if !progress.is_finite() {
            return;
        }
        self.position = self.current_song.as_ref().map_or(Duration::ZERO, |song| {
            song.duration().mul_f64(f64::from(progress.clamp(0., 1.)))
        });
    }

    pub fn progress(&self) -> f32 {
        let Some(song) = &self.current_song else {
            return 0.;
        };

        let duration = song.duration().as_secs_f32();
        if duration == 0. {
            0.
        } else {
            (self.position.as_secs_f32() / duration).clamp(0., 1.)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_handles_missing_and_out_of_range_times() {
        let mut playback = PlaybackState {
            current_song: None,
            position: Duration::from_secs(5),
            is_playing: false,
        };
        assert_eq!(playback.progress(), 0.);

        playback.current_song = Some(Song {
            dt: 10000,
            ..Default::default()
        });
        assert_eq!(playback.progress(), 0.5);

        playback.position = Duration::from_secs(15);
        assert_eq!(playback.progress(), 1.);

        playback.seek_to_progress(0.25);
        assert_eq!(playback.position, Duration::from_millis(2500));
        playback.seek_to_progress(2.);
        assert_eq!(playback.position, Duration::from_secs(10));
        playback.seek_to_progress(f32::NAN);
        assert_eq!(playback.position, Duration::from_secs(10));
        playback.seek_to_progress(-1.);
        assert_eq!(playback.position, Duration::ZERO);
        playback.current_song = None;
        playback.seek_to_progress(0.5);
        assert_eq!(playback.position, Duration::ZERO);
        playback.is_playing = true;
        playback.position = Duration::from_secs(4);
        playback.select_song(Song {
            id: 42,
            ..Default::default()
        });
        assert_eq!(playback.current_song.as_ref().unwrap().id, 42);
        assert_eq!(playback.position, Duration::ZERO);
        assert!(!playback.is_playing);
    }
}
