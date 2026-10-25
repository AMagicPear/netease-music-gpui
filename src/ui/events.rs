//! 跨视图导航和布局通知使用 Entity 事件，不依赖焦点；键位与原生菜单使用 Action。

use crate::models::{Playlist, Song};
use gpui::{App, Entity, EventEmitter};

/// MainWindow 统一处理导航，页面与组件只提交意图。
#[derive(Clone)]
pub(crate) enum Navigate {
    /// 摘要用于先画页头，详情仍按 ID 加载。
    Playlist { id: u64, summary: Option<Playlist> },
    /// 看这首歌的评论：唱片页展开时只翻到评论屏，否则打开歌曲评论页。
    SongComments(Song),
    /// 每日推荐不是歌单，单独一条路由；页面的数据由 DailySongsStore 自己加载。
    DailySongs,
    /// 开始心动模式：换一条新的推荐队列，从头播起。
    ///
    /// 和 [`Self::DailySongs`] 一样是「入口卡片触发的动作」而不是翻页，
    /// 首页卡片和侧栏图标共用这一条。
    HeartMode,
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

/// 评论发布成功，列表正在刷新；编辑框据此清空草稿。
#[derive(Clone, Copy)]
pub(in crate::ui) struct CommentPublished;

pub(in crate::ui) struct CommentCountChanged {
    pub resource: crate::models::CommentResource,
    pub total: u64,
}

/// 歌单元数据更新，窗口需要重新读取封面颜色；行 hover 和列表滚动不影响窗口背景。
pub(in crate::ui) struct PlaylistMetadataChanged;

/// 页面导航或侧栏宽度变化，窗口需要重排背景及歌单覆盖层。
pub(in crate::ui) struct ContentLayoutChanged;
