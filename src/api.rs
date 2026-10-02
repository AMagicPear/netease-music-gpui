use std::{io, time::Duration};

use gpui::Global;
use ncm_api_rs::{ApiClient, ApiResponse, NcmError, Query, create_client};
use tokio::runtime::Runtime;

use crate::state::user::UserProfile;
use crate::state::{playlist::Playlist, song::Song};

/// GPUI 负责界面，Tokio 负责 SDK 的网络请求。客户端和运行时在应用内复用。
pub struct MusicApi {
    pub client: ApiClient,
    pub runtime: Runtime,
}

impl Global for MusicApi {}

impl MusicApi {
    pub fn http_client(&self) -> std::sync::Arc<reqwest_client::ReqwestClient> {
        // 让 GPUI 的图片请求复用现有 Tokio 运行时；不携带账号 COOKIE。
        let _runtime = self.runtime.enter();
        std::sync::Arc::new(reqwest_client::ReqwestClient::new())
    }

    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let cookie = std::env::var("COOKIE")?;
        if cookie.trim().is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "COOKIE 环境变量为空").into());
        }

        Ok(Self {
            client: create_client(Some(cookie)),
            runtime: Runtime::new()?,
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

    async fn request(
        request: impl std::future::Future<Output = Result<ApiResponse, NcmError>>,
    ) -> Result<ApiResponse, String> {
        let response = tokio::time::timeout(Duration::from_secs(20), request)
            .await
            .map_err(|_| "API 请求超时".to_string())?
            .map_err(|_| "API 请求失败，请检查网络及 COOKIE".to_string())?;
        if response.status != 200 || response.body["code"].as_i64() != Some(200) {
            return Err(format!(
                "API 请求失败（状态 {}），请检查 COOKIE",
                response.status
            ));
        }
        Ok(response)
    }

    pub async fn library(
        client: ApiClient,
        user_id: u64,
    ) -> Result<(Vec<Playlist>, Vec<Song>, Vec<u64>), String> {
        let mut playlists = Vec::new();
        loop {
            let mut response = Self::request(
                client.user_playlist(
                    &Query::new()
                        .param("uid", &user_id.to_string())
                        .param("limit", "100")
                        .param("offset", &playlists.len().to_string()),
                ),
            )
            .await?;
            let page: Vec<Playlist> = serde_json::from_value(response.body["playlist"].take())
                .map_err(|error| format!("歌单列表格式无效：{error}"))?;
            let more = response.body["more"].as_bool() == Some(true);
            let empty = page.is_empty();
            playlists.extend(page);
            if !more || empty {
                break;
            }
        }

        let like_query = Query::new().param("uid", &user_id.to_string());
        let mut likes = Self::request(client.likelist(&like_query)).await?;
        let liked_song_ids = serde_json::from_value(likes.body["ids"].take())
            .map_err(|_| "喜欢的歌曲列表格式无效".to_string())?;
        let mut songs = Vec::new();
        if let Some(index) = playlists
            .iter()
            .position(|playlist| playlist.special_type == 5)
        {
            let mut response = Self::request(
                client.playlist_detail(&Query::new().param("id", &playlists[index].id.to_string())),
            )
            .await?;
            let playlist: Playlist = serde_json::from_value(response.body["playlist"].take())
                .map_err(|error| format!("歌单详情格式无效：{error}"))?;
            // 分批获取所有 trackIds，避免 SDK playlist_track_all 默认只取 1000 首。
            for tracks in playlist.track_ids.chunks(200) {
                let ids = tracks
                    .iter()
                    .map(|track| track.id.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let mut response =
                    Self::request(client.song_detail(&Query::new().param("ids", &ids))).await?;
                let batch: Vec<Song> = serde_json::from_value(response.body["songs"].take())
                    .map_err(|error| format!("歌曲详情格式无效：{error}"))?;
                songs.extend(batch);
            }
            playlists[index] = playlist;
        }
        Ok((playlists, songs, liked_song_ids))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::AsyncReadExt;
    use gpui::http_client::HttpClient;

    #[test]
    #[ignore = "需要 COOKIE 环境变量和网络连接"]
    fn cookie_can_load_real_library() {
        let api = MusicApi::from_env().expect("无法读取 COOKIE 环境变量");
        let profile = api
            .runtime
            .block_on(MusicApi::user_profile(api.client.clone()))
            .expect("无法获取登录账号");
        assert!(profile.user_id > 0);
        assert!(!profile.nickname.is_empty());
        let (playlists, songs, likes) = api
            .runtime
            .block_on(MusicApi::library(api.client.clone(), profile.user_id))
            .expect("无法加载真实音乐库");
        let favorite = playlists
            .iter()
            .find(|playlist| playlist.special_type == 5)
            .expect("缺少喜欢的音乐歌单");
        assert_eq!(songs.len(), favorite.track_ids.len());
        assert!(
            songs
                .iter()
                .zip(&favorite.track_ids)
                .all(|(song, track)| song.id == track.id)
        );
        assert!(songs.iter().all(|song| likes.contains(&song.id)));
        // 使用与应用相同的客户端验证真实头像、歌单封面和歌曲封面可下载。
        let http = api.http_client();
        let cover = songs
            .iter()
            .find_map(|song| song.al.pic_url.as_deref())
            .expect("缺少歌曲封面");
        for url in [
            &profile.avatar_url,
            favorite.cover_img_url.as_deref().expect("缺少歌单封面"),
            cover,
        ] {
            api.runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(20), async {
                    let mut response = http.get(url, ().into(), true).await.expect("图片下载失败");
                    assert!(response.status().is_success());
                    assert!(
                        response.headers()["content-type"]
                            .to_str()
                            .unwrap()
                            .starts_with("image/")
                    );
                    let mut bytes = Vec::new();
                    response
                        .body_mut()
                        .read_to_end(&mut bytes)
                        .await
                        .expect("图片读取失败");
                    assert!(!bytes.is_empty());
                })
                .await
                .expect("图片下载超时");
            });
        }
    }
}
