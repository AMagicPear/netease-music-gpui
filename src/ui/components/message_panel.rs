//! 消息内容（私信 / 评论 / @我 / 通知）：挂在全局 [`super::side_panel::SidePanel`]
//! 里的一页内容，外壳的定位、动画、开合都在那边。
//!
//! 数据来自四个消息接口（`msg_private` / `msg_comments` / `msg_forwards` /
//! `msg_notices`），解析统一落在 [`MessageItem`]。每个 tab 独立缓存一份、首次
//! 切到才拉取，面板每次展开强制刷新当前 tab；「一键全读」清本地徽标的同时
//! 调服务端 `/api/msg/count/clear` 同步未读计数（见 [`Self::mark_all_read`]）。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{ColorTokens, Scrollbar, ScrollbarMode, Theme};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::avatar::Avatar;

use super::play_queue::{PANEL_HEADER_HEIGHT, header_text_button};
use super::spinner;
use super::tab_bar::{TabBar, TabChanged, TabItem};
use crate::api::MusicApi;
use crate::models::MessageItem;
use crate::state::account::AccountState;
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

const TAB_PRIVATE: usize = 0;
const TAB_COMMENTS: usize = 1;
const TAB_MENTIONS: usize = 2;
const TAB_COUNT: usize = 4;

/// 一个 tab 的数据与加载状态。
#[derive(Default)]
struct TabData {
    items: Vec<MessageItem>,
    /// 服务端给出的账号级未读数（私信、@我 接口提供，其余为 0）。
    unread_total: u32,
    loaded: bool,
    loading: bool,
    error: Option<String>,
    /// 请求代际：切 tab / 重开面板会发起新请求，旧响应按代际丢弃。
    generation: u32,
}

pub struct MessagePanel {
    account: Entity<AccountState>,
    tabs: Entity<TabBar>,
    /// 当前展示的 tab；与 `tabs` 的选中项保持一致。
    active_tab: usize,
    tab_data: [TabData; TAB_COUNT],
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
        Self {
            account,
            tabs,
            active_tab: TAB_PRIVATE,
            tab_data: Default::default(),
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
        if visible {
            self.load_tab(self.active_tab, true, cx);
        }
        cx.notify();
    }

    fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.active_tab == index {
            return;
        }
        self.active_tab = index;
        // 换 tab 回到顶部；这个 tab 没拉过就先拉一份。
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.load_tab(index, false, cx);
        cx.notify();
    }

    /// 确保某个 tab 有数据。`force` 为真时即使加载过也重拉（面板展开时）。
    fn load_tab(&mut self, tab: usize, force: bool, cx: &mut Context<Self>) {
        let data = &mut self.tab_data[tab];
        if data.loading || (data.loaded && !force) {
            return;
        }
        let uid = self.account.read(cx).profile.user_id;
        if uid == 0 {
            data.loading = false;
            data.error = Some("未登录".into());
            cx.notify();
            return;
        }
        data.generation += 1;
        let generation = data.generation;
        data.loading = true;
        let api = MusicApi::global(cx);
        let request = api.runtime.spawn(fetch_tab(api.client.clone(), tab, uid));
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("消息请求任务失败".into()));
            let _ = this.update(cx, |this, cx| this.apply_tab(tab, generation, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn apply_tab(
        &mut self,
        tab: usize,
        generation: u32,
        result: Result<(Vec<MessageItem>, u32), String>,
        cx: &mut Context<Self>,
    ) {
        let data = &mut self.tab_data[tab];
        if data.generation != generation {
            return;
        }
        data.loading = false;
        match result {
            Ok((items, unread_total)) => {
                data.items = items;
                data.unread_total = unread_total;
                data.error = None;
                data.loaded = true;
                self.sync_tab_badge(tab, cx);
            }
            Err(error) => data.error = Some(error),
        }
        cx.notify();
    }

    /// tab 上的未读角标：只有接口给出账号级未读数的两个 tab 才显示。
    fn sync_tab_badge(&mut self, tab: usize, cx: &mut Context<Self>) {
        let unread = self.tab_data[tab].unread_total;
        let count = if matches!(tab, TAB_PRIVATE | TAB_MENTIONS) && unread > 0 {
            Some(unread.to_string())
        } else {
            None
        };
        self.tabs
            .update(cx, |tabs, cx| tabs.set_count(tab, count, cx));
    }

    /// 一键全读：先清本地未读（行内徽标 + tab 角标），再同步服务端。
    ///
    /// 必须调服务端：面板每次展开会强制重拉，如果只清本地，服务端的未读数
    /// 会把徽标盖回来。接口是官方「清空全部新消息提醒」用的 `/api/msg/count/clear`
    /// （从官方 web 客户端代码里扒出来的，探针 `probe_msg_count_clear` 实测有效）。
    /// 请求失败时静默忽略：本地已经清了，下次重拉徽标会如实回来。
    fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        for tab in 0..TAB_COUNT {
            let data = &mut self.tab_data[tab];
            data.unread_total = 0;
            for item in &mut data.items {
                item.unread = 0;
            }
            self.sync_tab_badge(tab, cx);
        }
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let request = api
            .runtime
            .spawn(async move { MusicApi::clear_message_counts(&client).await });
        cx.spawn(async move |this, cx| {
            if let Err(error) = request
                .await
                .unwrap_or_else(|_| Err("已读请求任务失败".into()))
            {
                eprintln!("一键全读同步服务端失败：{error}");
                let _ = this.update(cx, |_, _| {});
            }
        })
        .detach();
        cx.notify();
    }
}

/// 四个 tab 各自的接口；统一成「条目 + 账号级未读数」的返回。
async fn fetch_tab(
    client: crate::api::ApiClient,
    tab: usize,
    uid: u64,
) -> Result<(Vec<MessageItem>, u32), String> {
    match tab {
        TAB_PRIVATE => MusicApi::private_messages(&client, uid).await,
        TAB_COMMENTS => MusicApi::comment_messages(&client, uid)
            .await
            .map(|items| (items, 0)),
        TAB_MENTIONS => MusicApi::mention_messages(&client).await,
        // 其余（通知）没有账号级未读数。
        _ => MusicApi::notice_messages(&client)
            .await
            .map(|items| (items, 0)),
    }
}

impl Render for MessagePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let data = &self.tab_data[self.active_tab];
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
fn message_list_state(data: &TabData, colors: ColorTokens) -> Vec<AnyElement> {
    let centered = |content: AnyElement| -> AnyElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(content)
            .into_any_element()
    };
    if data.loading && data.items.is_empty() {
        return vec![centered(
            spinner("message-panel-loading", 20., colors.muted_foreground).into_any_element(),
        )];
    }
    if let Some(error) = &data.error {
        return vec![centered(
            div()
                .text_size(px(13.))
                .text_color(colors.muted_foreground)
                .child(error.clone())
                .into_any_element(),
        )];
    }
    if data.items.is_empty() {
        return vec![centered(
            div()
                .text_size(px(13.))
                .text_color(colors.muted_foreground)
                .child("暂无消息")
                .into_any_element(),
        )];
    }
    data.items
        .iter()
        .map(|item| message_row(item, colors))
        .collect()
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
                // 第二行：正文截断（`source` 仍会解析，UI 暂不显示）。
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
