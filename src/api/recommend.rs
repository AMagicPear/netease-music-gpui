use ncm_api_rs::Query;

use super::{ApiClient, MusicApi};
use crate::models::{
    FeaturedActivity, HomepageBlock, RecommendEntry, RecommendedPlaylist, parse_activities,
    parse_entries, parse_homepage_blocks, parse_playlists, parse_radar,
};

impl MusicApi {
    pub async fn recommend_entries(client: ApiClient) -> Result<Vec<RecommendEntry>, String> {
        let response = Self::request(client.pc_rcmd_resource_show(&Query::new())).await?;
        Ok(parse_entries(&response.body))
    }

    pub async fn recommended_playlists(
        client: ApiClient,
    ) -> Result<Vec<RecommendedPlaylist>, String> {
        let response = Self::request(client.personal_page_playlist_rcmd(&Query::new())).await?;
        Ok(parse_playlists(&response.body))
    }

    pub async fn radar_playlists(client: ApiClient) -> Result<Vec<RecommendedPlaylist>, String> {
        let response = Self::request(client.customize_block_page(&Query::new())).await?;
        Ok(parse_radar(&response.body))
    }

    pub async fn featured_activities(client: ApiClient) -> Result<Vec<FeaturedActivity>, String> {
        // `banner()` 的默认 `type=0` 就是 `clientType=pc`。
        let response = Self::request(client.banner(&Query::new())).await?;
        Ok(parse_activities(&response.body))
    }

    /// 游客可用，供未登录的推荐页兜底。
    pub async fn homepage_blocks(client: ApiClient) -> Result<Vec<HomepageBlock>, String> {
        let response = Self::request(client.homepage_block_page(&Query::new())).await?;
        Ok(parse_homepage_blocks(&response.body["data"]["blocks"]))
    }
}
