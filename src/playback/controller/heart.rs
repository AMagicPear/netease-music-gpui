use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::*;
use crate::models::HeartRecommendation;

/// 只持久化续播所需的身份和来源；请求、错误和等待状态不会跨启动恢复。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct HeartSession {
    pub user_id: u64,
    pub playlist_id: u64,
}

const REFILL_REMAINING: usize = 3;
const RECENT_SONGS: usize = 50;
const KEEP_HISTORY: usize = 100;

impl PlaybackController {
    pub(super) fn cancel_heart(&mut self) {
        self.heart_generation = self.heart_generation.wrapping_add(1);
        if let Some(request) = self.heart_request.take() {
            request.abort();
        }
        self.heart = None;
        if self.heart_active() {
            self.state.mode = PlayMode::Sequential;
            self.apply_mode_to_list(PlayMode::Sequential);
        }
        self.heart_continue = false;
        self.heart_waiting = false;
        self.state.heart_loading = false;
        self.state.heart_error = None;
    }

    pub fn exit_heart_mode(&mut self, cx: &mut Context<Self>) {
        self.cancel_heart();
        self.cancel_preload();
        self.save_cache();
        self.publish(cx, PlaybackEvent::SnapshotChanged);
    }

    pub fn validate_heart_account(&mut self, user_id: u64, cx: &mut Context<Self>) {
        if self
            .heart
            .as_ref()
            .is_some_and(|session| session.user_id != user_id)
        {
            self.exit_heart_mode(cx);
        }
    }

    pub(super) fn start_heart_mode(&mut self, cx: &mut Context<Self>) {
        if self.state.heart_loading || !self.heart_reachable() {
            return;
        }
        let source = self.heart_source.clone();
        self.cancel_heart();
        self.heart = source;
        self.heart_continue = true;
        self.heart_waiting = false;
        self.request_heart(cx);
    }

    /// 首页卡片是一份已准备好的新队列，与当前播放模式无关。
    pub fn play_heart_recommendation(
        &mut self,
        user_id: u64,
        recommendation: HeartRecommendation,
        cx: &mut Context<Self>,
    ) {
        if user_id == 0 {
            return;
        }
        self.cancel_heart();
        self.heart = Some(HeartSession {
            user_id,
            playlist_id: recommendation.playlist_id,
        });
        self.heart_waiting = true;
        if let Some((song, play)) = self.finish_heart(
            self.heart_generation,
            Ok((recommendation.playlist_id, recommendation.songs)),
        ) {
            self.load_song(song, Duration::ZERO, play, cx);
        }
        self.save_cache();
        self.publish(cx, PlaybackEvent::SnapshotChanged);
        self.maybe_preload(cx);
    }

    pub fn retry_heart_mode(&mut self, cx: &mut Context<Self>) {
        if self.heart.is_none() || self.state.heart_loading {
            return;
        }
        self.state.heart_error = None;
        if !self.state.duration.is_zero() && self.state.position >= self.state.duration {
            self.heart_waiting = true;
        }
        self.request_heart(cx);
    }

