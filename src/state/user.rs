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
    /// 独立的 vip_info 接口返回值，不属于 user_account 的 profile。
    #[serde(skip)]
    pub vip: Option<VipInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VipInfo {
    pub red_vip_level: u32,
    pub associator: Option<VipMembership>,
    pub redplus: Option<VipMembership>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VipMembership {
    pub expire_time: u64,
}

impl VipInfo {
    pub fn badge_path(&self, now_ms: u64) -> Option<String> {
        let level = self.red_vip_level;
        let prefix = if self
            .redplus
            .as_ref()
            .is_some_and(|vip| vip.expire_time > now_ms)
        {
            "svip"
        } else if self
            .associator
            .as_ref()
            .is_some_and(|vip| vip.expire_time > now_ms)
        {
            "vip"
        } else if (1..=7).contains(&level) {
            "vip_remain"
        } else {
            return None;
        };
        let badge = if (1..=7).contains(&level) {
            format!("{prefix}{level}")
        } else {
            prefix.into()
        };
        Some(format!("icons/VIP/{badge}.svg"))
    }
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
            let loaded = match result {
                Ok(profile) => profile,
                Err(message) => {
                    eprintln!("{message}");
                    let _ = this.update(cx, |profile, cx| {
                        profile.nickname = "未登录".into();
                        cx.notify();
                    });
                    return;
                }
            };
            let user_id = loaded.user_id;
            let Ok(request) = this.update(cx, |profile, cx| {
                *profile = loaded;
                // 账号就绪立即触发音乐库加载，会员请求不阻塞歌单索引。
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
                    let _ = this.update(cx, |profile, cx| {
                        if profile.user_id == user_id {
                            profile.vip = Some(vip);
                            cx.notify();
                        }
                    });
                }
                Err(message) => eprintln!("{message}"),
            }
        })
        .detach();

        Self {
            nickname: "加载中…".into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{VipInfo, VipMembership};

    #[test]
    fn badge_follows_membership_expiry() {
        let vip = VipInfo {
            red_vip_level: 7,
            associator: Some(VipMembership { expire_time: 2000 }),
            redplus: Some(VipMembership { expire_time: 1000 }),
        };
        assert_eq!(vip.badge_path(999).as_deref(), Some("icons/VIP/svip7.svg"));
        assert_eq!(vip.badge_path(1000).as_deref(), Some("icons/VIP/vip7.svg"));
        assert_eq!(
            vip.badge_path(2000).as_deref(),
            Some("icons/VIP/vip_remain7.svg")
        );
        let free = VipInfo {
            red_vip_level: 0,
            associator: None,
            redplus: None,
        };
        assert!(free.badge_path(0).is_none());
    }
}
