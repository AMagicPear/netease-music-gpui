//! 每日推荐歌曲的独立状态：登录后按当天拉取，切换账号或手动重试时重新请求。

use gpui::{Context, Entity, ReadGlobal, Subscription};

use super::account::AccountState;
use super::resource::{DataState, Resource};
use crate::api::MusicApi;
use crate::models::Song;

/// 日推必须登录；游客没有数据，页面根据 `needs_login` 给提示。
pub struct DailySongsStore {
    /// 每天的推荐歌曲。显示层只管读它，数据层只管拉取和更新。
    songs: Resource<Vec<Song>>,
    /// 已加载过的身份：`Some(0)` 表示游客，`None` 表示还没取过。
    user_id: Option<u64>,
    _user_subscription: Option<Subscription>,
}

impl DailySongsStore {
    pub fn new(user: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&user, |this, user, cx| {
            this.load(user.read(cx).profile.user_id, false, cx);
        });
        let mut this = Self {
            songs: Resource::new(),
            user_id: None,
            _user_subscription: Some(subscription),
        };
        let (user_id, loading) = {
            let account = user.read(cx);
            (account.profile.user_id, account.loading)
        };
        // 会话恢复完成后由账号通知触发加载，避免先按游客身份走一遍空态。
        if user_id != 0 || !loading {
            this.load(user_id, false, cx);
        }
        this
    }

    /// 当前的歌曲。显示层按行取用：还没拿到过时是空的。
    pub fn songs(&self) -> &[Song] {
        self.songs.value().map(|songs| songs.as_slice()).unwrap_or(&[])
    }

    /// 显示状态：显示层按它决定给内容还是给提示。
    pub fn state(&self) -> DataState<'_, Vec<Song>> {
        self.songs.state()
    }

    /// 游客没有日推：页面据此显示登录提示，而不是「暂无推荐」。
    pub fn needs_login(&self) -> bool {
        self.user_id == Some(0)
    }

    /// 出错后的手动重试；成功过的情况下也允许强制刷新。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(self.user_id.unwrap_or_default(), true, cx);
    }

    /// 切回页面时的静默刷新：重新拉一次，旧内容不清空，新数据到了再替换。
    ///
    /// 已经在拉就算了，不打断。
    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        if self.songs.is_loading() {
            return;
        }
        self.load(self.user_id.unwrap_or_default(), true, cx);
    }

    fn load(&mut self, user_id: u64, force: bool, cx: &mut Context<Self>) {
        if !self.should_load(user_id, force) {
            return;
        }
        if self.user_id != Some(user_id) {
            // 换身份：旧数据是别人的，不能留着。
            self.songs.reset();
        }
        self.user_id = Some(user_id);
        if user_id == 0 {
            // 游客确定没有日推：给一份空数据，页面据此显示登录提示而不是转圈。
            self.songs.set(Vec::new());
            cx.notify();
            return;
        }
        let generation = self.songs.begin();
        let (runtime, client) = {
            let api = MusicApi::global(cx);
            (api.runtime.handle().clone(), api.client.clone())
        };
        let request = runtime.spawn(MusicApi::daily_songs(client));
        self.songs.track(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("每日推荐请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.songs.apply(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 同一身份只取一次；失败或主动刷新时允许重试。
    fn should_load(&self, user_id: u64, force: bool) -> bool {
        force || self.user_id != Some(user_id) || self.songs.error().is_some()
    }
}

impl Drop for DailySongsStore {
    fn drop(&mut self) {
        self.songs.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::DailySongsStore;
    use super::super::resource::Resource;
    use crate::models::Song;

    fn store() -> DailySongsStore {
        DailySongsStore {
            songs: Resource::new(),
            user_id: None,
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
        // 失败过一次就允许重试。
        this.songs.begin();
        let generation = this.songs.begin();
        this.songs.apply(generation, Err("网络错误".into()));
        assert!(this.should_load(42, false));
        assert!(this.should_load(42, true));
        assert!(this.should_load(7, false));
        assert!(this.should_load(0, false));
    }

    #[test]
    fn a_failure_keeps_the_previous_songs() {
        let mut this = store();
        let generation = this.songs.begin();
        assert!(this.songs.apply(generation, Ok(vec![song(2), song(3)])));
        assert_eq!(this.songs.value().unwrap().len(), 2);

        let generation = this.songs.begin();
        assert!(this.songs.apply(generation, Err("每日推荐请求失败".into())));
        assert!(this.songs.error().is_some());
        // 旧歌曲还在：失败不该把已经拿到的内容打回空态。
        assert_eq!(this.songs.value().unwrap().len(), 2);
    }

    #[test]
    fn guests_have_no_requested_songs() {
        let mut this = store();
        this.user_id = Some(0);
        this.songs.set(Vec::new());
        assert!(this.needs_login());
        assert!(this.songs.value().unwrap().is_empty());
        assert!(!this.should_load(0, false));
    }
}
