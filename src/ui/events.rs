//! 跨视图的 UI 意图和布局通知。
//!
//! GPUI 里有两条互不替代的派发通道，分工固定，别混用：
//! - **action**（`window.dispatch_action` / `on_action`）：*输入*层。键位和原生菜单
//!   （歌曲右键菜单、Dock 菜单）派发的命令，沿焦点路径找处理器，天生带「输入」语义。
//! - **typed event**（`cx.emit` / `cx.subscribe`）：*消息*层。跨视图的导航意图和布局变化
//!   由订阅方（通常是 shell 里的路由者）处理，不依赖焦点。
//!
//! 同一视图内部的点击直接调用方法；修改共享状态则调用对应 Entity 的命令。
//! `observe` 用于读取整个实体的状态，位置推进、渐变等高频变化用领域事件区分，
//! 避免让无关消费者跟着重绘。

use crate::models::Song;

/// 「打开某首歌的评论页」：播放栏、歌单行的评论图标发它，`MainContent` 订阅后导航。
/// 原生歌曲菜单里的同名项仍走 [`super::comments::ViewSongComments`] action（输入层），
/// 两者终点相同、通道各归其位。
#[derive(Clone)]
pub(in crate::ui) struct OpenSongComments(pub Song);

/// 「歌单页停进 / 离开评论页签」：评论列表要铺满整个内容区，`MainContent` 订阅后重排。
/// 只在翻转时发，切「歌曲 ↔ 收藏者」不打扰布局方；当前是纯信号事件，订阅方
/// 渲染时直接问页面要状态，不需要载荷。
#[derive(Clone, Copy)]
pub(in crate::ui) struct PlaylistCommentsTab;

/// 「打开推荐页的一张推荐歌单」：卡片只发 id（消息层），导航和详情加载归
/// `MainContent`——详情按 id 现取，卡片上没有完整摘要可传。
#[derive(Clone, Copy)]
pub(in crate::ui) struct OpenRecommendedPlaylist(pub u64);

/// 歌单元数据更新，窗口需要重新读取封面颜色；行 hover 和列表滚动不影响窗口背景。
pub(in crate::ui) struct PlaylistMetadataChanged;

/// 页面导航或侧栏宽度变化，窗口需要重排背景及歌单覆盖层。
pub(in crate::ui) struct ContentLayoutChanged;