    fn request_heart(&mut self, cx: &mut Context<Self>) {
        if self.state.heart_loading {
            return;
        }
        let Some(session) = self.heart.clone() else {
            return;
        };
        self.state.heart_loading = true;
        self.state.heart_error = None;
        let generation = self.heart_generation;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let seed = self
            .heart_continue
            .then(|| self.current_list_song().map(|song| song.id))
            .flatten();
        let request = api.runtime.spawn(async move {
            let playlist_id = if session.playlist_id == 0 {
                MusicApi::heart_playlist(&client, session.user_id).await?
            } else {
                session.playlist_id
            };
            let songs = MusicApi::heart_songs_from_seed(client, playlist_id, seed).await?;
            Ok((playlist_id, songs))
        });
        self.heart_request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("心动推荐请求失败，请重试".into()));
            let _ = this.update(cx, |this, cx| {
                if generation != this.heart_generation {
                    return;
                }
                let selection = this.finish_heart(generation, result);
                if let Some((song, play)) = selection {
                    this.load_song(song, Duration::ZERO, play, cx);
                }
                this.save_cache();
                this.publish(cx, PlaybackEvent::SnapshotChanged);
                this.maybe_preload(cx);
            });
        })
        .detach();
        self.publish(cx, PlaybackEvent::SnapshotChanged);
    }

    fn finish_heart(
        &mut self,
        generation: u64,
        result: Result<(u64, Vec<Song>), String>,
    ) -> Option<(Song, bool)> {
        if generation != self.heart_generation {
            return None;
        }
        let mut session = self.heart.clone()?;
        self.heart_request = None;
        self.state.heart_loading = false;
        let initial = !self.heart_active();
        let waiting = self.heart_waiting;
        let result = result.and_then(|(playlist_id, songs)| {
            if playlist_id == 0 || songs.is_empty() {
                return Err("暂时没有心动推荐，请稍后重试".into());
            }
            Ok((playlist_id, songs))
        });
        let (playlist_id, songs) = match result {
            Ok(batch) => batch,
            Err(message) => {
                self.state.heart_error = Some(message);
                self.heart_waiting = false;
                if initial {
                    self.heart = None;
                    self.heart_continue = false;
                }
                return None;
            }
        };
        self.heart_waiting = false;
        self.state.heart_error = None;
        if initial {
            let continuing = self.heart_continue;
            self.heart_continue = false;
            if continuing {
                // 保留当前音源、进度与暂停意图，只替换尚未播放的部分。
                let cursor = self.queue_cursor?;
                self.queue.truncate(cursor + 1);
                if !self.append_heart_songs(songs) {
                    self.queue_order = (0..self.queue.len()).collect();
                    self.state.heart_error = Some("这批推荐没有新歌曲，点击重试换一批".into());
                }
                self.cancel_preload();
            } else {
                self.set_list(Some(playlist_id), songs, None);
                self.queue_cursor = Some(0);
            }
            session.playlist_id = playlist_id;
            self.heart = Some(session);
            // 心动模式生效：模式本身变成这一档，队列仍然按顺序往下走。
            self.state.mode = PlayMode::Heart;
            return if continuing && waiting {
                self.step_list(1).map(|song| (song, true))
            } else if continuing {
                None
            } else {
                self.current_list_song().map(|song| (song, waiting))
            };
        }
        if !self.append_heart_songs(songs) {
            self.state.heart_error = Some("这批推荐没有新歌曲，点击重试换一批".into());
            return None;
        }
        if waiting {
            return self.step_list(1).map(|song| (song, true));
        }
        None
    }

    fn append_heart_songs(&mut self, songs: Vec<Song>) -> bool {
        let cursor = self.queue_cursor.unwrap_or(0);
        // 保留待播歌曲与最近听过的歌曲，远期历史可再次出现，避免红心歌单最终被全部排除。
        let mut seen: HashSet<_> = self
            .queue
            .iter()
            .skip(cursor.saturating_sub(RECENT_SONGS))
            .map(|song| song.id)
            .collect();
        let songs: Vec<_> = songs
            .into_iter()
            .filter(|song| song.id != 0 && seen.insert(song.id))
            .collect();
        if songs.is_empty() {
            return false;
        }
        // 长时间心动播放只保留最近 100 首历史，不让队列和落盘数据无限增长。
        let remove = cursor.saturating_sub(KEEP_HISTORY);
        self.queue.drain(..remove);
        self.queue_cursor = Some(cursor - remove);
        self.queue.extend(songs);
        self.queue_order = (0..self.queue.len()).collect();
        true
    }

    pub(super) fn maybe_refill_heart(&mut self, cx: &mut Context<Self>) {
        if self.heart_active()
            && !self.state.heart_loading
            && self.state.heart_error.is_none()
            && self.queue_cursor.is_some_and(|cursor| {
                self.queue.len().saturating_sub(cursor + 1) <= REFILL_REMAINING
            })
        {
            self.request_heart(cx);
        }
    }

    /// 到队尾后等待推荐；暂停、退出和换队列都能撤销自动切入的意图。
    pub(super) fn wait_for_heart(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.heart_active() {
            return false;
        }
        self.heart_waiting = true;
        if self.state.heart_error.is_some() {
            self.retry_heart_mode(cx);
        } else {
            self.request_heart(cx);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn songs(ids: &[u64]) -> Vec<Song> {
        ids.iter()
            .map(|&id| Song {
                id,
                ..Default::default()
            })
            .collect()
    }

    fn pending() -> PlaybackController {
        let mut controller = PlaybackController::for_test();
        controller.set_list(Some(9), songs(&[90, 91]), Some(90));
        controller.state.current_song = controller.current_list_song();
        controller.state.mode = PlayMode::Shuffle;
        controller.heart = Some(HeartSession {
            user_id: 1,
            playlist_id: 0,
        });
        controller.state.heart_loading = true;
        controller.heart_waiting = true;
        controller
    }

    fn active(ids: &[u64]) -> PlaybackController {
        let mut controller = pending();
        let generation = controller.heart_generation;
        controller.finish_heart(generation, Ok((7, songs(ids))));
        controller
    }

    fn ids(controller: &PlaybackController) -> Vec<u64> {
        controller.queue.iter().map(|song| song.id).collect()
    }

    #[test]
    fn heart_requires_a_selected_song_from_the_favorite_playlist() {
        let mut controller = pending();
        assert!(!controller.heart_reachable());
        controller.heart_source = Some(HeartSession {
            user_id: 1,
            playlist_id: 7,
        });
        assert!(!controller.heart_reachable());
        controller.heart_source = Some(HeartSession {
            user_id: 1,
            playlist_id: 9,
        });
        assert!(controller.heart_reachable());
        controller.queue_cursor = None;
        assert!(!controller.heart_reachable());
        controller.queue_source = Some(0);
        controller.heart_source = None;
        assert!(!controller.heart_reachable());
    }

    #[test]
    fn entering_from_player_keeps_current_song_position_and_play_intent() {
        for play in [false, true] {
            let mut controller = pending();
            controller.queue_cursor = Some(1);
            controller.state.current_song = controller.current_list_song();
            controller.state.position = Duration::from_secs(12);
            controller.state.is_playing = play;
            controller.heart_continue = true;
            controller.heart_waiting = false;
            assert!(
                controller
                    .finish_heart(controller.heart_generation, Ok((9, songs(&[91, 3, 2, 3]))))
                    .is_none()
            );
            assert_eq!(ids(&controller), [90, 91, 3, 2]);
            assert_eq!(controller.current_list_song().unwrap().id, 91);
            assert_eq!(controller.state.current_song.as_ref().unwrap().id, 91);
            assert_eq!(controller.state.position, Duration::from_secs(12));
            assert_eq!(controller.state.is_playing, play);
            assert_eq!(controller.state.mode, PlayMode::Heart);
            assert_eq!(controller.queue_source, Some(9));
        }
    }

    #[test]
    fn initial_player_batch_advances_only_if_song_finished_while_waiting() {
        for waiting in [false, true] {
            let mut controller = pending();
            controller.heart_continue = true;
            controller.heart_waiting = waiting;
            let selected =
                controller.finish_heart(controller.heart_generation, Ok((9, songs(&[3, 2]))));
            assert_eq!(
                selected.map(|(song, play)| (song.id, play)),
                waiting.then_some((3, true))
            );
            assert_eq!(
                controller.current_list_song().unwrap().id,
                if waiting { 3 } else { 90 }
            );
        }
    }

    #[test]
    fn new_home_batch_replaces_an_existing_heart_queue() {
        let mut controller = active(&[1, 2, 3]);
        controller.cancel_heart();
        controller.heart = Some(HeartSession {
            user_id: 1,
            playlist_id: 7,
        });
        controller.heart_waiting = true;
        let (song, play) = controller
            .finish_heart(controller.heart_generation, Ok((7, songs(&[8, 9]))))
            .unwrap();
        assert_eq!((song.id, play), (8, true));
        assert_eq!(ids(&controller), [8, 9]);
        assert_eq!(controller.state.mode, PlayMode::Heart);
    }

    #[test]
    fn old_session_previous_mode_is_ignored() {
        let session: HeartSession = serde_json::from_value(serde_json::json!({
            "user_id": 1, "playlist_id": 7, "previous_mode": "Shuffle"
        }))
        .unwrap();
        assert_eq!(
            session,
            HeartSession {
                user_id: 1,
                playlist_id: 7
            }
        );
    }

    #[test]
    fn heart_first_batch_preserves_server_order_and_pause_intent() {
        for play in [false, true] {
            let mut controller = pending();
            controller.heart_waiting = play;
            let selected = controller
                .finish_heart(controller.heart_generation, Ok((7, songs(&[3, 1, 2]))))
                .unwrap();
            assert_eq!((selected.0.id, selected.1), (3, play));
            assert_eq!(ids(&controller), [3, 1, 2]);
            assert_eq!(controller.state.mode, PlayMode::Heart);
            assert!(!controller.state.heart_loading);
        }
    }

    #[test]
    fn heart_initial_failure_keeps_existing_queue_song_and_mode() {
        for result in [Err("断网".into()), Ok((7, vec![]))] {
            let mut controller = pending();
            assert!(
                controller
                    .finish_heart(controller.heart_generation, result)
                    .is_none()
            );
            assert_eq!(ids(&controller), [90, 91]);
            assert_eq!(controller.state.current_song.as_ref().unwrap().id, 90);
            assert_eq!(controller.state.mode, PlayMode::Shuffle);
            assert!(controller.state.heart_error.is_some());
            assert!(controller.heart.is_none());
        }
    }

    #[test]
    fn heart_replacement_and_stop_discard_late_results() {
        for stop in [false, true] {
            let mut controller = pending();
            let generation = controller.heart_generation;
            if stop {
                controller.stop();
            } else {
                controller.set_list(Some(8), songs(&[8, 9]), Some(8));
            }
            let expected = ids(&controller);
            assert!(
                controller
                    .finish_heart(generation, Ok((7, songs(&[1, 2]))))
                    .is_none()
            );
            assert_eq!(ids(&controller), expected);
            assert_ne!(controller.state.mode, PlayMode::Heart);
            assert!(!controller.state.heart_loading);
        }
    }

    #[tokio::test]
    async fn heart_cancel_aborts_request_and_invalidates_session() {
        let mut controller = pending();
        let request = tokio::spawn(std::future::pending::<()>());
        controller.heart_request = Some(request.abort_handle());
        let generation = controller.heart_generation;
        controller.cancel_heart();
        assert!(request.await.unwrap_err().is_cancelled());
        assert_ne!(controller.heart_generation, generation);
        assert!(!controller.heart_waiting);
    }

    #[test]
    fn heart_refill_appends_in_order_and_deduplicates_current_future_and_batch() {
        let mut controller = active(&[1, 2, 3]);
        controller.queue_cursor = Some(1);
        assert!(controller.append_heart_songs(songs(&[3, 4, 2, 4, 5, 0])));
        assert_eq!(ids(&controller), [1, 2, 3, 4, 5]);
        assert_eq!(controller.queue_cursor, Some(1));
        assert_eq!(controller.queue_order, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn heart_refill_only_advances_when_explicitly_waiting() {
        for waiting in [false, true] {
            let mut controller = active(&[1, 2]);
            controller.queue_cursor = Some(1);
            controller.heart_waiting = waiting;
            controller.state.heart_loading = true;
            let selected =
                controller.finish_heart(controller.heart_generation, Ok((7, songs(&[3, 4]))));
            assert_eq!(
                selected.map(|(song, play)| (song.id, play)),
                waiting.then_some((3, true))
            );
            assert_eq!(controller.queue_cursor, Some(if waiting { 2 } else { 1 }));
            assert!(!controller.state.heart_loading);
        }
    }

    #[test]
    fn heart_failed_or_duplicate_refill_preserves_queue_and_exposes_retry() {
        for result in [Err("断网".into()), Ok((7, songs(&[1, 2])))] {
            let mut controller = active(&[1, 2]);
            controller.queue_cursor = Some(1);
            controller.heart_waiting = true;
            controller.state.heart_loading = true;
            assert!(
                controller
                    .finish_heart(controller.heart_generation, result)
                    .is_none()
            );
            assert_eq!(ids(&controller), [1, 2]);
            assert_eq!(controller.queue_cursor, Some(1));
            assert!(controller.state.heart_error.is_some());
            assert_eq!(controller.state.mode, PlayMode::Heart);
            assert!(!controller.state.heart_loading);
            assert!(!controller.heart_waiting);
        }
    }

    #[test]
    fn heart_history_is_bounded_without_losing_current_or_queued_songs() {
        let original: Vec<_> = (1..=160).collect();
        let mut controller = active(&original);
        controller.queue_cursor = Some(150);
        assert!(controller.append_heart_songs(songs(&[1, 150, 160, 161])));
        assert_eq!(controller.queue_cursor, Some(100));
        assert_eq!(controller.current_list_song().unwrap().id, 151);
        assert_eq!(
            &ids(&controller)[100..],
            &[151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 1, 161]
        );
        assert_eq!(
            controller.queue_order,
            (0..controller.queue.len()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn heart_session_round_trip_restores_without_playing_and_exit_is_sequential() {
        let mut controller = active(&[3, 1, 2]);
        controller.queue_cursor = Some(1);
        controller.state.position = Duration::from_secs(12);
        controller.save_cache();
        controller.persistence.flush();
        let cache = controller.persistence.load_playback().unwrap();
        let mut restored = PlaybackController::for_test();
        restored.restore_list(cache);
        assert_eq!(restored.heart, controller.heart);
        assert_eq!(ids(&restored), [3, 1, 2]);
        assert_eq!(restored.queue_cursor, Some(1));
        assert_eq!(restored.state.mode, PlayMode::Heart);
        assert!(!restored.state.is_playing);
        assert!(!restored.state.heart_loading);
        restored.cancel_heart();
        assert_eq!(restored.state.mode, PlayMode::Sequential);
        assert_eq!(restored.current_list_song().unwrap().id, 1);
    }
}
