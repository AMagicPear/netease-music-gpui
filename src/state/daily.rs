//! 每日推荐歌曲的独立状态：登录后按当天拉取，切换账号或手动重试时重新请求。

use gpui::{Context, Entity, ReadGlobal, Subscription};

use super::account::AccountState;
use crate::api::MusicApi;
use crate::models::Song;

/// 日推必须登录；游客没有数据，页面根据 `needs_login` 给提示。
pub struct DailySongsStore {
    pub songs: Vec<Song>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
    /// 已加载过的身份：`Some(0)` 表示游客，`None` 表示还没取过。
    user_id: Option<u64>,
    generation: u64,
    request: Option<tokio::task::AbortHandle>,
    _user_subscription: Option<Subscription>,
}

impl DailySongsStore {
    pub fn new(user: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&user, |this, user, cx| {
            this.load(user.read(cx).profile.user_id, false, cx);
        });
        let mut this = Self {
            songs: Vec::new(),
            loading: false,
            loaded: false,
            error: None,
            user_id: None,
            generation: 0,
            request: None,
            _user_subscription: Some(subscription),
        };
        let (user_id, loading) = {
            let account = user.read(cx);
            (account.profile.user_id, account.loading)
        };
        // 会话恢复完成后由账号通知触发加载，避免先按游客身份走一遍空态。
        if user_id != 0 || !loading {
            this.load(user_id, false, cx);
        } else {
            // 等账号结果期间页面显示加载中，而不是先闪一下「暂无推荐」。
            this.loading = true;
        }
        this
    }

    /// 游客没有日推：页面据此显示登录提示，而不是「暂无推荐」。
    pub fn needs_login(&self) -> bool {
        self.user_id == Some(0)
    }

    /// 出错后的手动重试；成功过的情况下也允许强制刷新。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(self.user_id.unwrap_or_default(), true, cx);
    }

    fn load(&mut self, user_id: u64, force: bool, cx: &mut Context<Self>) {
        if !self.should_load(user_id, force) {
            return;
        }
        self.abort_request();
        if self.user_id != Some(user_id) {
            self.songs.clear();
            self.loaded = false;
        }
        self.user_id = Some(user_id);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.error = None;
        if user_id == 0 {
            self.loading = false;
            self.loaded = true;
            cx.notify();
            return;
        }
        self.loading = true;
        let (runtime, client) = {
            let api = MusicApi::global(cx);
            (api.runtime.handle().clone(), api.client.clone())
        };
        let request = runtime.spawn(MusicApi::daily_songs(client));
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("每日推荐请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.apply(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 同一身份只取一次；失败或主动刷新时允许重试。
    fn should_load(&self, user_id: u64, force: bool) -> bool {
        force || self.user_id != Some(user_id) || self.error.is_some()
    }

    fn apply(&mut self, generation: u64, result: Result<Vec<Song>, String>) -> bool {
        if self.generation != generation {
            return false;
        }
        self.request = None;
        self.loading = false;
        match result {
            Ok(songs) => {
                self.songs = songs;
                self.loaded = true;
                self.error = None;
            }
            // 失败保留上一批歌曲，重试成功后整体替换。
            Err(message) => self.error = Some(message),
        }
        true
    }

    fn abort_request(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
    }
}

impl Drop for DailySongsStore {
    fn drop(&mut self) {
        self.abort_request();
    }
}

#[cfg(test)]
mod tests {
    use super::DailySongsStore;
    use crate::models::Song;

    fn store() -> DailySongsStore {
        DailySongsStore {
            songs: Vec::new(),
            loading: false,
            loaded: false,
            error: None,
            user_id: None,
            generation: 0,
            request: None,
            _user_subscription: None,
        }
    }

    fn song(id: u64) -> Song {
        Song {
            id,
            name: format!("歌曲 {id}"),
            ..Default::default()
        }
    }

    #[test]
    fn same_identity_loads_once_until_it_fails() {
        let mut this = store();
        assert!(this.should_load(42, false));
        this.user_id = Some(42);
        assert!(!this.should_load(42, false));
        this.error = Some("网络错误".into());
        assert!(this.should_load(42, false));
        this.error = None;
        assert!(this.should_load(42, true));
        assert!(this.should_load(7, false));
        assert!(this.should_load(0, false));
    }

    #[test]
    fn stale_results_are_discarded_and_failures_keep_previous_songs() {
        let mut this = store();
        this.generation = 2;
        this.loading = true;
        assert!(!this.apply(1, Ok(vec![song(1)])));
        assert!(this.songs.is_empty());
        assert!(this.loading);

        assert!(this.apply(2, Ok(vec![song(2), song(3)])));
        assert!(!this.loading);
        assert!(this.loaded);
        assert_eq!(this.songs.len(), 2);

        this.loading = true;
        assert!(this.apply(2, Err("每日推荐请求失败".into())));
        assert!(!this.loading);
        assert!(this.error.is_some());
        assert_eq!(this.songs.len(), 2);
    }

    #[test]
    fn guests_have_no_requested_songs() {
        let mut this = store();
        this.user_id = Some(0);
        this.loaded = true;
        assert!(this.needs_login());
        assert!(this.songs.is_empty());
        assert!(!this.should_load(0, false));
    }
}
