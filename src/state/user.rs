use gpui::{Context, ReadGlobal};
use serde::{Deserialize, Serialize};

use crate::api::MusicApi;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub user_id: u64,
    pub nickname: String,
    pub avatar_url: String,
    pub background_url: Option<String>,
    pub signature: Option<String>,
    pub gender: Option<u8>,
    pub birthday: Option<i64>,
    pub create_time: Option<u64>,
    pub vip_type: Option<u32>,
    pub user_type: Option<u32>,
    pub auth_status: Option<u32>,
}

impl UserProfile {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let api = MusicApi::global(cx);
        let request = api
            .runtime
            .spawn(MusicApi::user_profile(api.client.clone()));
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("账号请求任务失败".into()));
            let _ = this.update(cx, |profile, cx| {
                match result {
                    Ok(loaded) => *profile = loaded,
                    Err(message) => {
                        eprintln!("{message}");
                        profile.nickname = "未登录".into();
                    }
                }
                // 已有观察者会同步刷新页头和「我喜欢的音乐」中的用户资料。
                cx.notify();
            });
        })
        .detach();

        Self {
            nickname: "加载中…".into(),
            ..Default::default()
        }
    }
}
