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
}
