use super::{
    account::AccountState,
    resource::{Resource, request},
};
use crate::{api::MusicApi, models::MessageItem};
use gpui::{Context, Entity, ReadGlobal, Subscription};

pub const TAB_PRIVATE: usize = 0;
pub const TAB_COMMENTS: usize = 1;
pub const TAB_MENTIONS: usize = 2;
pub const TAB_COUNT: usize = 4;

pub struct MessagesStore {
    pub tabs: [Resource<(Vec<MessageItem>, u32)>; TAB_COUNT],
    pub user_id: u64,
    active: Option<usize>,
    read: Resource<()>,
    _account: Subscription,
}

impl MessagesStore {
    pub fn new(account: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let user_id = account.read(cx).profile.user_id;
        let subscription = cx.observe(&account, |this, account, cx| {
            let id = account.read(cx).profile.user_id;
            if this.user_id == id {
                return;
            }
            this.user_id = id;
            for tab in &mut this.tabs {
                tab.reset();
            }
            this.read.reset();
            if let Some(tab) = this.active {
                this.load(tab, cx);
            }
            cx.notify();
        });
        Self {
            tabs: Default::default(),
            user_id,
            active: None,
            read: Resource::new(),
            _account: subscription,
        }
    }

    pub fn show(&mut self, tab: Option<usize>, cx: &mut Context<Self>) {
        self.active = tab;
        if let Some(tab) = tab {
            self.load(tab, cx);
        }
    }

    fn load(&mut self, tab: usize, cx: &mut Context<Self>) {
        let uid = self.user_id;
        if uid == 0 {
            self.tabs[tab].set((Vec::new(), 0));
            cx.notify();
            return;
        }
        if self.tabs[tab].is_loading() {
            return;
        }
        let generation = self.tabs[tab].begin();
        let client = MusicApi::global(cx).client.clone();
        self.tabs[tab].track(request(
            cx,
            "messages",
            fetch_tab(client, tab, uid),
            move |this, result, cx| {
                if this.tabs[tab].apply(generation, result) {
                    cx.notify();
                }
            },
        ));
        cx.notify();
    }

    pub fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        if self.user_id == 0 || self.read.is_loading() {
            return;
        }
        for tab in &mut self.tabs {
            // 已经在途的旧未读数不能盖回本地修改。
            tab.cancel();
            if let Some((items, unread)) = tab.value_mut() {
                *unread = 0;
                for item in items {
                    item.unread = 0;
                }
            }
        }
        let generation = self.read.begin();
        let client = MusicApi::global(cx).client.clone();
        self.read.track(request(
            cx,
            "mark messages read",
            async move { MusicApi::clear_message_counts(&client).await },
            move |this, result, cx| {
                if this.read.apply(generation, result) {
                    if let Some(tab) = this.active {
                        this.load(tab, cx);
                    }
                    cx.notify();
                }
            },
        ));
        cx.notify();
    }
}

/// 四个 tab 各自的接口；统一成「条目 + 账号级未读数」的返回。
async fn fetch_tab(
    client: crate::api::ApiClient,
    tab: usize,
    uid: u64,
) -> Result<(Vec<MessageItem>, u32), String> {
    match tab {
        TAB_PRIVATE => MusicApi::private_messages(&client, uid).await,
        TAB_COMMENTS => MusicApi::comment_messages(&client, uid)
            .await
            .map(|items| (items, 0)),
        TAB_MENTIONS => MusicApi::mention_messages(&client).await,
        // 其余（通知）没有账号级未读数。
        _ => MusicApi::notice_messages(&client)
            .await
            .map(|items| (items, 0)),
    }
}
