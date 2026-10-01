use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::native_menu::NativeMenu;

/// 操作按钮的统一高度
const ACTION_BUTTON_HEIGHT: Pixels = px(36.);
/// 带文字按钮的左右内边距，宽度靠它 + 内容自适应
const ACTION_BUTTON_PADDING: Pixels = px(12.);

use super::ContentPage;
use crate::components::{TabBar, TabChanged, TabItem};
use crate::state::user::UserProfile;
use crate::theme::{DOLPHIN_FAMILY, PRESSED_OPACITY};

// 「更多」菜单里的三个命令。
//
// `NativeMenu` 的每一项都挂一个 GPUI `Action`，选中后由 `Window::dispatch_action`
// 派发 —— 和系统菜单栏、快捷键走的是同一套机制，所以将来只要在某个视图上
// `on_action(...)` 就能接住，不必改菜单代码。现在还没有任何监听者，
// 选中即派发到空处，等于什么都不做。
actions!(favorite_music, [Share, BatchOperation, AddAllToPlaylist]);

#[derive(Clone, Copy)]
enum FavoriteMusicTab {
    Songs,
    Comments,
    Collectors,
}

impl FavoriteMusicTab {
    fn content_title(self) -> &'static str {
        match self {
            Self::Songs => "歌曲列表",
            Self::Comments => "评论",
            Self::Collectors => "收藏者",
        }
    }
}

/// 我喜欢的音乐
pub struct FavoriteMusicPage {
    user_profile: Entity<UserProfile>,
    _user_profile_subscription: Subscription,
    tabs: Entity<TabBar>,
    _tabs_subscription: Subscription,
}

impl FavoriteMusicPage {
    pub fn new(user_profile: Entity<UserProfile>, cx: &mut Context<Self>) -> Self {
        let user_profile_subscription = cx.observe(&user_profile, |_, _, cx| cx.notify());
        let tabs = cx.new(|_| {
            TabBar::new(vec![
                TabItem::new("歌曲").count("1193"),
                TabItem::new("评论"),
                TabItem::new("收藏者").count("5"),
            ])
        });
        let tabs_subscription = cx.subscribe(&tabs, |_, _, _: &TabChanged, cx| cx.notify());
        Self {
            user_profile,
            _user_profile_subscription: user_profile_subscription,
            tabs,
            _tabs_subscription: tabs_subscription,
        }
    }
}

