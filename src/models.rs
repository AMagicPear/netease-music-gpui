mod audio_source;
mod comment;
mod homepage;
mod message;
mod play_mode;
mod playlist;
mod recommend;
mod song;
mod song_detail;
mod user;

pub use audio_source::{AudioQualityLevel, AudioSourceInfo};
pub(crate) use comment::parse_page as parse_comment_page;
pub use comment::{CommentResource, SongComment, SongComments};
pub(crate) use homepage::parse_blocks as parse_homepage_blocks;
// 推荐页的新接口解析已经直接走 `homepage` 模块内部，这个导出只给测试夹具用。
#[cfg(test)]
pub use homepage::CreativeTarget;
pub use homepage::{HomepageBlock, PLAYLIST_BLOCK_CODE};
pub use message::MessageItem;
pub(crate) use message::{parse_comments, parse_forwards, parse_notices, parse_private};
pub use play_mode::PlayMode;
pub use playlist::{Playlist, RecommendedPlaylist, TrackId};
pub use recommend::{FeaturedActivity, HeartRecommendation, RecommendEntry};
pub(crate) use recommend::{
    parse_activities, parse_daily_songs, parse_entries, parse_playlists, parse_radar,
    playlists_from_creatives,
};
pub use song::Song;
pub(crate) use song::attach_privileges;
pub use song_detail::LyricLine;
pub use user::{UserProfile, VipInfo};
