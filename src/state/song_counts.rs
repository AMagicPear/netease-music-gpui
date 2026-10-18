//! 歌曲互动计数（喜欢 / 评论数）：跟着当前播放歌曲加载，过期结果由 store 自己丢弃。
//!
//! 播放栏只读 `counts()`；请求取消与过期校验都留在这里。

use gpui::{Context, Entity, ReadGlobal, Subscription};

use crate::api::MusicApi;
use crate::playback::{PlaybackController, PlaybackEvent};

/// 两个互动区图标旁的计数（下标 0 = 喜欢，1 = 评论）；`None` 表示没拿到。
pub struct SongCounts {
    playback: Entity<PlaybackController>,
    /// 计数对应的歌曲；请求身份另外校验，避免 A → B → A 时旧结果覆盖新结果。
    song_id: Option<u64>,
    counts: [Option<u64>; 2],
    request: Option<tokio::task::AbortHandle>,
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
            counts: [None; 2],
            request: None,
            _playback_subscription: subscription,
        };
        this.reload(cx);
        this
    }

    /// 互动区计数快照。渲染时现读，不在别处存镜像。
    pub fn counts(&self) -> [Option<u64>; 2] {
        self.counts
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
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.song_id = song_id;
        self.counts = [None; 2];
        cx.notify();
        let Some(song_id) = song_id else {
            return;
        };
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::song_counts(api.client.clone(), song_id));
        let request_id = request.id();
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                if !this
                    .request
                    .as_ref()
                    .is_some_and(|request| request.id() == request_id)
                {
                    return;
                }
                this.request = None;
                match result {
                    Ok(counts) => {
                        for (index, count) in counts.into_iter().enumerate() {
                            match count {
                                Ok(count) => this.counts[index] = Some(count),
                                Err(message) => eprintln!("{message}"),
                            }
                        }
                        cx.notify();
                    }
                    Err(_) => eprintln!("歌曲互动计数任务失败"),
                }
            });
        })
        .detach();
    }
}

impl Drop for SongCounts {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
    }
}
