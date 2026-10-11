use gpui::{Context, ReadGlobal};
use ncm_api_rs::create_client;

use crate::{
    api::MusicApi,
    models::{UserProfile, VipInfo},
    persistence::Persistence,
};

/// 登录账号的共享状态。歌单创建者只使用 models::UserProfile。
pub struct AccountState {
    pub profile: UserProfile,
    pub vip: Option<VipInfo>,
    pub loading: bool,
    pub error: Option<String>,
    persistence: Persistence,
    generation: u64,
}

impl AccountState {
    pub fn new(persistence: Persistence, has_session: bool, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            profile: UserProfile {
                nickname: "未登录".into(),
                ..Default::default()
            },
            vip: None,
            loading: false,
            error: None,
            persistence,
            generation: 0,
        };
        if has_session {
            this.refresh(cx);
        }
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.loading = true;
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::user_profile(api.client.clone()));
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("账号请求任务失败".into()));
            let _ = this.update(cx, |account, cx| {
                if account.generation != generation {
                    return;
                }
                account.loading = false;
                match result {
                    Ok(profile) => account.set_profile(profile, cx),
                    Err(_) => {
                        account.profile = UserProfile {
                            nickname: "未登录".into(),
                            ..Default::default()
                        };
                        account.error = Some("尚未登录或登录已失效，请扫码登录".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub fn sign_in(&mut self, cookie: String, profile: UserProfile, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        let persistence = self.persistence.clone();
        let saved_cookie = cookie.clone();
        let request = MusicApi::global(cx)
            .runtime
            .spawn_blocking(move || persistence.save_session(&saved_cookie));
        // 手机确认后由账号 Entity 完成保存；关闭扫码窗口也不会丢掉已授权的登录。
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |account, cx| {
                if account.generation != generation {
                    return;
                }
                account.loading = false;
                match result {
                    Ok(Ok(())) => account.accept_login(cookie, profile, cx),
                    _ => account.error = Some("保存登录凭据失败，请重新扫码".into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn accept_login(&mut self, cookie: String, profile: UserProfile, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        cx.global_mut::<MusicApi>().client = create_client(Some(cookie));
        self.loading = false;
        self.error = None;
        self.set_profile(profile, cx);
    }

    fn set_profile(&mut self, profile: UserProfile, cx: &mut Context<Self>) {
        let user_id = profile.user_id;
        let generation = self.generation;
        self.profile = profile;
        self.vip = None;
        cx.notify();
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::vip_info(api.client.clone(), user_id));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(vip)) = request.await {
                let _ = this.update(cx, |account, cx| {
                    if account.generation == generation {
                        account.vip = Some(vip);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        let persistence = self.persistence.clone();
        let request = MusicApi::global(cx)
            .runtime
            .spawn_blocking(move || persistence.clear_session());
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |account, cx| {
                if account.generation != generation {
                    return;
                }
                account.loading = false;
                match result {
                    Ok(Ok(())) => {
                        cx.global_mut::<MusicApi>().client = create_client(None);
                        account.profile = UserProfile {
                            nickname: "未登录".into(),
                            ..Default::default()
                        };
                        account.vip = None;
                    }
                    _ => account.error = Some("清除登录凭据失败，请重试".into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
