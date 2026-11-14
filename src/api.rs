mod comment;
mod heart;
mod message;
mod playlist;
mod recommend;
mod song;

#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Duration};

use gpui::Global;
pub use ncm_api_rs::ApiClient;
use ncm_api_rs::{ApiResponse, NcmError, Query, create_client};
use tokio::runtime::Runtime;

use crate::models::{UserProfile, VipInfo};

/// GPUI 负责界面，Tokio 负责 SDK 的网络请求。客户端和运行时在应用内复用。
pub struct MusicApi {
    pub client: ApiClient,
    pub runtime: Runtime,
    /// 当前登录会话的 Cookie 串。SDK 的客户端不暴露原始 Cookie，写评论要从中
    /// 取 csrf，只能在这里留一份；登录 / 登出时同步更新。
    pub cookie: Option<String>,
    pub session_generation: u64,
    http: Arc<reqwest_client::ReqwestClient>,
    audio_http: reqwest::Client,
}

impl Global for MusicApi {}

impl MusicApi {
    pub fn set_session(&mut self, cookie: Option<String>) {
        self.session_generation = self.session_generation.wrapping_add(1);
        self.client = create_client(cookie.clone());
        self.cookie = cookie;
    }

    pub fn http_client(&self) -> Arc<reqwest_client::ReqwestClient> {
        self.http.clone()
    }

    pub fn audio_http(&self) -> reqwest::Client {
        self.audio_http.clone()
    }

    /// 真实接口测试优先使用 COOKIE，缺省时读取已保存的会话。
    #[cfg(test)]
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let cookie = std::env::var("COOKIE")
            .ok()
            .filter(|cookie| !cookie.trim().is_empty())
            .or_else(|| {
                crate::persistence::Persistence::new()
                    .ok()
                    .and_then(|persistence| persistence.load_session().ok())
                    .flatten()
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "没有 COOKIE 环境变量，也没有已保存的登录会话",
                )
            })?;

        Self::new(Some(cookie))
    }

    pub fn new(cookie: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let audio_http = reqwest::Client::builder()
            .no_proxy()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(20))
            .build()?;
        let http = {
            // GPUI 适配器使用 gpui-pre-reqwest 分支，单独复用图片客户端与 Tokio 运行时。
            // 图片和音频的 HTTP 客户端都不向资源服务器发送账号 COOKIE。
            let _runtime = runtime.enter();
            let image_http = gpui_reqwest::Client::builder()
                .no_proxy()
                .use_rustls_tls()
                .connect_timeout(Duration::from_secs(10))
                .tcp_keepalive(Duration::from_secs(30))
                .pool_idle_timeout(Duration::from_secs(30))
                .http2_keep_alive_interval(Duration::from_secs(15))
                .http2_keep_alive_timeout(Duration::from_secs(10))
                .http2_keep_alive_while_idle(true)
                .build()?;
            Arc::new(reqwest_client::ReqwestClient::from(image_http))
        };
        Ok(Self {
            client: create_client(cookie.clone()),
            cookie,
            session_generation: 0,
            runtime,
            http,
            audio_http,
        })
    }

    pub async fn user_profile(client: ApiClient) -> Result<UserProfile, String> {
        let mut response = Self::request(client.user_account(&Query::new())).await?;
        let profile: UserProfile = serde_json::from_value(response.body["profile"].take())
            .map_err(|_| "账号资料格式无效，COOKIE 可能已失效".to_string())?;
        if profile.user_id == 0 || profile.nickname.trim().is_empty() {
            return Err("账号资料无效，COOKIE 可能已失效".into());
        }
        Ok(profile)
    }

    pub async fn vip_info(client: ApiClient, user_id: u64) -> Result<VipInfo, String> {
        let mut response =
            Self::request(client.vip_info(&Query::new().param("uid", &user_id.to_string())))
                .await?;
        serde_json::from_value(response.body["data"].take())
            .map_err(|error| format!("会员资料格式无效：{error}"))
    }

    async fn request(
        request: impl std::future::Future<Output = Result<ApiResponse, NcmError>>,
    ) -> Result<ApiResponse, String> {
        let response = tokio::time::timeout(Duration::from_secs(20), request)
            .await
            .map_err(|_| "网络请求超时，请稍后重试".to_string())?
            .map_err(Self::request_error_message)?;
        if !(200..300).contains(&response.status) || response.code() != Some(200) {
            let message = response.message().unwrap_or("音乐服务未返回错误说明");
            let code = response
                .code()
                .map(|code| format!("，业务码 {code}"))
                .unwrap_or_default();
            let message = format!("{message}（HTTP {}{code}）", response.status);
            log::warn!("API 请求失败：{message}");
            return Err(message);
        }
        Ok(response)
    }

    fn request_error_message(error: NcmError) -> String {
        // 只记录错误及其原因，不输出 Cookie、请求体或完整响应。
        if let NcmError::Http(error) = error {
            let error = error.without_url();
            let mut detail = error.to_string();
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                detail.push_str(&format!("：{cause}"));
                source = cause.source();
            }
            log::warn!("API 网络请求失败：{detail}");
            return format!("网络请求失败：{detail}");
        }
        log::warn!("API 请求失败：{error}");
        error.to_string()
    }
}
