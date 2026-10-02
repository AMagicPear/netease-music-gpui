mod favorite_music;
mod featured;
mod following;
mod main_content;
mod my_collection;
mod my_podcast;
mod podcast;
mod recent;
mod recommend;
mod roaming;
mod sidebar_page;

use crate::state::user::UserProfile;
use crate::state::{library::MusicLibrary, playback::PlaybackState};
use gpui::*;

pub use main_content::MainContent;

/// 主内容区可切换的页面，侧边栏里的每一个导航项都对应其中一个。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ContentPage {
    Recommend,
    Featured,
    Podcast,
    Roaming,
    Following,
    FavoriteMusic,
    Recent,
    MyPodcast,
    MyCollection,
}

/// 顶部导航分组：推荐、精选、播客、漫游、关注。
const MAIN_PAGES: [ContentPage; 5] = [
    ContentPage::Recommend,
    ContentPage::Featured,
    ContentPage::Podcast,
    ContentPage::Roaming,
    ContentPage::Following,
];

/// 「我的音乐库」分组，展示在分隔线下方。
const LIBRARY_PAGES: [ContentPage; 4] = [
    ContentPage::FavoriteMusic,
    ContentPage::Recent,
    ContentPage::MyPodcast,
    ContentPage::MyCollection,
];

impl ContentPage {
    /// 同时作为元素 id，GPUI 靠它来匹配状态与事件。
    fn id(self) -> &'static str {
        match self {
            Self::Recommend => "recommend",
            Self::Featured => "featured",
            Self::Podcast => "podcast",
            Self::Roaming => "roaming",
            Self::Following => "following",
            Self::FavoriteMusic => "favorite-music",
            Self::Recent => "recent",
            Self::MyPodcast => "my-podcast",
            Self::MyCollection => "my-collection",
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Recommend => "推荐",
            Self::Featured => "精选",
            Self::Podcast => "播客",
            Self::Roaming => "漫游",
            Self::Following => "关注",
            Self::FavoriteMusic => "我喜欢的音乐",
            Self::Recent => "最近播放",
            Self::MyPodcast => "我的播客",
            Self::MyCollection => "我的收藏",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Recommend => "icons/sidebar_home.svg",
            Self::Featured => "icons/sidebar_featured.svg",
            Self::Podcast => "icons/sidebar_podcast.svg",
            Self::Roaming => "icons/sidebar_fm.svg",
            Self::Following => "icons/sidebar_community.svg",
            Self::FavoriteMusic => "icons/sidebar_like.svg",
            Self::Recent => "icons/sidebar_history.svg",
            Self::MyPodcast => "icons/sidebar_my_podcast.svg",
            Self::MyCollection => "icons/sidebar_favourite.svg",
        }
    }

    /// TODO: 如果有通知的话，会在右边显示个小红点
    fn has_notification(self) -> bool {
        matches!(self, Self::Following | Self::MyPodcast)
    }

    /// 遍历所有页面，供 `MainContent` 预创建。
    fn all() -> impl Iterator<Item = ContentPage> {
        MAIN_PAGES.into_iter().chain(LIBRARY_PAGES)
    }

    /// 创建该导航项对应的页面 View。
    ///
    /// 9 个页面是不同的类型，用 `AnyView` 抹平后才能放进同一张表里；
    /// 这样 `MainContent` 可以一直持有它们，切走再切回不会丢 View 自身的状态。
    fn build(
        self,
        user_profile: Entity<UserProfile>,
        library: Entity<MusicLibrary>,
        playback: Entity<PlaybackState>,
        cx: &mut App,
    ) -> AnyView {
        match self {
            Self::Recommend => cx.new(|_| recommend::RecommendPage).into(),
            Self::Featured => cx.new(|_| featured::FeaturedPage).into(),
            Self::Podcast => cx.new(|_| podcast::PodcastPage).into(),
            Self::Roaming => cx.new(|_| roaming::RoamingPage).into(),
            Self::Following => cx.new(|_| following::FollowingPage).into(),
            Self::FavoriteMusic => cx
                .new(|cx| {
                    favorite_music::FavoriteMusicPage::new(user_profile, library, playback, cx)
                })
                .into(),
            Self::Recent => cx.new(|_| recent::RecentPage).into(),
            Self::MyPodcast => cx.new(|_| my_podcast::MyPodcastPage).into(),
            Self::MyCollection => cx.new(|_| my_collection::MyCollectionPage).into(),
        }
    }
}
