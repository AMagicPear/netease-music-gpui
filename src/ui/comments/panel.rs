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
use crate::ui::events::CommentPublished;

pub(in crate::ui) struct CommentsPanel {
    list: Entity<CommentsView>,
    input: Entity<TextareaState>,
    composer: CommentComposer,
    /// 三个订阅都挂在宿主实体上（gpui 的 Subscription 一丢就退订），所以必须存在这里：
    /// 输入框限长由它通知宿主重绘剩余字数，列表滚动 / 翻页时宿主也要跟着重绘，
    /// 吸顶编辑框才能贴着占位重新定位；发布成功则由它清空草稿。
    _subscriptions: [Subscription; 3],
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
        let edited_list = list.clone();
        let input_subscription = cx.subscribe_in(
            &input,
            window,
            move |_, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    input.update(cx, |input, cx| clamp_to_limit(input, window, cx));
                    // 用户又开始改了，上一次的失败提示就不再相关。
                    edited_list.update(cx, |view, cx| view.clear_publish_error(cx));
                }
                cx.notify();
            },
        );
        let list_subscription = cx.observe(&list, |_, _, cx| cx.notify());
        // 发布成功由列表广播，编辑框据此清空草稿：谁在编辑由谁决定怎么清。
        let publish_input = input.clone();
        let publish_subscription =
            cx.subscribe_in(&list, window, move |_, _, _: &CommentPublished, window, cx| {
                // `set_value` 不发 InputEvent::Change，宿主得自己重绘，
                // 否则剩余字数还停在旧值上。
                publish_input.update(cx, |input, cx| input.set_value("", window, cx));
                cx.notify();
            });
        Self {
            list,
            input,
            composer,
            _subscriptions: [input_subscription, list_subscription, publish_subscription],
        }
    }

    /// 列表实体：宿主要订阅它的事件（比如发布成功后更新页签上的评论数）。
    pub(in crate::ui) fn list(&self) -> &Entity<CommentsView> {
        &self.list
    }

    /// 打开一个资源的评论。
    ///
    /// 每次切到评论页都会拉一次：资源变了就重新加载，没变也刷新第一页，
    /// 让列表回到服务端的最新那份。滚动位置和已加载的分页都留着。
    ///
    /// `total` 是宿主手上已有的评论数：先用它渲染标题，别让标题空等到第一页回来。
    pub(in crate::ui) fn open_resource(
        &self,
        resource: CommentResource,
        total: Option<u64>,
        cx: &mut App,
    ) {
        self.list.update(cx, |view, cx| {
            view.open_resource(resource, total, cx);
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
