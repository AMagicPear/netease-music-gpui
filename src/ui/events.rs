//! 跨视图导航和布局通知使用 Entity 事件，不依赖焦点；键位与原生菜单使用 Action。

use crate::models::{Playlist, Song};
use gpui::{App, Entity, EventEmitter};

/// MainWindow 统一处理导航，页面与组件只提交意图。
#[derive(Clone)]
pub(crate) enum Navigate {
    /// 摘要用于先画页头，详情仍按 ID 加载。
    Playlist {
        id: u64,
        summary: Option<Playlist>,
    },
    SongComments(Song),
}

pub(crate) struct NavigationBus;

impl EventEmitter<Navigate> for NavigationBus {}

pub(crate) struct NavigationBusHandle(pub(crate) Entity<NavigationBus>);

impl gpui::Global for NavigationBusHandle {}

impl NavigationBus {
    pub(crate) fn global(cx: &App) -> Entity<NavigationBus> {
        cx.global::<NavigationBusHandle>().0.clone()
    }

    pub(crate) fn emit(cx: &mut App, intent: Navigate) {
        Self::global(cx).update(cx, |_, cx| cx.emit(intent));
    }
}

/// 进入或离开歌单评论页签时重排内容区。
#[derive(Clone, Copy)]
pub(in crate::ui) struct PlaylistCommentsTab;

/// 歌单元数据更新，窗口需要重新读取封面颜色；行 hover 和列表滚动不影响窗口背景。
pub(in crate::ui) struct PlaylistMetadataChanged;

/// 页面导航或侧栏宽度变化，窗口需要重排背景及歌单覆盖层。
pub(in crate::ui) struct ContentLayoutChanged;
