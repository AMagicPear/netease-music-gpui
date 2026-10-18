//! 推荐页的「推荐歌单」数据：跟着登录状态加载，过期结果由 store 自己丢弃。
//!
//! 页面只读快照；请求取消、新旧账号校验（generation）和「同一账号只取一次」
//! 的守卫都收在这里，分工同 `SearchResults`。

use gpui::{Context, Entity, ReadGlobal, Subscription};

use super::account::AccountState;
use crate::api::MusicApi;
use crate::models::RecommendedPlaylist;

/// 推荐歌单。未登录时不发请求：`/recommend/resource` 要登录态，
/// 此时列表为空、`error` 也为空，页面据此显示登录引导而不是报错。
pub struct RecommendedPlaylists {
    pub playlists: Vec<RecommendedPlaylist>,
    pub loading: bool,
    pub error: Option<String>,
    /// 已加载过的账号；`None` 表示还没为任何账号取过。
    user_id: Option<u64>,
    generation: u64,
    request: Option<tokio::task::AbortHandle>,
    /// 刷新按钮置位：绕过「同一账号只取一次」的守卫，强制重取。
    force_reload: bool,
    _user_subscription: Option<Subscription>,
}

impl RecommendedPlaylists {
    pub fn new(user: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        // 账号变化（扫码登录 / 退出 / 启动时会话恢复完成）时重新加载。
        let subscription = cx.observe(&user, |this, user, cx| {
            this.load(user.read(cx).profile.user_id, cx);
        });
        let mut this = Self {
            playlists: Vec::new(),
            loading: false,
            error: None,
            user_id: None,
            generation: 0,
            request: None,
            force_reload: false,
            _user_subscription: Some(subscription),
        };
        let (user_id, loading) = {
            let account = user.read(cx);
            (account.profile.user_id, account.loading)
        };
        // 会话还在恢复中（user_id == 0 且 loading）时先不取：
        // observe 会在账号落地后补上这一次加载。
        if user_id != 0 || !loading {
            this.load(user_id, cx);
        }
        this
    }

    /// 刷新按钮：重新拉一份推荐歌单。进行中的旧请求会被取消，
    /// 结果由 generation 挡掉，不会覆盖新一批。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.force_reload = true;
        self.load(self.user_id.unwrap_or_default(), cx);
    }

    fn load(&mut self, user_id: u64, cx: &mut Context<Self>) {
        if user_id == 0 {
            self.abort_request();
            self.playlists.clear();
            self.error = None;
            self.loading = false;
            self.user_id = None;
            cx.notify();
            return;
        }
        let force = std::mem::take(&mut self.force_reload);
        if !self.should_load(user_id, force) {
            return;
        }
        self.abort_request();
        self.user_id = Some(user_id);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.error = None;
        self.loading = true;
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::recommend_playlists(api.client.clone()));
        self.request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("推荐歌单请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.apply(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 是否该为 `user_id` 发请求：同一账号取过（或正在取）就不再发，
    /// 只有上次失败才允许在下次账号通知时重试；`force` 供刷新按钮绕过守卫。
    fn should_load(&self, user_id: u64, force: bool) -> bool {
        if user_id == 0 {
            return false;
        }
        force || self.user_id != Some(user_id) || self.error.is_some()
    }

    /// 落地一次请求结果；`generation` 过期（切账号 / 刷新取消了旧请求）时
    /// 整份丢弃，连 loading 都不碰——那个状态归新请求管。
    fn apply(&mut self, generation: u64, result: Result<Vec<RecommendedPlaylist>, String>) -> bool {
        if self.generation != generation {
            return false;
        }
        self.request = None;
        self.loading = false;
        match result {
            Ok(playlists) => {
                self.playlists = playlists;
                self.error = None;
            }
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

impl Drop for RecommendedPlaylists {
    fn drop(&mut self) {
        self.abort_request();
    }
}

#[cfg(test)]
mod tests {
    use super::{RecommendedPlaylist, RecommendedPlaylists};

    fn store() -> RecommendedPlaylists {
        RecommendedPlaylists {
            playlists: Vec::new(),
            loading: false,
            error: None,
            user_id: None,
            generation: 0,
            request: None,
            force_reload: false,
            _user_subscription: None,
        }
    }

    fn recommended(id: u64, name: &str) -> RecommendedPlaylist {
        RecommendedPlaylist {
            id,
            name: name.into(),
            pic_url: None,
            play_count: 0,
        }
    }

    #[test]
    fn same_account_loads_once_until_it_fails() {
        let mut this = store();
        // 未取过：该取。
        assert!(this.should_load(42, false));
        this.user_id = Some(42);
        this.loading = true;
        // 正在取：账号通知（比如会员信息回来）不重复发。
        assert!(!this.should_load(42, false));
        this.loading = false;
        // 取成功后 error 仍为 None：账号没变就不再取。
        assert!(!this.should_load(42, false));
        // 上次失败：允许在下次通知时重试。
        this.error = Some("网络错误".into());
        assert!(this.should_load(42, false));
        // 刷新按钮强制重取，与成败无关。
        this.error = None;
        assert!(this.should_load(42, true));
        // 未登录不取。
        assert!(!this.should_load(0, false));
        // 换账号要取。
        assert!(this.should_load(7, false));
    }

    #[test]
    fn stale_results_are_discarded_after_account_switch_or_refresh() {
        let mut this = store();
        this.user_id = Some(1);
        // 4 = 新一批（切账号或点了刷新）的编号；3 = 被取消的旧请求的编号。
        this.generation = 4;
        this.loading = true;
        assert!(!this.apply(3, Ok(vec![recommended(1, "旧账号的歌单")])));
        assert!(this.loading && this.playlists.is_empty() && this.error.is_none());
        // 新一批落地。
        assert!(this.apply(4, Ok(vec![recommended(2, "新账号的歌单")])));
        assert!(!this.loading);
        assert_eq!(this.playlists.len(), 1);
        assert_eq!(this.playlists[0].name, "新账号的歌单");
        assert_eq!(this.playlists[0].id, 2);
    }

    #[test]
    fn failed_request_records_error_and_keeps_previous_list() {
        let mut this = store();
        this.user_id = Some(1);
        this.generation = 1;
        this.loading = true;
        this.playlists = vec![recommended(1, "旧的一批")];
        assert!(this.apply(1, Err("推荐歌单请求失败".into())));
        assert!(!this.loading);
        assert!(this.error.is_some());
        // 刷新失败不清空已有列表：旧内容比空列表好。
        assert_eq!(this.playlists.len(), 1);
    }
}
