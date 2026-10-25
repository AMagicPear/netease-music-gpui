use super::{
    account::AccountState,
    resource::{DataState, Resource},
};
use crate::{api::MusicApi, models::Song};
use gpui::{Context, Entity, ReadGlobal, Subscription};

pub struct DailySongsStore {
    songs: Resource<Vec<Song>>,
    user_id: Option<u64>,
    _account: Subscription,
}

impl DailySongsStore {
    pub fn new(account: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&account, |this, account, cx| {
            let account = account.read(cx);
            if !account.loading && this.user_id != Some(account.profile.user_id) {
                this.set_account(account.profile.user_id, cx);
            }
        });
        let mut this = Self {
            songs: Resource::new(),
            user_id: None,
            _account: subscription,
        };
        let account = account.read(cx);
        if !account.loading {
            this.set_account(account.profile.user_id, cx);
        }
        this
    }

    fn set_account(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.songs.reset();
        self.user_id = Some(user_id);
        self.revalidate(cx);
    }

    pub fn songs(&self) -> &[Song] {
        self.songs.value().map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn state(&self) -> DataState<'_, Vec<Song>> {
        self.songs.state()
    }
    pub fn needs_login(&self) -> bool {
        self.user_id == Some(0)
    }
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.revalidate(cx);
    }

    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = self.user_id else {
            return;
        };
        if user_id == 0 {
            self.songs.set(Vec::new());
            cx.notify();
            return;
        }
        let client = MusicApi::global(cx).client.clone();
        self.songs.fetch(
            "daily songs",
            cx,
            |this| &mut this.songs,
            MusicApi::daily_songs(client),
        );
    }
}
