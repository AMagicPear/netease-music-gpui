//! 「正在播放」沉浸页——一个概念，两个部分。
//!
//! 这是覆盖在窗口之上、点封面或歌名滑出的那一整页（封面、黑胶、歌词、评论）。它
//! 不是侧边栏可切换的页面，所以独立成模块，而不放进 `pages`（那里放的是 [`ContentPage`]
//! 对应的导航页）。
//!
//! - [`state::NowPlaying`]：这一页的共享状态，也是整页展开/收起的唯一状态源与节拍源。
//!   由 `main.rs` 创建、注入给播放栏与本页——播放栏的迷你碟、进度条、音量都读它。
//! - [`page::NowPlayingPage`]：这一页的视图本体，歌词子视图 [`page::lyrics`]，以及
//!   第二屏嵌入的 [`comments::CommentsView`]（模块内的列表行为见它自己的文档）。
//!
//! [`ContentPage`]: crate::ui::pages::ContentPage

mod page;
mod state;

pub(in crate::ui) use page::NowPlayingPage;
pub(crate) use state::NowPlaying;
pub(in crate::ui) use state::NowPlayingEvent;
