//! 歌曲互动计数（喜欢 / 评论数）：跟着当前播放歌曲加载，过期结果由 store 自己丢弃。
//!
//! 播放栏只读 `counts()`；请求取消与过期校验都留在这里。

use gpui::{Context, Entity, ReadGlobal, Subscription};

use super::resource::Resource;
use crate::api::MusicApi;
use crate::playback::{PlaybackController, PlaybackEvent};

/// 两个互动区图标旁的计数（下标 0 = 喜欢，1 = 评论）；`None` 表示没拿到。
pub struct SongCounts {
    playback: Entity<PlaybackController>,
    /// 计数对应的歌曲；请求身份另外校验，避免 A → B → A 时旧结果覆盖新结果。
    song_id: Option<u64>,
    data: Resource<[Option<u64>; 2]>,
    _playback_subscription: Subscription,
}

impl SongCounts {
    pub fn new(playback: Entity<PlaybackController>, cx: &mut Context<Self>) -> Self {
        // 切歌才重新加载：100ms 的位置推进是另一个变体，打不到这。
        let subscription = cx.subscribe(&playback, |this, _, event, cx| {
            if matches!(event, PlaybackEvent::SnapshotChanged) {
                this.reload(cx);
            }
        });
        let mut this = Self {
            playback,
            song_id: None,
            data: Resource::new(),
            _playback_subscription: subscription,
        };
        this.reload(cx);
        this
    }

    /// 互动区计数快照。渲染时现读，不在别处存镜像。
    pub fn counts(&self) -> [Option<u64>; 2] {
        self.data.value().copied().unwrap_or([None; 2])
    }

    /// 同一首歌不重复请求；换歌取消旧请求、清空计数。
    fn reload(&mut self, cx: &mut Context<Self>) {
        let song_id = self
            .playback
            .read(cx)
            .snapshot()
            .current_song
            .as_ref()
            .map(|song| song.id);
        if self.song_id == song_id {
            return;
        }
        self.song_id = song_id;
        self.data.reset();
        cx.notify();
        let Some(id) = song_id else {
            return;
        };
        let client = MusicApi::global(cx).client.clone();
        self.data
            .fetch("song counts", cx, |this| &mut this.data, async move {
                let counts = MusicApi::song_counts(client, id).await;
                Ok(counts.map(|count| {
                    count
                        .map_err(|error| log::warn!("song count: {error}"))
                        .ok()
                }))
            });
    }
}
