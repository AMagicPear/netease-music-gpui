//! 「正在播放」沉浸页——一个概念，两个部分。
//!
//! 这是覆盖在窗口之上、点封面或歌名滑出的那一整页（封面、黑胶、歌词、评论）。它
//! 不是侧边栏可切换的页面，所以独立成模块，而不放进 `pages`（那里放的是 [`ContentPage`]
//! 对应的导航页）。
//!
//! - [`state::NowPlaying`]：这一页的共享状态，也是整页展开/收起的唯一状态源与节拍源。
//!   由 `main.rs` 创建、注入给播放栏与本页——播放栏的迷你碟、进度条、音量都读它。
//! - [`page::NowPlayingPage`]：这一页的视图本体，以及它的歌词 [`page::lyrics`] 与
//!   评论 [`page::comments`] 子模块。
//!
//! [`ContentPage`]: crate::ui::pages::ContentPage

mod page;
mod state;

pub(in crate::ui) use page::NowPlayingPage;
pub(crate) use state::NowPlaying;
