//! 发布评论：写操作，失败原因要能直接说给用户听。
//!
//! 读评论和写评论的加密方式相同（都是 weapi），但写操作必须带登录态：
//! 服务端用 Cookie 里的 `__csrf` 校验 `csrf_token`，没登录必然被拒。

use std::time::Duration;

use ncm_api_rs::{ApiResponse, NcmError, Query};

use super::{ApiClient, MusicApi};
use crate::models::CommentResource;

impl MusicApi {
    /// 发布一条评论。
    ///
    /// 只管成没成：新评论的内容由列表接口给（发布成功后重新拉第一页），
    /// 这里不去解析响应里的评论对象。
    ///
    /// `cookie` 是当前会话的 Cookie 串：SDK 的客户端不暴露原始 Cookie，
    /// 写操作要从中取 csrf，只能由调用方转交。
    pub async fn publish_comment(
        client: ApiClient,
        cookie: Option<String>,
        resource: CommentResource,
        content: &str,
    ) -> Result<(), String> {
        let mut query = Query::new()
            .param("t", "1")
            .param("type", resource.api_type())
            .param("id", &resource.id().to_string())
            .param("content", content);
        if let Some(cookie) = publish_cookie(cookie.as_deref()) {
            query = query.cookie(&cookie);
        }
        publish_request(client.comment(&query)).await?;
        Ok(())
    }
}

/// 发布用的请求包装。超时报自己的话；未登录、频率限制这类服务端消息直接透出，
/// 比统一的「请求失败」有用得多。
async fn publish_request(
    request: impl std::future::Future<Output = Result<ApiResponse, NcmError>>,
) -> Result<ApiResponse, String> {
    let response = tokio::time::timeout(Duration::from_secs(20), request)
        .await
        .map_err(|_| "评论发布超时，请稍后再试".to_string())?
        .map_err(|_| "评论发布失败，请检查网络连接".to_string())?;
    if response.status != 200 {
        return Err(format!(
            "评论发布失败（状态 {}），请稍后再试",
            response.status
        ));
    }
    match response.body["code"].as_i64() {
        Some(200) => Ok(response),
        // 301 = 需要登录。csrf 缺失或过期都会落到这里。
        Some(301) => Err("请先登录后再发布评论".to_string()),
        Some(code) => {
            let message = response.body["msg"]
                .as_str()
                .or_else(|| response.body["message"].as_str())
                .unwrap_or_default()
                .trim();
            if message.is_empty() {
                Err(format!("评论发布失败（代码 {code}）"))
            } else {
                Err(message.to_string())
            }
        }
        None => Err("评论发布结果无效".to_string()),
    }
}

/// 组装发布请求用的 Cookie。
///
/// weapi 的 `csrf_token` 取自 Cookie 里的 `__csrf`，而网易云的 `__csrf` 与
/// `MUSIC_U` 同值。保存下来的会话未必留着 `__csrf`（它有有效期），登录时
/// 也不一定回填，所以统一按 `MUSIC_U` 重写一份，缺 `MUSIC_U` 就是没登录，
/// 交给服务端回「请先登录」。
fn publish_cookie(cookie: Option<&str>) -> Option<String> {
    let cookie = cookie?;
    let pairs: Vec<&str> = cookie
        .split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| !pair.starts_with("__csrf="))
        .collect();
    let music_u = pairs
        .iter()
        .find_map(|pair| pair.strip_prefix("MUSIC_U=").map(str::trim))
        .filter(|value| !value.is_empty())?;
    let mut pairs = pairs
        .iter()
        .map(|pair| (*pair).to_string())
        .collect::<Vec<_>>();
    pairs.push(format!("__csrf={music_u}"));
    Some(pairs.join("; "))
}

#[cfg(test)]
mod tests {
    use super::publish_cookie;

    #[test]
    fn csrf_is_rewritten_from_music_u() {
        // 会话里没有 __csrf：按 MUSIC_U 补一个，发布才不会被判成未登录。
        assert_eq!(
            publish_cookie(Some("MUSIC_U=token==; __remember_me=true")).as_deref(),
            Some("MUSIC_U=token==; __remember_me=true; __csrf=token==")
        );
        // 旧的 __csrf 一律作废：它有有效期，留在会话里反而会让服务端拒绝。
        assert_eq!(
            publish_cookie(Some("MUSIC_U=abc; __csrf=stale")).as_deref(),
            Some("MUSIC_U=abc; __csrf=abc")
        );
        // 没登录就没有 MUSIC_U，交给服务端回「请先登录」。
        assert_eq!(publish_cookie(Some("__remember_me=true")), None);
        assert_eq!(publish_cookie(None), None);
    }
}
