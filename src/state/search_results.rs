use gpui::{Context, ReadGlobal};

use crate::api::MusicApi;
use crate::models::Song;

/// 当前搜索关键词的结果；页面 View 只管展示，加载状态都收在这里。
#[derive(Default)]
pub struct SearchResults {
    pub keywords: Option<String>,
    pub songs: Vec<Song>,
    pub loading: bool,
    pub error: Option<String>,
    generation: u64,
    request: Option<tokio::task::AbortHandle>,
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

    fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.keywords = None;
        self.songs.clear();
        self.loading = false;
        self.error = None;
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
}

impl Drop for SearchResults {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
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
}
