use serde::{Deserialize, Serialize};

use super::user::UserProfile;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: u64,
    pub name: String,
    pub cover_img_url: Option<String>,
    pub creator: UserProfile,
    pub create_time: i64,
    pub play_count: u64,
    pub track_count: usize,
    pub subscribed_count: u64,
    pub comment_count: Option<u64>,
    pub special_type: u32,
    #[serde(default)]
    pub track_ids: Vec<TrackId>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrackId {
    pub id: u64,
}
