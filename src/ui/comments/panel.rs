//! 评论页的组合件：虚拟列表 + 吸顶编辑框 + 输入限长。
//!
//! 歌曲评论页与歌单评论页都只有「列表 + 编辑框」这一件事，差别是列表首个条目
//! （各自的页头）。宿主页面把页头渲染出来交给这里，剩下的 wiring —— 列表实体的
//! 创建、输入框的限长订阅、翻页时要重新定位吸顶框的通知、整棵树怎么叠 —— 两个页面
//! 完全一致，收在一处不必各写一遍。

use gpui::prelude::{ParentElement, Styled};
use gpui::{
    AnyElement, App, AppContext, Context, Div, Entity, IntoElement, Subscription, Window, div,
};
use gpui_kit::base::input::{InputEvent, TextareaState};

use super::{CommentComposer, CommentResource, CommentsView, ListMode, clamp_to_limit, new_input};
use crate::ui::cover_color::Backdrop;

pub(in crate::ui) struct CommentsPanel {
    list: Entity<CommentsView>,
    input: Entity<TextareaState>,
    composer: CommentComposer,
    /// 两个订阅都挂在宿主实体上（gpui 的 Subscription 一丢就退订），所以必须存在这里：
    /// 输入框限长由它通知宿主重绘剩余字数，列表滚动 / 翻页时宿主也要跟着重绘，
    /// 吸顶编辑框才能贴着占位重新定位。
    _subscriptions: [Subscription; 2],
}

impl CommentsPanel {
    /// `header` 是列表的首个条目，由宿主渲染自己的页头；`backdrop` 是页面的底色，
    /// 吸顶框要按它补画同一份渐变才无缝。
    pub(in crate::ui) fn new<V>(
        window: &mut Window,
        cx: &mut Context<V>,
        backdrop: Option<Backdrop>,
        header: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self
    where
        V: 'static,
    {
        // 独立评论页统一使用主题颜色和 40px 留白；渐变只供吸顶编辑框补画背景。
        let list = cx.new(|_| CommentsView::new(None));
        list.update(cx, |view, cx| view.set_header(header, cx));
        let input = new_input(window, cx);
        let composer = CommentComposer::new(input.clone(), list.clone(), backdrop);
        let input_subscription = cx.subscribe_in(
            &input,
            window,
            |_, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    input.update(cx, |input, cx| clamp_to_limit(input, window, cx));
                }
                cx.notify();
            },
        );
        let list_subscription = cx.observe(&list, |_, _, cx| cx.notify());
        Self {
            list,
            input,
            composer,
            _subscriptions: [input_subscription, list_subscription],
        }
    }

    /// 打开一个资源的评论。同一资源重复打开不会重载，切回评论页时保留评论与滚动位置。
    /// `total` 是宿主手上已有的评论数：先用它渲染标题，别让标题空等到第一页回来。
    pub(in crate::ui) fn open_resource(
        &self,
        resource: CommentResource,
        total: Option<u64>,
        cx: &mut App,
    ) {
        self.list.update(cx, |view, cx| {
            view.set_resource(Some(resource), cx);
            view.set_estimated_total(total, cx);
            view.configure(ListMode::Owned, cx);
        });
    }

    /// 资源没了（换歌单、页面不再展示评论）：取消在途请求并清空列表。
    pub(in crate::ui) fn close_resource(&self, cx: &mut App) {
        self.list.update(cx, |view, cx| view.set_resource(None, cx));
    }

    /// 让列表重新量首个条目：歌单详情回来后页头会变高，虚拟列表不会自己发现。
    pub(in crate::ui) fn remeasure_header(&self, cx: &mut App) {
        self.list.update(cx, |view, cx| {
            view.scroll_state().remeasure_items(0..1);
            cx.notify();
        });
    }

    /// 换资源即换语境：上一处的草稿不该落到新资源的发布框里，
    /// 占位位置记录一起丢掉，新页头量出来前不画到旧位置。
    pub(in crate::ui) fn clear_draft(&self, window: &mut Window, cx: &mut App) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.composer.reset();
    }

    /// 编辑框的等高占位：放进宿主的页头条目参与列表布局。
    pub(in crate::ui) fn placeholder(&self) -> impl IntoElement {
        self.composer.placeholder()
    }

    /// 整页的样子：列表铺满，吸顶编辑框压在上面。
    pub(in crate::ui) fn render(&self) -> Div {
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .child(self.list.clone())
            .child(self.composer.overlay())
    }
}
