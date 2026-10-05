use std::{fs, path::PathBuf};

use gpui::{AssetSource, Result, SharedString};

pub struct Assets {
    pub base: PathBuf,
}

/// 网易封面 CDN 在服务端缩放，避免下载原图后只在 GPUI 中缩小显示。
pub fn thumbnail_url(url: &str, pixels: u32) -> String {
    let Ok(mut parsed) = gpui::http_client::Url::parse(url) else {
        return url.into();
    };
    if !parsed
        .host_str()
        .is_some_and(|host| host == "music.126.net" || host.ends_with(".music.126.net"))
    {
        return url.into();
    }
    // 同一张封面在 `user/playlist`（歌单摘要，返回 http://）和 `playlist/detail`
    // （歌单详情，返回 https://）里的协议不一致，而 GPUI 的资源缓存以整条 URL 作为
    // key（`Resource::Uri` / `SharedUri` 按内容哈希），协议不同就会被当成两张不同的图。
    // 结果：详情返回时这张封面命中不了摘要那份缓存，要重新下载并解码，画面表现为封面闪一下。
    // 统一升级到 https，让两边命中同一份缓存；网易 CDN 的 https 与 http 是同一批节点。
    if parsed.scheme() == "http" {
        // 只有「特殊协议 ↔ 非特殊协议」互转才会失败，http → https 不会。
        let _ = parsed.set_scheme("https");
    }
    let parameters: Vec<_> = parsed
        .query_pairs()
        .filter(|(key, _)| key != "param")
        .map(|pair| (pair.0.into_owned(), pair.1.into_owned()))
        .collect();
    parsed
        .query_pairs_mut()
        .clear()
        .extend_pairs(parameters)
        .append_pair("param", &format!("{pixels}y{pixels}"));
    parsed.into()
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        fs::read(self.base.join(path))
            .map(|data| Some(std::borrow::Cow::Owned(data)))
            .map_err(Into::into)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        fs::read_dir(self.base.join(path))
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        entry
                            .ok()
                            .and_then(|entry| entry.file_name().into_string().ok())
                            .map(SharedString::from)
                    })
                    .collect()
            })
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::thumbnail_url;

    #[test]
    fn thumbnail_replaces_size_and_preserves_other_parameters() {
        let url = thumbnail_url(
            "https://p1.music.126.net/cover.jpg?token=abc&param=1000y1000",
            72,
        );
        assert_eq!(
            url,
            "https://p1.music.126.net/cover.jpg?token=abc&param=72y72"
        );
        assert_eq!(thumbnail_url(&url, 72), url);
        assert_eq!(
            thumbnail_url("https://example.com/cover.jpg", 72),
            "https://example.com/cover.jpg"
        );
    }

    /// 摘要走 `user/playlist`（http），详情走 `playlist/detail`（https），归一化后必须
    /// 得到同一个字符串，否则 GPUI 的资源缓存会把它当成两张图，封面在详情返回时闪一下。
    #[test]
    fn thumbnail_normalizes_scheme_so_summary_and_detail_share_one_cache_entry() {
        let summary = thumbnail_url("http://p1.music.126.net/abc==/109951174028075973.jpg", 340);
        let detail = thumbnail_url(
            "https://p1.music.126.net/abc==/109951174028075973.jpg?param=200y200",
            340,
        );
        assert_eq!(summary, detail);
        assert_eq!(
            summary,
            "https://p1.music.126.net/abc==/109951174028075973.jpg?param=340y340"
        );
    }
}
