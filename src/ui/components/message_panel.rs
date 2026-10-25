//! 消息侧面板；业务数据由 MessagesStore 管理。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{ColorTokens, Scrollbar, ScrollbarMode, Theme};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::avatar::Avatar;

use super::play_queue::{PANEL_HEADER_HEIGHT, header_text_button};
use super::spinner;
use super::tab_bar::{TabBar, TabChanged, TabItem};
use crate::models::MessageItem;
use crate::state::account::AccountState;
use crate::state::messages::{MessagesStore, TAB_COUNT, TAB_MENTIONS, TAB_PRIVATE};
use crate::state::resource::{DataState, Resource};
use crate::ui::assets::thumbnail_url;
use crate::ui::theme::DOLPHIN_FAMILY;

/// 每条消息占的高度：两行文字（昵称+时间 / 正文）比播放列表行更高。
const ROW_HEIGHT: Pixels = px(72.);
/// 行内头像尺寸；圆形由 gpui-kit 的 [`Avatar`] 自带。
const AVATAR_SIZE: Pixels = px(40.);
/// 头部与每一行的左右留白（与播放列表内容一致）。
const ROW_PADDING: Pixels = px(20.);
/// 昵称与正文之间的行距。
const LINE_GAP: Pixels = px(3.);

pub struct MessagePanel {
    store: Entity<MessagesStore>,
    tabs: Entity<TabBar>,
    /// 当前展示的 tab；与 `tabs` 的选中项保持一致。
    active_tab: usize,
    _data_subscription: Subscription,
    scroll: ScrollHandle,
    visible: bool,
    _tabs_subscription: Subscription,
}

impl MessagePanel {
    pub fn new(account: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let tabs = cx.new(|_| {
            TabBar::new(vec![
                TabItem::new("私信"),
                TabItem::new("评论"),
                TabItem::new("@我"),
                TabItem::new("通知"),
            ])
        });
        let _tabs_subscription = cx.subscribe(&tabs, |this, tabs, _: &TabChanged, cx| {
            let index = tabs.read(cx).selected_index();
            this.select_tab(index, cx);
        });
        let store = cx.new(|cx| MessagesStore::new(account, cx));
        let data_subscription = cx.observe(&store, |this, _, cx| {
            for tab in 0..TAB_COUNT {
                this.sync_tab_badge(tab, cx);
            }
            cx.notify();
        });
        Self {
            store,
            tabs,
            active_tab: TAB_PRIVATE,
            _data_subscription: data_subscription,
            scroll: ScrollHandle::default(),
            visible: false,
            _tabs_subscription,
        }
    }

    /// 面板可见性（由 side_panel 维护）。每次展开强制刷新当前 tab，保证未读数新鲜。
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.store.update(cx, |store, cx| {
            store.show(visible.then_some(self.active_tab), cx)
        });
        cx.notify();
    }

    fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.active_tab == index {
            return;
        }
        self.active_tab = index;
        // 换分类回到顶部，并在后台更新该分类。
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.store
            .update(cx, |store, cx| store.show(Some(index), cx));
        cx.notify();
    }

    /// tab 上的未读角标：只有接口给出账号级未读数的两个 tab 才显示。
    fn sync_tab_badge(&mut self, tab: usize, cx: &mut Context<Self>) {
        let unread = self.store.read(cx).tabs[tab]
            .value()
            .map(|(_, unread)| *unread)
            .unwrap_or(0);
        let count = if matches!(tab, TAB_PRIVATE | TAB_MENTIONS) && unread > 0 {
            Some(unread.to_string())
        } else {
            None
        };
        self.tabs
            .update(cx, |tabs, cx| tabs.set_count(tab, count, cx));
    }

    fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.mark_all_read(cx));
    }
}

impl Render for MessagePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let data = &self.store.read(cx).tabs[self.active_tab];
        let list_state = message_list_state(data, colors);

        // 内容填满 side_panel 的外壳；这里只管排版，不管定位与动画。
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(PANEL_HEADER_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(ROW_PADDING)
                    .pr(ROW_PADDING)
                    .child(
                        // 58px 头部里指示条挂在文字下方，居中会让文字视觉重心偏上；
                        // 下压让它和播放列表那行大标题的视觉基线齐平。
                        div().mt(px(6.)).child(self.tabs.clone()),
                    )
                    .child(div().flex_1())
                    .child(header_text_button(
                        "message-panel-read-all",
                        "icons/eye.svg",
                        "一键全读",
                        colors,
                        cx.listener(|this, _, _, cx| this.mark_all_read(cx)),
                    )),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("message-panel-list")
                            .flex_1()
                            .min_h(px(0.))
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .children(list_state),
                    )
                    .child(Scrollbar::vertical(&self.scroll).mode(ScrollbarMode::Always)),
            )
            .into_any_element()
    }
}

