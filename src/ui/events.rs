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

use crate::models::{Playlist, Song};
use gpui::{App, Entity, EventEmitter};

/// 跨视图的统一导航意图：所有「点开某个东西」的消息层意图都走这一条通道，由
/// `MainWindow` 里的一个订阅（路由者）统一处理，不再让每个页面在 `MainWindow` /
/// `MainContent` 各挂一条订阅。以后加「专辑 / 歌手」这类跳转，只要在这里加一个
/// 变体、在路由者里加一个分支即可，不用再动接线。
///
/// 卡片 / 图标只发意图（消息层），导航与详情加载归路由者；详情按 id 现取。
#[derive(Clone)]
pub(crate) enum Navigate {
    /// 打开一个歌单：详情按 id 现取；搜索结果带摘要就先画页头（封面 / 歌单名）。
    Playlist { id: u64, summary: Option<Playlist> },
    /// 打开一首歌的评论页（播放栏 / 歌单行的评论图标）。
    SongComments(Song),
}

/// 导航意图的总线：页面与组件只往这里发意图，不自己持有订阅者。
/// 它是 [`Navigate`] 的事件源，`MainWindow` 订阅一次就接管全部导航。
pub(crate) struct NavigationBus;

impl EventEmitter<Navigate> for NavigationBus {}

/// 把总线实体挂到 App 全局的句柄（`Entity` 本身不实现 `Global`，用这层包一下）。
pub(crate) struct NavigationBusHandle(pub(crate) Entity<NavigationBus>);

impl gpui::Global for NavigationBusHandle {}

impl NavigationBus {
    /// 取全局唯一的总线实体（在 `main.rs` 启动时 `set_global`）。
    pub(crate) fn global(cx: &App) -> Entity<NavigationBus> {
        cx.global::<NavigationBusHandle>().0.clone()
    }

    /// 往总线发一个导航意图（消息层）。任何持有 `&mut App` 的上下文都能调。
    pub(crate) fn emit(cx: &mut App, intent: Navigate) {
        Self::global(cx).update(cx, |_, cx| cx.emit(intent));
    }
}

/// 「歌单页停进 / 离开评论页签」：评论列表要铺满整个内容区，`MainContent` 订阅后重排。
/// 只在翻转时发，切「歌曲 ↔ 收藏者」不打扰布局方；当前是纯信号事件，订阅方
/// 渲染时直接问页面要状态，不需要载荷。
#[derive(Clone, Copy)]
pub(in crate::ui) struct PlaylistCommentsTab;

/// 歌单元数据更新，窗口需要重新读取封面颜色；行 hover 和列表滚动不影响窗口背景。
pub(in crate::ui) struct PlaylistMetadataChanged;

/// 页面导航或侧栏宽度变化，窗口需要重排背景及歌单覆盖层。
pub(in crate::ui) struct ContentLayoutChanged;
