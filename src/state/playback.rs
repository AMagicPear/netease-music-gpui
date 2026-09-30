use std::time::Duration;

pub struct Song {
    pub title: String,
    pub artist: String,
    pub duration: Duration,
}

pub struct PlaybackState {
    pub current_song: Option<Song>,
    pub position: Duration,
    pub is_playing: bool,
}

impl PlaybackState {
    pub fn progress(&self) -> f32 {
        let Some(song) = &self.current_song else {
            return 0.;
        };

        let duration = song.duration.as_secs_f32();
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
            title: String::new(),
            artist: String::new(),
            duration: Duration::from_secs(10),
        });
        assert_eq!(playback.progress(), 0.5);

        playback.position = Duration::from_secs(15);
        assert_eq!(playback.progress(), 1.);
    }
}
