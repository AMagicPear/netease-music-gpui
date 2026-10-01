use gpui::*;
use gpui_kit::base::Theme;

use super::ContentPage;
use crate::state::user::UserProfile;

/// 我喜欢的音乐
pub struct FavoriteMusicPage {
    user_profile: Entity<UserProfile>,
    _user_profile_subscription: Subscription,
}

impl FavoriteMusicPage {
    pub fn new(user_profile: Entity<UserProfile>, cx: &mut Context<Self>) -> Self {
        let user_profile_subscription = cx.observe(&user_profile, |_, _, cx| cx.notify());
        Self {
            user_profile,
            _user_profile_subscription: user_profile_subscription,
        }
    }
}

impl Render for FavoriteMusicPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
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
                                    .text_size(px(14.))
                                    .font_family("Trebuchet MS")
                                    .font_weight(FontWeight::SEMIBOLD)
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
                                                img(user_profile.avatar_path.clone())
                                                    .size(px(28.))
                                                    .rounded_full()
                                                    .flex_none(),
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
                            .child("2"),
                    ),
            )
            // 控件区域
            .child(div().flex().justify_between())
            // 虚拟列表区域
            .child("")
    }
}