/// 列表内容：加载中 / 出错 / 空态各给一个居中提示，有数据就铺消息行。
fn message_list_state(
    data: &Resource<(Vec<MessageItem>, u32)>,
    colors: ColorTokens,
) -> Vec<AnyElement> {
    let centered = |content: AnyElement| -> AnyElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(content)
            .into_any_element()
    };
    let items = data
        .value()
        .map(|(items, _)| items.as_slice())
        .unwrap_or(&[]);
    let mut rows: Vec<AnyElement> = Vec::new();
    match data.state() {
        DataState::Idle | DataState::Loading => {
            return vec![centered(
                spinner("message-panel-loading", 20., colors.muted_foreground).into_any_element(),
            )];
        }
        DataState::Failed => {
            return vec![centered(
                div()
                    .text_size(px(13.))
                    .text_color(colors.muted_foreground)
                    .child("消息加载失败，重新打开可重试")
                    .into_any_element(),
            )];
        }
        DataState::Ready(_) => {}
    }
    if items.is_empty() {
        return vec![centered(
            div()
                .text_size(px(13.))
                .text_color(colors.muted_foreground)
                .child("暂无消息")
                .into_any_element(),
        )];
    }
    rows.extend(items.iter().map(|item| message_row(item, colors)));
    rows
}

/// 一条消息行：圆形头像（未读时右上角红色计数徽标）+ 昵称/时间 + 正文/来源。
fn message_row(item: &MessageItem, colors: ColorTokens) -> AnyElement {
    let avatar_src: SharedString = if item.avatar_url.is_empty() {
        "images/default_avatar.png".into()
    } else {
        thumbnail_url(&item.avatar_url, 80).into()
    };
    div()
        .w_full()
        .h(ROW_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(ROW_PADDING)
        // 行高是固定的，内容一旦多出高度就会压到下一行；这里裁掉溢出，
        // 保证「最多两行文字」这条约定在极端数据下也成立。
        .overflow_hidden()
        .child(
            div()
                .relative()
                .flex_none()
                .child(
                    Avatar::new()
                        .with_size(AVATAR_SIZE)
                        .border_0()
                        .src(avatar_src),
                )
                .when(item.unread > 0, |wrap| {
                    wrap.child(
                        div()
                            .absolute()
                            .top(px(-3.))
                            .right(px(-3.))
                            .min_w(px(16.))
                            .h(px(16.))
                            .px(px(4.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(colors.primary)
                            .text_size(px(10.))
                            .text_color(colors.primary_foreground)
                            .child(item.unread.to_string()),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(LINE_GAP)
                // 第一行：昵称靠左，时间靠右（等宽数字字体，与歌单计数同套）。
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(item.nickname.clone()),
                        )
                        // 通知 tab：昵称后面跟动作（如「赞了你的评论」），
                        // 收缩优先级低于昵称，昵称过长时先截昵称保动作。
                        .when(!item.action.is_empty(), |row| {
                            row.child(
                                div()
                                    .flex_none()
                                    .max_w(px(120.))
                                    .truncate()
                                    .text_size(px(14.))
                                    .text_color(colors.muted_foreground)
                                    .child(item.action.clone()),
                            )
                        })
                        .child(
                            div()
                                .ml_auto()
                                .flex_none()
                                .text_size(px(12.))
                                .font_family(DOLPHIN_FAMILY)
                                .text_color(colors.foreground.alpha(0.45))
                                .child(message_time(item.time)),
                        ),
                )
                // 第二行：正文。GPUI 的空白折叠规则会把正文里的换行折成空格，
                // 所以这里只管放不下时省略，不会把行撑高。
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(13.))
                        .text_color(colors.muted_foreground)
                        .child(item.content.clone()),
                ),
        )
        .into_any_element()
}

/// 消息时间：今天显示时分，昨天带「昨天」前缀，今年显示月日，跨年补上年份
/// （月日/年份部分与评论日期同一套规则）。
fn message_time(millis: i64) -> String {
    let Ok(date) = time::OffsetDateTime::from_unix_timestamp(millis / 1000) else {
        return String::new();
    };
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let date = date.to_offset(now.offset());
    if date.date() == now.date() {
        format!("{:02}:{:02}", date.hour(), date.minute())
    } else if Some(date.date()) == now.date().previous_day() {
        format!("昨天{:02}:{:02}", date.hour(), date.minute())
    } else if date.year() == now.year() {
        format!("{:02}-{:02}", u8::from(date.month()), date.day())
    } else {
        format!(
            "{}-{:02}-{:02}",
            date.year(),
            u8::from(date.month()),
            date.day()
        )
    }
}
