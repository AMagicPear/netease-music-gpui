//! 系统媒体控件只接受本地封面。
//!
//! 原生控件直接读远程 URL 时，加载失败会拿到空图（macOS 上甚至是不可恢复的崩溃）。
//! 所以远程封面先由应用下载、按魔数校验、落到缓存目录，再以 `file://` 交给原生控件：
//! 控件读到的永远是我们自己确认过的本地文件。

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use sha2::{Digest, Sha256};

/// 落盘后缀，也用于复用上一次运行留下的文件（文件名只有 URL 的哈希）。
const EXTENSIONS: [&str; 6] = ["png", "jpg", "gif", "webp", "bmp", "tiff"];

pub struct CoverArtCache {
    root: PathBuf,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// 已下载的封面；`None` 表示确认不可用，不再重复下载。
    finished: HashMap<String, Option<PathBuf>>,
    downloading: HashSet<String>,
}

impl CoverArtCache {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            state: Mutex::new(State::default()),
        }
    }

    /// 已经落盘的本地封面；缓存目录被系统清理时返回 None，下次重新下载。
    pub fn local_path(&self, url: &str) -> Option<PathBuf> {
        let mut state = self.state.lock().unwrap();
        if let Some(finished) = state.finished.get(url).cloned() {
            return match finished {
                Some(path) if path.is_file() => Some(path),
                // 文件被清掉了，忘掉这条记录，之后可以重新下载。
                Some(_) => {
                    state.finished.remove(url);
                    None
                }
                None => None,
            };
        }
        let name = cache_name(url);
        let existing = EXTENSIONS
            .iter()
            .map(|extension| self.root.join(format!("{name}.{extension}")))
            .find(|path| path.is_file());
        if let Some(path) = existing.as_ref() {
            state.finished.insert(url.to_owned(), Some(path.clone()));
        }
        existing
    }

    /// 是否该由调用方发起下载：已就绪、已确认失败或正在下载都返回 false。
    pub fn claim(&self, url: &str) -> bool {
        if self.local_path(url).is_some() {
            return false;
        }
        let mut state = self.state.lock().unwrap();
        if state.finished.contains_key(url) {
            return false;
        }
        state.downloading.insert(url.to_owned())
    }

    /// 下载并落盘；失败也记入缓存，避免同一首歌反复重试。
    pub async fn download(&self, url: String, http: &reqwest::Client) -> Option<PathBuf> {
        let path = fetch(&self.root, &url, http).await;
        let mut state = self.state.lock().unwrap();
        state.downloading.remove(&url);
        state.finished.insert(url, path.clone());
        path
    }

    /// 直接写入已知字节；测试用它绕开网络。
    #[cfg(test)]
    fn store(&self, url: &str, bytes: &[u8]) -> Option<PathBuf> {
        let path = write_cover(&self.root, url, bytes);
        self.state
            .lock()
            .unwrap()
            .finished
            .insert(url.to_owned(), path.clone());
        path
    }
}

async fn fetch(root: &Path, url: &str, http: &reqwest::Client) -> Option<PathBuf> {
    let bytes = http
        .get(url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .await
        .ok()?;
    // 落盘是阻塞 I/O，交给专用线程池，不占用 tokio 工作线程。
    let root = root.to_owned();
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || write_cover(&root, &url, &bytes))
        .await
        .ok()?
}

/// 只接受真实图片字节：CDN 出错时常返回 200 + 一段 HTML，交给原生控件就是空图。
fn write_cover(root: &Path, url: &str, bytes: &[u8]) -> Option<PathBuf> {
    let extension = image_extension(bytes)?;
    fs::create_dir_all(root).ok()?;
    let name = cache_name(url);
    let destination = root.join(format!("{name}.{extension}"));
    let mut temporary = tempfile::Builder::new()
        .prefix(&name)
        .suffix(".part")
        .tempfile_in(root)
        .ok()?;
    temporary.write_all(bytes).ok()?;
    temporary.as_file().sync_all().ok()?;
    temporary.persist(&destination).ok()?;
    Some(destination)
}

/// 按魔数判断格式并返回后缀；只看头部，不解码整张图。
fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12).is_some_and(|tag| tag == b"WEBP") {
        Some("webp")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"II\x2a\0") || bytes.starts_with(b"MM\0\x2a") {
        Some("tiff")
    } else {
        None
    }
}

fn cache_name(url: &str) -> String {
    format!("{:x}", Sha256::digest(url.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 PNG 头，足以通过魔数校验。
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n', 0, 0];

    fn temporary_cache(name: &str) -> CoverArtCache {
        let root =
            std::env::temp_dir().join(format!("netease-cover-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        CoverArtCache::new(root)
    }

    #[test]
    fn only_real_image_bytes_reach_the_native_controls() {
        let cache = temporary_cache("invalid");
        let url = "https://p1.music.126.net/cover.jpg?param=512y512";
        assert_eq!(cache.store(url, b"<html>404</html>"), None);
        assert_eq!(cache.local_path(url), None);
        // 已确认失败，不重复下载。
        assert!(!cache.claim(url));
        // 无效字节连缓存目录都不该创建。
        assert_eq!(
            fs::read_dir(&cache.root).map_or(0, |entries| entries.count()),
            0
        );
    }

    #[test]
    fn downloaded_cover_is_reused_with_redownloading() {
        let cache = temporary_cache("valid");
        let url = "https://p1.music.126.net/cover.jpg";
        let path = cache.store(url, PNG).expect("图片应落盘");
        assert!(path.is_file());
        assert_eq!(fs::read(&path).unwrap(), PNG);
        assert_eq!(cache.local_path(url), Some(path.clone()));
        assert!(!cache.claim(url));
        // 新实例只看得到磁盘，等价于重启后复用。
        let restarted = CoverArtCache::new(cache.root.clone());
        assert_eq!(restarted.local_path(url), Some(path));
        assert!(!restarted.claim(url));
        fs::remove_dir_all(&cache.root).unwrap();
    }

    #[test]
    fn cleared_cache_file_triggers_a_new_download() {
        let cache = temporary_cache("cleared");
        let url = "https://p1.music.126.net/cover.jpg";
        let path = cache.store(url, PNG).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(cache.local_path(url), None);
        assert!(cache.claim(url));
        fs::remove_dir_all(&cache.root).unwrap();
    }

    #[test]
    fn cache_name_only_depends_on_the_address() {
        assert_eq!(cache_name("https://a/b.jpg"), cache_name("https://a/b.jpg"));
        assert_ne!(cache_name("https://a/b.jpg"), cache_name("https://a/b.png"));
    }
}
