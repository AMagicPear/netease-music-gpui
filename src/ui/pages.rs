pub(super) mod daily;
pub(super) mod playlist;
pub(super) mod recommend;
pub(super) mod search;

use gpui::*;

/// 内容区路由；带 ID 的页面共用 View，按路由加载资源。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ContentPage {
    Recommend,
    Featured,
    Podcast,
    Roaming,
    Following,
    FavoriteMusic,
    Playlist(u64),
    /// 每日推荐：顶部入口进来，侧栏没有对应按钮。
    DailySongs,
    /// 歌曲评论页：评论图标进来，侧栏没有对应按钮。
    SongComments(u64),
    Recent,
    MyPodcast,
    MyCollection,
    DownloadManagement,
    MyCloud,
    /// 搜索结果页：只在搜索框提交后出现，侧边栏没有对应按钮。
    Search,
}

/// 顶部导航分组：推荐、精选、播客、漫游、关注。
pub(super) const MAIN_PAGES: [ContentPage; 5] = [
    ContentPage::Recommend,
    ContentPage::Featured,
    ContentPage::Podcast,
    ContentPage::Roaming,
    ContentPage::Following,
];

/// 「我的音乐库」分组，展示在分隔线下方。
pub(super) const LIBRARY_PAGES: [ContentPage; 6] = [
    ContentPage::FavoriteMusic,
    ContentPage::Recent,
    ContentPage::MyPodcast,
    ContentPage::MyCollection,
    ContentPage::DownloadManagement,
    ContentPage::MyCloud,
];

impl ContentPage {
    /// 同时作为元素 id，GPUI 靠它来匹配状态与事件。
    pub(super) fn id(self) -> SharedString {
        let id = match self {
            Self::Recommend => "recommend",
            Self::Featured => "featured",
            Self::Podcast => "podcast",
            Self::Roaming => "roaming",
            Self::Following => "following",
            Self::FavoriteMusic => "favorite-music",
            Self::Playlist(id) => return format!("playlist-{id}").into(),
            Self::DailySongs => "daily-songs",
            Self::SongComments(id) => return format!("song-comments-{id}").into(),
            Self::Recent => "recent",
            Self::MyPodcast => "my-podcast",
            Self::MyCollection => "my-collection",
            Self::DownloadManagement => "download-management",
            Self::MyCloud => "my-cloud",
            Self::Search => "search",
        };
        id.into()
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Recommend => "推荐",
            Self::Featured => "精选",
            Self::Podcast => "播客",
            Self::Roaming => "漫游",
            Self::Following => "关注",
            Self::FavoriteMusic => "我喜欢的音乐",
            Self::Playlist(_) => "歌单",
            Self::DailySongs => "每日推荐",
            Self::SongComments(_) => "歌曲评论",
            Self::Recent => "最近播放",
            Self::MyPodcast => "我的播客",
            Self::MyCollection => "我的收藏",
            Self::DownloadManagement => "下载管理",
            Self::MyCloud => "我的音乐云盘",
            Self::Search => "搜索",
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self {
            Self::Recommend => "icons/sidebar/sidebar_home.svg",
            Self::Featured => "icons/sidebar/sidebar_featured.svg",
            Self::Podcast => "icons/sidebar/sidebar_podcast.svg",
            Self::Roaming => "icons/sidebar/sidebar_fm.svg",
            Self::Following => "icons/sidebar/sidebar_community.svg",
            Self::FavoriteMusic => "icons/sidebar/sidebar_like.svg",
            Self::Playlist(_) => "icons/playlist.svg",
            Self::DailySongs => "icons/sidebar/sidebar_home.svg",
            Self::SongComments(_) => "icons/comment.svg",
            Self::Recent => "icons/sidebar/sidebar_history.svg",
            Self::MyPodcast => "icons/sidebar/sidebar_my_podcast.svg",
            Self::MyCollection => "icons/sidebar/sidebar_favourite.svg",
            Self::DownloadManagement => "icons/sidebar/sidebar_download.svg",
            Self::MyCloud => "icons/sidebar/sidebar_cloud.svg",
            Self::Search => "icons/search.svg",
        }
    }

    /// TODO: 如果有通知的话，会在右边显示个小红点
    pub(super) fn has_notification(self) -> bool {
        matches!(self, Self::Following | Self::MyPodcast)
    }

    pub(super) fn all() -> impl Iterator<Item = ContentPage> {
        MAIN_PAGES.into_iter().chain(LIBRARY_PAGES)
    }

    /// 业务页面由 MainContent 注入依赖，其余页面共用占位 View。
    pub(super) fn build(self, cx: &mut App) -> Option<AnyView> {
        match self {
            Self::Recommend
            | Self::FavoriteMusic
            | Self::Playlist(_)
            | Self::DailySongs
            | Self::SongComments(_)
            | Self::Search => None,
            _ => Some(cx.new(|_| PlaceholderPage(self)).into()),
        }
    }
}

struct PlaceholderPage(ContentPage);

impl Render for PlaceholderPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(self.0.title())
    }
}
