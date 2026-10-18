//! 评论：从接口分页到页面的全部行为都在这一个模块里。
//!
//! 评论有几件事是绑在一起的——「哪些算热评」由接口合并的第一页决定，列表据此分出
//! 「精选评论」小节；行高随正文换行变化，所以列表必须自己管虚拟化与行高实测；
//! 吸顶编辑框要在列表外定位，却又占着列表首个条目里的一块等高占位。它们分到不同
//! 目录后改动总要跨模块来回传话，因此按功能收在这里：
//!
//! - `list`：变高虚拟列表 [`CommentsView`] —— 分页、去重、重试、滚动与滚动条；
//!   正在播放页的下半屏、歌曲评论页、歌单评论页共用同一个实体。
//! - `row`：单条评论的行渲染 [`comment_row`]。
//! - `composer`：吸顶评论框 [`CommentComposer`]，占位与覆盖层都在它内部。
//! - `panel`：列表 + 评论框的组合件 [`CommentsPanel`]，两个评论页各自持有一份。
//! - `page`：歌曲评论页 [`SongCommentsPage`] 与进入它的 action [`ViewSongComments`]。
//!
//! 歌单评论页没有在这里：它是歌单页的一个页签，页头要用歌单详情，交给
//! `pages::playlist` 自己渲染，只从这里取 [`CommentsPanel`]。

mod composer;
mod list;
mod page;
mod panel;
mod row;

pub(in crate::ui) use composer::{CommentComposer, clamp_to_limit, new_input};
pub(in crate::ui) use list::{CommentResource, CommentsView, ListMode};
pub(in crate::ui) use page::{SongCommentsPage, ViewSongComments};
pub(in crate::ui) use panel::CommentsPanel;
pub(in crate::ui) use row::comment_row;
