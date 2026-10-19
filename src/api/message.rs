use ncm_api_rs::Query;

use super::{ApiClient, MusicApi};
use crate::models::{MessageItem, parse_comments, parse_forwards, parse_notices, parse_private};

impl MusicApi {
    pub async fn private_messages(
        client: &ApiClient,
        user_id: u64,
    ) -> Result<(Vec<MessageItem>, u32), String> {
        let mut response =
            Self::request(client.msg_private(&Query::new().param("limit", "30"))).await?;
        let unread = response.body["newMsgCount"].as_u64().unwrap_or(0) as u32;
        let msgs = response.body["msgs"].take();
        Ok((parse_private(&msgs, user_id), unread))
    }

    pub async fn comment_messages(
        client: &ApiClient,
        user_id: u64,
    ) -> Result<Vec<MessageItem>, String> {
        let mut response =
            Self::request(client.msg_comments(&Query::new().param("uid", &user_id.to_string())))
                .await?;
        let comments = response.body["comments"].take();
        Ok(parse_comments(&comments))
    }

    pub async fn mention_messages(client: &ApiClient) -> Result<(Vec<MessageItem>, u32), String> {
        let mut response =
            Self::request(client.msg_forwards(&Query::new().param("limit", "30"))).await?;
        let unread = response.body["newCount"].as_u64().unwrap_or(0) as u32;
        let forwards = response.body["forwards"].take();
        Ok((parse_forwards(&forwards), unread))
    }

    pub async fn notice_messages(client: &ApiClient) -> Result<Vec<MessageItem>, String> {
        let mut response =
            Self::request(client.msg_notices(&Query::new().param("limit", "30"))).await?;
        let notices = response.body["notices"].take();
        Ok(parse_notices(&notices))
    }

    pub async fn clear_message_counts(client: &ApiClient) -> Result<(), String> {
        Self::request(client.msg_count_clear()).await.map(|_| ())
    }
}
