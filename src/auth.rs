use std::{collections::BTreeMap, time::Duration};

use ncm_api_rs::{ApiClient, ApiResponse, Query};

pub enum QrStatus {
    Expired,
    Waiting,
    Scanned,
    Authorized(String),
}

pub async fn qr_key(client: &ApiClient) -> Result<String, String> {
    let response =
        tokio::time::timeout(Duration::from_secs(20), client.login_qr_key(&Query::new()))
            .await
            .map_err(|_| "获取二维码超时".to_string())?
            .map_err(|_| "获取二维码失败，请检查网络".to_string())?;
    parse_qr_key(response)
}

fn parse_qr_key(response: ApiResponse) -> Result<String, String> {
    if response.body["code"].as_i64() != Some(200) {
        return Err("获取二维码失败，请重试".into());
    }
    response.body["unikey"]
        .as_str()
        .filter(|key| {
            !key.is_empty()
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .map(str::to_owned)
        .ok_or_else(|| "二维码 key 格式无效".into())
}

pub async fn qr_check(client: &ApiClient, key: &str) -> Result<QrStatus, String> {
    let response = tokio::time::timeout(
        Duration::from_secs(20),
        client.login_qr_check(&Query::new().param("key", key)),
    )
    .await
    .map_err(|_| "查询扫码状态超时，请重试".to_string())?
    .map_err(|_| "查询扫码状态失败，请检查网络后重试".to_string())?;
    parse_qr_status(response)
}

/// 手机号登录凭据：验证码或密码（明文，SDK 内部做 MD5）。
pub enum PhoneSecret {
    Captcha(String),
    Password(String),
}

/// 发送手机验证码。
pub async fn send_captcha(client: &ApiClient, phone: &str) -> Result<(), String> {
    let response = tokio::time::timeout(
        Duration::from_secs(20),
        client.captcha_sent(&Query::new().param("phone", phone)),
    )
    .await
    .map_err(|_| "发送验证码超时".to_string())?
    .map_err(|_| "发送验证码失败，请检查网络".to_string())?;
    parse_business_code(&response.body, "发送验证码失败，请重试")
}

/// 手机号登录（验证码或密码），成功返回登录 Cookie。
///
/// 不用 SDK 的 `login_cellphone`：它在验证码模式下仍会发送 `password`
/// 字段（填验证码明文），服务器就会按密码登录分支校验，返回“账号或密码错误”。
/// 这里对齐官方 Web 端参数：验证码模式只发 captcha，不发 password。
/// 手机号登录（验证码或密码），成功返回登录 Cookie。
/// 验证码登录依赖 vendored SDK 修复的参数互斥（见 vendor/ncm-api-rs
/// `login_cellphone`，upstream issue #5）。
pub async fn phone_login(
    client: &ApiClient,
    phone: &str,
    secret: PhoneSecret,
) -> Result<String, String> {
    let mut query = Query::new()
        .param("phone", phone)
        .param("countrycode", "86");
    match secret {
        PhoneSecret::Captcha(code) => query = query.param("captcha", &code),
        PhoneSecret::Password(password) => query = query.param("password", &password),
    }
    let response = tokio::time::timeout(Duration::from_secs(20), client.login_cellphone(&query))
        .await
        .map_err(|_| "登录请求超时".to_string())?
        .map_err(|_| "登录请求失败，请检查网络".to_string())?;
    if response.body["code"].as_i64() != Some(200) {
        return Err(business_message(
            &response.body,
            "手机号登录失败，请检查账号信息",
        ));
    }
    extract_cookie(&response, "登录成功但没有收到账号凭据，请重试")
}

/// 从响应 Cookie 头里提取登录凭据，要求必须含非空 MUSIC_U。
/// 返回「name=value; …」拼接的 Cookie 串，与扫码登录的格式一致。
fn extract_cookie(response: &ApiResponse, missing_credential: &str) -> Result<String, String> {
    let mut cookies = BTreeMap::new();
    for header in &response.cookie {
        if header.contains(['\r', '\n']) {
            return Err("登录 Cookie 格式无效".into());
        }
        let pair = header.split(';').next().unwrap_or_default();
        let (name, value) = pair.split_once('=').ok_or("登录 Cookie 格式无效")?;
        let name = name.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err("登录 Cookie 格式无效".into());
        }
        cookies.insert(name.to_owned(), value.trim().to_owned());
    }
    if cookies.get("MUSIC_U").is_none_or(|value| value.is_empty()) {
        return Err(missing_credential.into());
    }
    Ok(cookies
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; "))
}

fn business_message(body: &serde_json::Value, fallback: &str) -> String {
    body["message"]
        .as_str()
        .filter(|message| !message.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| fallback.into())
}

/// 业务码 200 视为成功，否则取接口返回的错误文案。
fn parse_business_code(body: &serde_json::Value, fallback: &str) -> Result<(), String> {
    if body["code"].as_i64() == Some(200) {
        Ok(())
    } else {
        Err(business_message(body, fallback))
    }
}

fn parse_qr_status(response: ApiResponse) -> Result<QrStatus, String> {
    // SDK 会把 800–803 的 response.status 归一为 200，扫码进度必须读 body.code。
    match response.body["code"].as_i64() {
        Some(800) => Ok(QrStatus::Expired),
        Some(801) => Ok(QrStatus::Waiting),
        Some(802) => Ok(QrStatus::Scanned),
        Some(803) => Ok(QrStatus::Authorized(extract_cookie(
            &response,
            "登录成功但没有收到账号凭据，请刷新二维码",
        )?)),
        _ => Err("扫码状态无效，请刷新二维码".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(code: i64, cookie: Vec<String>) -> ApiResponse {
        ApiResponse {
            status: 200,
            body: json!({"code": code}),
            cookie,
        }
    }

    #[test]
    fn qr_progress_uses_business_code() {
        assert!(matches!(
            parse_qr_status(response(800, vec![])),
            Ok(QrStatus::Expired)
        ));
        assert!(matches!(
            parse_qr_status(response(801, vec![])),
            Ok(QrStatus::Waiting)
        ));
        assert!(matches!(
            parse_qr_status(response(802, vec![])),
            Ok(QrStatus::Scanned)
        ));
        assert!(parse_qr_status(response(500, vec![])).is_err());
    }

    #[test]
    fn phone_login_extracts_music_u_cookie() {
        let ok = ApiResponse {
            status: 200,
            body: json!({"code": 200}),
            cookie: vec!["MUSIC_U=abc".to_owned()],
        };
        assert_eq!(extract_cookie(&ok, "x").unwrap(), "MUSIC_U=abc");
        let no_cookie = ApiResponse {
            status: 200,
            body: json!({"code": 200}),
            cookie: vec![],
        };
        assert!(extract_cookie(&no_cookie, "x").is_err());
    }

    #[test]
    fn phone_login_reports_business_message() {
        let response = ApiResponse {
            status: 200,
            body: json!({"code": 502, "message": "密码错误"}),
            cookie: vec![],
        };
        assert_eq!(business_message(&response.body, "fallback"), "密码错误");
        let silent = ApiResponse {
            status: 200,
            body: json!({"code": 502}),
            cookie: vec![],
        };
        assert_eq!(business_message(&silent.body, "fallback"), "fallback");
    }

    #[test]
    fn qr_key_reads_the_sdk_response_and_rejects_invalid_urls() {
        let key = ApiResponse {
            status: 200,
            body: json!({"code": 200, "unikey": "abc-123_xyz"}),
            cookie: vec![],
        };
        assert_eq!(parse_qr_key(key).unwrap(), "abc-123_xyz");
        for value in ["", "abc&redirect=evil", "abc\n", "测试"] {
            assert!(
                parse_qr_key(ApiResponse {
                    status: 200,
                    body: json!({"code": 200, "unikey": value}),
                    cookie: vec![]
                })
                .is_err()
            );
        }
        assert!(parse_qr_key(response(500, vec![])).is_err());
    }

    #[test]
    fn authorization_keeps_cookie_pairs_and_requires_music_u() {
        let headers = vec![
            "MUSIC_U=token==; Path=/; HttpOnly".into(),
            "__csrf=csrf; Max-Age=10".into(),
        ];
        let QrStatus::Authorized(cookie) = parse_qr_status(response(803, headers)).unwrap() else {
            panic!("expected authorization")
        };
        assert_eq!(cookie, "MUSIC_U=token==; __csrf=csrf");
        assert!(parse_qr_status(response(803, vec![])).is_err());
        assert!(parse_qr_status(response(803, vec!["MUSIC_U=; Path=/".into()])).is_err());
        assert!(parse_qr_status(response(803, vec!["MUSIC_U=a\r\nx=y".into()])).is_err());
    }

    #[test]
    #[ignore = "需要网络连接，只请求未授权二维码，不使用账号凭据"]
    fn qr_endpoint_returns_pending_login() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let client = ncm_api_rs::create_client(None);
            let key = qr_key(&client).await.expect("获取二维码 key 失败");
            assert!(matches!(
                qr_check(&client, &key).await.unwrap(),
                QrStatus::Waiting
            ));
        });
    }
}
