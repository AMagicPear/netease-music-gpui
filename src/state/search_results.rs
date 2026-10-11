use gpui::{Context, ReadGlobal};

use crate::api::MusicApi;
use crate::models::{Playlist, Song};

/// 当前搜索关键词的结果；页面 View 只管展示，加载状态都收在这里。
///
/// 单曲和歌单各存一份：搜索只发单曲请求，歌单等切到「歌单」分类时再取，
/// 两种分类的加载状态因此互不牵连。
#[derive(Default)]
pub struct SearchResults {
    pub keywords: Option<String>,
    pub songs: Vec<Song>,
    pub playlists: Vec<Playlist>,
    pub loading: bool,
    pub playlists_loading: bool,
    pub error: Option<String>,
    pub playlists_error: Option<String>,
    generation: u64,
    request: Option<tokio::task::AbortHandle>,
    playlists_request: Option<tokio::task::AbortHandle>,
    /// 当前关键词的歌单是否取过。单看 `playlists_error == None` 分不出「还没取」
    /// 和「取到了」，需要单独记：前者该发请求，后者不该。
    playlists_loaded: bool,
}

impl SearchResults {
    /// 发起搜索。同一个关键词且已有结果（或正在加载）时不重复请求。
    pub fn search(&mut self, keywords: String, cx: &mut Context<Self>) {
        if self.keywords.as_deref() == Some(keywords.as_str())
            && (self.loading || self.error.is_none())
        {
            return;
        }
        self.clear();
        self.keywords = Some(keywords.clone());
        self.loading = true;
        let generation = self.generation;
        let api = MusicApi::global(cx);
        // client 先克隆出来：异步块不能捕获 api 引用，否则它活得比 cx 的借用久。
        let client = api.client.clone();
        let request = api.runtime.spawn({
            let keywords = keywords.clone();
            async move { MusicApi::search_songs(client, &keywords).await }
        });
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("搜索请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.finish(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 取当前关键词的歌单结果，由页面在切到「歌单」分类时调用。
    /// 不预取：十个分类里绝大多数人只看单曲，顺手拉一遍歌单纯属浪费一次往返。
    /// 守卫和 `search` 对称 —— 取到过（或正在取）就不重发，只有上次失败才重来。
    pub fn search_playlists(&mut self, cx: &mut Context<Self>) {
        let Some(keywords) = self.keywords.clone() else {
            return;
        };
        if self.playlists_loading || (self.playlists_loaded && self.playlists_error.is_none()) {
            return;
        }
        self.playlists_loaded = true;
        self.playlists_loading = true;
        self.playlists_error = None;
        let generation = self.generation;
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let request = api
            .runtime
            .spawn(async move { MusicApi::search_playlists(client, &keywords).await });
        self.playlists_request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("歌单搜索请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.finish_playlists(generation, result) {
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
        if let Some(request) = self.playlists_request.take() {
            request.abort();
        }
        self.keywords = None;
        self.songs.clear();
        self.playlists.clear();
        self.loading = false;
        self.playlists_loading = false;
        self.playlists_loaded = false;
        self.error = None;
        self.playlists_error = None;
    }

    fn finish(&mut self, generation: u64, result: Result<Vec<Song>, String>) -> bool {
        // 取消网络任务之外，再拦截已经送回界面的旧结果：连续搜两个词时，
        // 先发出、后返回的那个请求不能覆盖后一个关键词的列表。
        if self.generation != generation {
            return false;
        }
        self.request = None;
        self.loading = false;
        match result {
            Ok(songs) => {
                self.songs = songs;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
        true
    }

    /// 和 `finish` 同一套 generation 校验：换关键词时 `clear()` 递增了 generation，
    /// 旧关键词的歌单请求即使后返回也会被挡掉。
    fn finish_playlists(&mut self, generation: u64, result: Result<Vec<Playlist>, String>) -> bool {
        if self.generation != generation {
            return false;
        }
        self.playlists_request = None;
        self.playlists_loading = false;
        match result {
            Ok(playlists) => {
                self.playlists = playlists;
                self.playlists_error = None;
            }
            Err(message) => self.playlists_error = Some(message),
        }
        true
    }
}

impl Drop for SearchResults {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
        if let Some(request) = self.playlists_request.take() {
            request.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: u64) -> Song {
        Song {
            id,
            ..Default::default()
        }
    }

    #[test]
    fn switching_keywords_discards_old_results() {
        let mut results = SearchResults::default();
        results.keywords = Some("旧词".into());
        results.loading = true;
        results.songs = vec![song(1)];
        results.generation = 3;
        // 旧关键词的迟到结果不能落到新关键词上。
        assert!(!results.finish(2, Err("旧请求失败".into())));
        // 新一次 search() 会先 clear()：generation 递增后，上一轮请求的返回一律作废。
        results.generation = 4;
        assert!(!results.finish(3, Err("切走前的旧结果".into())));
        assert!(results.loading && results.error.is_none());
        assert!(results.finish(4, Ok(vec![song(2), song(3)])));
        assert!(!results.loading);
        assert_eq!(results.songs.iter().map(|song| song.id).collect::<Vec<_>>(), [2, 3]);
        results.clear();
        assert!(results.keywords.is_none() && results.songs.is_empty() && results.error.is_none());
        assert_eq!(results.generation, 5);
    }

    /// 歌单是懒加载的：换关键词时不能顺手把它记成「已取过」，
    /// 否则切到歌单分类会以为这个词已经查过、什么都不发。
    #[test]
    fn switching_keywords_also_resets_the_lazy_playlist_request() {
        let mut results = SearchResults::default();
        results.keywords = Some("旧词".into());
        results.generation = 9;
        assert!(results.finish_playlists(9, Ok(vec![playlist(7, "旧歌单")])));
        results.playlists_loaded = true;
        assert_eq!(results.playlists.len(), 1);

        results.clear();
        assert!(!results.playlists_loaded);
        assert!(results.playlists.is_empty());
        assert!(!results.playlists_loading);

        // 新关键词下，迟到的旧歌单结果同样作废。
        results.generation = 10;
        assert!(!results.finish_playlists(9, Ok(vec![playlist(8, "迟到的旧歌单")])));
        assert!(results.playlists.is_empty());
    }

    /// 夹具照抄搜索结果的歌单项：没有 createTime，收藏数叫 bookCount。
    fn playlist(id: u64, name: &str) -> Playlist {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "creator": {"userId": 1, "nickname": "我", "avatarUrl": null},
            "trackCount": 10,
            "playCount": 100,
            "bookCount": 3,
            "specialType": 0
        }))
        .expect("夹具应当能解析")
    }
}