/// 「播放全部」：主题红实心按钮，白字白图标，无边框。
///
/// 主操作用实心色块来吸引视线，所以它刻意不描边 —— 描边在纯色填充上
/// 只会削弱色块边缘的锐利感。
fn play_all_button(colors: ColorTokens) -> Button {
    Button::new("favorite-play-all-button")
        .h(ACTION_BUTTON_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        // 宽度交给内容：图标 + 文字 + 左右内边距
        .px(ACTION_BUTTON_PADDING)
        .rounded_lg()
        .bg(colors.primary)
        // hover 只改颜色（向背景靠一档），按下统一降整体透明度
        .hover(|style| style.bg(colors.primary.alpha(0.88)))
        .active(|style| style.opacity(PRESSED_OPACITY))
        .text_size(px(13.))
        .text_color(colors.primary_foreground)
        .child(
            svg()
                .path("icons/play.svg")
                .size(px(18.))
                .flex_none()
                .text_color(colors.primary_foreground),
        )
        .child("播放全部")
}

impl Render for FavoriteMusicPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let active_tab = [
            FavoriteMusicTab::Songs,
            FavoriteMusicTab::Comments,
            FavoriteMusicTab::Collectors,
        ][self.tabs.read(cx).selected_index()];
        let cover_path = "/Users/amagicpear/Pictures/Perry Origin Character/ChatGPT Image 2026年9月29日 15_39_30.png";
        let user_profile = self.user_profile.read(cx);

        div()
            // 封面标题区域
            .child(
                div()
                    .flex()
                    .justify_start()
                    .gap_6()
                    .child(
                        div()
                            .w(px(170.))
                            .h(px(170.))
                            .flex_none()
                            .relative()
                            .overflow_hidden()
                            // 歌单封面
                            .child(
                                img(cover_path)
                                    .size_full()
                                    .object_fit(ObjectFit::Cover)
                                    .rounded(px(8.)),
                            )
                            // 播放量
                            .child(
                                div()
                                    .absolute()
                                    .top_1()
                                    .right_2()
                                    .flex()
                                    .items_center()
                                    .child(
                                        svg()
                                            .path("icons/headphone.svg")
                                            .size(px(15.))
                                            .text_color(colors.primary_foreground),
                                    )
                                    .text_color(colors.primary_foreground)
                                    .text_size(px(15.))
                                    .font_family(DOLPHIN_FAMILY)
                                    .child(7735.to_string()),
                            )
                            // 正中间的爱心图标，仅「我喜欢的音乐」有
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        svg()
                                            .path("icons/like.svg")
                                            .size(px(96.))
                                            .text_color(colors.primary_foreground)
                                            .opacity(0.95),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .child(
                                div()
                                    .child(
                                        div()
                                            .text_size(px(24.))
                                            .line_height(rems(3.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(colors.foreground)
                                            .child(text!(ContentPage::FavoriteMusic.title())),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                Avatar::new()
                                                    .with_size(px(26.))
                                                    .flex_none()
                                                    .src(user_profile.avatar_path.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(colors.secondary_foreground)
                                                    .child(user_profile.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .ml_2()
                                                    .text_size(px(12.))
                                                    .text_color(colors.muted_foreground)
                                                    .font_weight(FontWeight::LIGHT)
                                                    .child("2017-05-31创建"),
                                            ),
                                    ),
                            )
                            // 操作按钮组：播放全部 / 下载 / 更多
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(play_all_button(colors))
                                    // 下载：宽度自适应，比 muted 更浅的底 + 比 muted 更浅的描边，
                                    // 文字/图标用比 muted_foreground 深一档的灰保证可读
                                    .child(
                                        Button::new("favorite-download-button")
                                            .h(ACTION_BUTTON_HEIGHT)
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .gap(px(4.))
                                            .px(ACTION_BUTTON_PADDING)
                                            .border_1()
                                            .border_color(colors.foreground.alpha(0.06))
                                            .rounded_lg()
                                            .bg(colors.foreground.alpha(0.03))
                                            // 底色本身已有 3%，hover 只抬到 8%（+5 个点），
                                            // 观感与 header 返回键的 0 → accent(6%) 接近；
                                            // 按下不再换更深的颜色，统一降整体透明度
                                            .hover(|style| style.bg(colors.foreground.alpha(0.08)))
                                            .active(|style| style.opacity(PRESSED_OPACITY))
                                            .text_size(px(13.))
                                            .text_color(colors.secondary_foreground)
                                            .child(
                                                svg()
                                                    .path("icons/download.svg")
                                                    .size(px(18.))
                                                    .flex_none()
                                                    .text_color(colors.secondary_foreground),
                                            )
                                            .child("下载"),
                                    )
                                    // 更多：固定 36×36 的纯图标按钮，靠 flex 居中
                                    .child(
                                        Button::new("favorite-more-button")
                                            .size(px(36.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .border_1()
                                            .border_color(colors.foreground.alpha(0.06))
                                            .rounded_lg()
                                            .bg(colors.foreground.alpha(0.03))
                                            .hover(|style| style.bg(colors.foreground.alpha(0.08)))
                                            .active(|style| style.opacity(PRESSED_OPACITY))
                                            // 弹出系统原生菜单
                                            .on_click(|event, window, cx| {
                                                NativeMenu::new()
                                                    .menu("分享…", Box::new(Share))
                                                    .menu("批量操作", Box::new(BatchOperation))
                                                    .menu(
                                                        "添加全部至播放列表",
                                                        Box::new(AddAllToPlaylist),
                                                    )
                                                    .show(event.position(), window, cx);
                                            })
                                            .child(
                                                svg()
                                                    .path("icons/xpoint.svg")
                                                    .size(px(16.))
                                                    .flex_none()
                                                    .text_color(colors.secondary_foreground),
                                            ),
                                    ),
                            ),
                    ),
            )
            // 控件区域
            .child(div().mt(px(28.)).child(self.tabs.clone()))
            // 具体内容区域
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(colors.muted_foreground)
                    .child(active_tab.content_title()),
            )
    }
}
