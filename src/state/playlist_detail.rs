use gpui::{Context, ReadGlobal};

use super::{playlist::Playlist, song::Song};
use crate::api::MusicApi;

/// 当前打开的歌单数据；排序、标签和 hover 留在页面 View 中。
#[derive(Default)]
pub struct PlaylistDetail {
    pub id: Option<u64>,
    pub playlist: Option<Playlist>,
    pub songs: Vec<Song>,
    pub loading: bool,
    pub error: Option<String>,
    generation: u64,
    request: Option<tokio::task::AbortHandle>,
}

impl PlaylistDetail {
    pub fn open(&mut self, id: Option<u64>, summary: Option<Playlist>, cx: &mut Context<Self>) {
        if self.id == id && (self.loading || self.error.is_none()) {
            return;
        }
        self.clear();
        self.id = id;
        self.playlist = summary;
        let Some(id) = id else {
            cx.notify();
            return;
        };
        self.loading = true;
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::playlist(api.client.clone(), id));
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("歌单请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.finish(id, generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.id = None;
        self.playlist = None;
        self.songs.clear();
        self.loading = false;
        self.error = None;
    }

    fn finish(
        &mut self,
        id: u64,
        generation: u64,
        result: Result<(Playlist, Vec<Song>), String>,
    ) -> bool {
        // 取消网络任务之外，再拦截已经送回界面的旧结果。
        if self.id != Some(id) || self.generation != generation {
            return false;
        }
        self.request = None;
        self.loading = false;
        match result {
            Ok((playlist, songs)) => {
                self.playlist = Some(playlist);
                self.songs = songs;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
        true
    }
}

impl Drop for PlaylistDetail {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_playlists_discards_old_results_and_clears_rows() {
        let mut detail = PlaylistDetail::default();
        detail.id = Some(2);
        detail.loading = true;
        detail.songs = vec![Song::default()];
        detail.generation = 3;
        assert!(!detail.finish(1, 2, Err("旧歌单失败".into())));
        assert!(!detail.finish(2, 1, Err("切回同一个歌单的旧结果".into())));
        assert!(detail.loading && detail.error.is_none());
        assert!(detail.finish(2, 3, Err("当前歌单失败".into())));
        assert!(!detail.loading);
        assert_eq!(detail.error.as_deref(), Some("当前歌单失败"));
        detail.clear();
        assert!(detail.id.is_none() && detail.songs.is_empty() && detail.error.is_none());
        assert_eq!(detail.generation, 4);
    }
}
