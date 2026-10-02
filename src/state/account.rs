use gpui::{Context, ReadGlobal};

use crate::{
    api::MusicApi,
    models::{UserProfile, VipInfo},
};

/// 登录账号的共享状态。歌单创建者只使用 models::UserProfile。
#[derive(Default)]
pub struct AccountState {
    pub profile: UserProfile,
    pub vip: Option<VipInfo>,
}

impl AccountState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::user_profile(api.client.clone()));
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("账号请求任务失败".into()));
            let profile = match result {
                Ok(profile) => profile,
                Err(message) => {
                    eprintln!("{message}");
                    let _ = this.update(cx, |account, cx| {
                        account.profile.nickname = "未登录".into();
                        cx.notify();
                    });
                    return;
                }
            };
            let user_id = profile.user_id;
            let Ok(request) = this.update(cx, |account, cx| {
                account.profile = profile;
                // 账号就绪立即通知音乐库，会员请求独立补充。
                cx.notify();
                let api = MusicApi::global(cx);
                api.runtime
                    .spawn(MusicApi::vip_info(api.client.clone(), user_id))
            }) else {
                return;
            };
            match request
                .await
                .unwrap_or_else(|_| Err("会员请求任务失败".into()))
            {
                Ok(vip) => {
                    let _ = this.update(cx, |account, cx| {
                        if account.profile.user_id == user_id {
                            account.vip = Some(vip);
                            cx.notify();
                        }
                    });
                }
                Err(message) => eprintln!("{message}"),
            }
        })
        .detach();
        Self {
            profile: UserProfile {
                nickname: "加载中…".into(),
                ..Default::default()
            },
            vip: None,
        }
    }
}
