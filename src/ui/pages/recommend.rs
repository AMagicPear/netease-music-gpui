use std::time::Duration;

use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use crate::models::RecommendedPlaylist;
use crate::state::recommend::RecommendedPlaylists;
use crate::ui::assets::{track_cover_image, track_cover_url};
use crate::ui::components::{centered_empty, play_count_label, spinner};
use crate::ui::cover_color::{CoverColor, dark_gradient};
use crate::ui::events::{Navigate, NavigationBus};
use crate::ui::theme::{DOLPHIN_FAMILY, PRESSED_ICON_ALPHA};

/// 区块之间的竖向间距。
const SECTION_GAP: Pixels = px(28.);
/// 卡片之间的水平间距（列数公式里的「一列间距」也是它）。
const CARD_GAP: f32 = 16.;
/// 封面圆角，与歌单详情页封面一致。
const COVER_RADIUS: Pixels = px(8.);
/// 刷新图标转一圈的时长，和 `spinner.rs` 的 `SPIN_PERIOD` 一致，读作同一类加载。
const SPIN_PERIOD: Duration = Duration::from_millis(900);
/// 封面下方 caption 区的高度。官方实测整卡 190px = 142px 封面 + 48px caption 区；
/// 高度不随窗口变（宽窗口下封面变大、caption 区仍是 48px）。
const CARD_TEXT_HEIGHT: Pixels = px(48.);

/// 还没有接数据源的卡片用的兜底封面：应用自带的空白专辑图。
const PLACEHOLDER_COVER: &str = "images/trackBlank.png";

/// 推荐页。
///
/// 三块内容：「顶部入口」「推荐歌单」「精选活动」。只有「推荐歌单」接了真实数据
/// （`RecommendedPlaylists` store 跟随登录状态加载）；顶部入口和精选活动是固定栏目，
/// 等各自的数据源接好后再填，先还原外观。
///
/// 卡片布局依赖内容区宽度（一排放几张随宽度增长），所以不实现 [`Render`]，
/// 由 shell 渲染时通过 [`Self::content`] 把宽度传进来——同歌单页 `compact` 模式的路子。
pub struct RecommendPage {
    recommended: Entity<RecommendedPlaylists>,
    /// 顶部入口各自的封面取色缓存，和 `QUICK_ENTRIES` 一一对应：caption 区的底色
    /// 从封面取色压暗得来，与 img 共用 GPUI 图片缓存，新封面加载中保留上一张的颜色。
    entry_colors: Vec<CoverColor>,
}

impl RecommendPage {
    pub fn new(recommended: Entity<RecommendedPlaylists>) -> Self {
        Self {
            recommended,
            entry_colors: (0..QUICK_ENTRIES.len())
                .map(|_| CoverColor::default())
                .collect(),
        }
    }

    /// shell 的渲染入口：内容区宽度决定一排放几张卡片。
    /// 同歌单页 `content`：pages 里的页面 View 由 shell 注入布局参数后渲染。
    /// `window` 用于给入口封面取色（走 GPUI 图片缓存，和 img 同一份数据）。
    pub fn content(
        view: Entity<Self>,
        available_width: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        view.update(cx, |this, cx| {
            for (entry, color) in QUICK_ENTRIES.iter().zip(this.entry_colors.iter_mut()) {
                color.load(entry.cover, window, cx);
            }
            this.render_page(available_width, cx)
        })
    }

    fn render_page(&self, available_width: Pixels, cx: &Context<Self>) -> AnyElement {
        let colors = Theme::global(cx).tokens.colors;
        // 「每日推荐」角标显示当天日期；取本地时间，拿不到退回 UTC。
        let today = time::OffsetDateTime::now_local()
            .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
            .day();
        // 两行卡片共用同一套列数，卡片宽度才对得齐；封顶到入口数量
        // （顶部入口就这么多个，没有更多内容可放时两行一起拉伸）。
        let columns = columns_for(available_width).min(QUICK_ENTRIES.len());
        // caption 区底色现读取色缓存：渲染期间图片可能还没到，取到再随重绘换上。
        let strip_colors: Vec<Option<Hsla>> =
            self.entry_colors.iter().map(CoverColor::current).collect();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(SECTION_GAP)
            .child(quick_entries(today, columns, &strip_colors))
            .child(self.recommended_section(columns, colors, cx))
            .child(featured_activities(colors))
            .into_any_element()
    }
}

/// 一行放几张：内容区宽 ÷（单卡目标宽 + 间距），窗口越宽越多。
/// 数值大致复现官方（852 宽约 5 张、1307 宽约 7 张）即可，不较真。
const TARGET_CARD_WIDTH: f32 = 135.;

/// 内容区宽度 → 一行排几张。封顶交给调用方按入口数量裁。
fn columns_for(available_width: Pixels) -> usize {
    let per_column = TARGET_CARD_WIDTH + CARD_GAP;
    ((f32::from(available_width) + CARD_GAP) / per_column)
        .floor()
        .max(1.) as usize
}

/// 顶部一排固定入口。目前只还原外观：点击行为等各自的数据源接好后再补。
struct QuickEntry {
    title: &'static str,
    /// 标题下面的一行说明，取官方文案里静态可表达的部分。
    caption: &'static str,
    /// 封面地址；caption 区的底色就是从它取色压暗得来的。
    cover: &'static str,
    badge: Option<QuickEntryBadge>,
}

/// 封面左上角的角标，标题挨着它排在右边。
enum QuickEntryBadge {
    /// 「每日推荐」：当日日期（几号）。
    Date,
}

const QUICK_ENTRIES: [QuickEntry; 7] = [
    QuickEntry {
        title: "每日推荐",
        caption: "每天更新的个性化推荐",
        cover: PLACEHOLDER_COVER,
        badge: Some(QuickEntryBadge::Date),
    },
    QuickEntry {
        title: "心动模式",
        caption: "你的红心歌曲和更多相似推荐",
        cover: PLACEHOLDER_COVER,
        badge: None,
    },
    QuickEntry {
        title: "私人雷达",
        caption: "今日份的私人雷达歌单",
        cover: PLACEHOLDER_COVER,
        badge: None,
    },
    QuickEntry {
        title: "私人漫游",
        caption: "开启一场私人漫游之旅",
        cover: PLACEHOLDER_COVER,
        badge: None,
    },
    QuickEntry {
        title: "相似歌曲",
        caption: "从你喜欢的歌听起",
        cover: PLACEHOLDER_COVER,
        badge: None,
    },
    // 这两个入口官方按用户的听歌风格动态生成（「xx 日推」「听 xx 歌手」），
    // 名字和内容都跟着风格变；数据源还没接，先放占位文案占住位置。
    QuickEntry {
        title: "风格日推",
        caption: "根据你的听歌风格生成",
        cover: PLACEHOLDER_COVER,
        badge: None,
    },
    QuickEntry {
        title: "常听歌手",
        caption: "从你常听的歌手听起",
        cover: PLACEHOLDER_COVER,
        badge: None,
    },
];

/// 「精选活动」的占位标题；运营位接口接好后换成真数据。
const FEATURED_ACTIVITIES: [&str; 3] = ["JJ LIN 林俊杰", "可是这世界，总有温暖", "MICHAEL JACKSON"];

/// 区块标题：加粗前景色。
fn section_title(text: &'static str, colors: ColorTokens) -> impl IntoElement {
    div()
        .text_size(px(16.))
        .font_weight(FontWeight::BOLD)
        .text_color(colors.foreground)
        .child(text)
}

/// 顶部一排入口：封面 + caption 区是一个整体矩形，和「推荐歌单」共用同一套列数。
///
/// 窗口越宽排得越多（官方 852 宽 5 个、1307 宽 7 个），窄窗口则只放得下前几个。
/// 卡片 `flex_1` 按内容区宽度平分，永远铺满一行——不存在「放不下就换行」，
/// 尺寸完全由窗口宽度算出来，而不是按某个下限尺寸定宽。
fn quick_entries(
    today: u8,
    columns: usize,
    strip_colors: &[Option<Hsla>],
) -> impl IntoElement {
    div()
        .flex()
        .gap(px(CARD_GAP))
        .children(
            QUICK_ENTRIES
                .iter()
                .take(columns)
                .enumerate()
                .map(|(index, entry)| {
                    let strip = strip_color(strip_colors.get(index).copied().flatten());
                    quick_entry(entry, today, strip)
                }),
        )
}

/// caption 区底色：从封面取色，按黑胶页那套分段映射压暗，取到深色纯色；
/// 图还没加载出来（取色为 `None`）时退回中性深蓝灰，不会透出页面背景。
fn strip_color(color: Option<Hsla>) -> Hsla {
    dark_gradient(color).0[1].into()
}

fn quick_entry(
    entry: &QuickEntry,
    today: u8,
    strip: Hsla,
) -> impl IntoElement {
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .child(
            // 封面 + caption 是一个整体：圆角在大矩形这一层，但 GPUI 的 overflow_hidden
            // 按矩形裁剪、不会裁出圆角，所以圆角要直接设在子元素上——封面只圆顶两角
            // （下边两角见方，正好和 caption 接上），caption 只圆底两角。底色用取色，
            // 图没出来也不透。
            div()
                .w_full()
                .rounded(COVER_RADIUS)
                .overflow_hidden()
                .bg(strip)
                .flex()
                .flex_col()
                .child(
                    div()
                        .w_full()
                        // 封面是正方形：宽度跟随卡片（flex 平分后的宽度），高按 1:1 跟上。
                        .aspect_ratio(1.)
                        .relative()
                        .child(
                            // 封面只圆顶两角：下边见方，和下面 caption 区无缝相接。
                            img(entry.cover)
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .rounded_t(COVER_RADIUS),
                        )
                        // 顶部渐隐遮罩：和「推荐歌单」卡片同一份配方，保证亮封面上的白字清晰。
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded_t(COVER_RADIUS)
                                .bg(linear_gradient(
                                    180.,
                                    linear_color_stop(black().alpha(0.2), 0.),
                                    linear_color_stop(black().alpha(0.), 0.35),
                                )),
                        )
                        // 封面左上：角标（有的话）+ 挨着它排的白色标题。
                        .child(
                            div()
                                .absolute()
                                .top(px(6.))
                                .left(px(8.))
                                .right(px(8.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .children(entry.badge.as_ref().map(|_| date_badge(today)))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .text_size(px(16.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(white())
                                        .truncate()
                                        .child(entry.title),
                                ),
                        ),
                )
                .child(
                    // caption 区：从封面取来的深色底 + 白字；只圆底两角。
                    div()
                        .h(CARD_TEXT_HEIGHT)
                        .bg(strip)
                        .rounded_b(COVER_RADIUS)
                        .px(px(10.))
                        .flex()
                        .flex_col()
                        .justify_center()
                        .child(
                            div()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .line_clamp(2)
                                .text_color(white())
                                .child(entry.caption),
                        ),
                ),
        )
}

/// 「每日推荐」的当日日期：深色小方块压一条白边，读作日历的形状；标题挨着它排在右边。
fn date_badge(day: u8) -> AnyElement {
    div()
        .flex_none()
        .w(px(22.))
        .h(px(24.))
        .rounded(px(4.))
        .bg(black().alpha(0.55))
        .flex()
        .flex_col()
        .items_center()
        .overflow_hidden()
        // 白条 + 两个小孔：日历的装订边。
        .child(
            div()
                .w_full()
                .h(px(5.))
                .bg(white().alpha(0.85))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(7.))
                .child(
                    div()
                        .w(px(2.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(black().alpha(0.45)),
                )
                .child(
                    div()
                        .w(px(2.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(black().alpha(0.45)),
                ),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white())
                .child(day.to_string()),
        )
        .into_any_element()
}

impl RecommendPage {
    /// 「推荐歌单」区块：标题 + 刷新按钮 + 卡片行。加载中 / 出错 / 空列表
    /// 都按 store 的状态渲染对应的那一种。
    fn recommended_section(
        &self,
        columns: usize,
        colors: ColorTokens,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section_title("推荐歌单", colors))
                    .child(self.refresh_button(colors, cx)),
            )
            .child(self.recommended_body(columns, colors, cx))
    }

    /// 刷新按钮：加载中让图标自身绕中心匀速旋转——同一份加载语义，但复用
    /// 刷新图标本体（不另换一个缓存的转圈图标），转起来就自然读作「加载中」。
    fn refresh_button(&self, colors: ColorTokens, cx: &Context<Self>) -> AnyElement {
        let loading = self.recommended.read(cx).loading;
        if loading {
            // 加载中：图标自旋即加载反馈；此时按钮处于「进行中」，不再响应
            // hover/active/点击（和原先换成纯 spinner 时不可交互一致）。
            return svg()
                .path("icons/refresh.svg")
                .size(px(16.))
                .flex_none()
                .text_color(colors.muted_foreground)
                .with_animation(
                    "recommend-refresh-loading",
                    Animation::new(SPIN_PERIOD).repeat(),
                    |icon, delta| {
                        icon.with_transformation(Transformation::rotate(percentage(delta)))
                    },
                )
                .into_any_element();
        }
        // 空闲：静态刷新图标，保留 hover/active 换色与点击刷新。
        // `active` 属于 StatefulInteractiveElement，必须跟在 `.id()` 之后。
        svg()
            .path("icons/refresh.svg")
            .size(px(16.))
            .flex_none()
            .text_color(colors.muted_foreground)
            .hover(|style| style.text_color(colors.foreground))
            .id("recommend-refresh")
            .cursor_pointer()
            .role(Role::Button)
            .aria_label("换一批推荐歌单")
            .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.recommended.update(cx, |store, cx| store.reload(cx));
            }))
            .into_any_element()
    }

    fn recommended_body(
        &self,
        columns: usize,
        colors: ColorTokens,
        cx: &Context<Self>,
    ) -> AnyElement {
        let recommended = self.recommended.read(cx);
        if recommended.loading && recommended.playlists.is_empty() {
            return div()
                .w_full()
                .py(px(36.))
                .flex()
                .justify_center()
                .child(spinner(
                    "recommend-loading",
                    20.,
                    colors.muted_foreground,
                ))
                .into_any_element();
        }
        if recommended.playlists.is_empty() {
            return match &recommended.error {
                Some(error) => div()
                    .w_full()
                    .py(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .text_size(px(14.))
                    .text_color(colors.muted_foreground)
                    .child(error.clone())
                    .child(
                        Button::new("recommend-retry")
                            .child("重试")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.recommended.update(cx, |store, cx| store.reload(cx));
                            })),
                    )
                    .into_any_element(),
                // 未登录时接口不会给内容：提示登录，而不是报一个错。
                None => centered_empty(
                    "images/empty/cm_empty_wuzuizen.png",
                    "登录后查看每日推荐歌单",
                    colors,
                )
                .into_any_element(),
            };
        }
        // 只铺满一行，列数和顶部入口一致（两张一行才对得齐）：窗口越宽列数越多，
        // 一行放下的推荐歌单也跟着变多。卡片 flex_1 平分整行，不换行也不会溢出。
        let mut row = div().flex().gap(px(CARD_GAP));
        for playlist in recommended.playlists.iter().take(columns) {
            row = row.child(recommended_card(playlist, colors, cx));
        }
        row.into_any_element()
    }
}

/// 推荐歌单卡片：正方形封面 + 播放量（耳机图标 + Dolphin 数字）+ 两行歌单名。
/// 整卡是入口：点它发 typed event（消息层），导航和详情加载归 `MainContent`。
fn recommended_card(
    playlist: &RecommendedPlaylist,
    colors: ColorTokens,
    cx: &Context<RecommendPage>,
) -> impl IntoElement {
    let playlist_id = playlist.id;
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .id(("recommend-playlist", playlist_id))
        .cursor_pointer()
        .child(
            div()
                .w_full()
                .aspect_ratio(1.)
                .relative()
                .rounded(COVER_RADIUS)
                .overflow_hidden()
                .bg(colors.muted)
                .child(
                    // track_cover_url 顺手做两件事：空地址换默认封面，真实地址拼上
                    // CDN 缩放参数，不必下载原图再在本地缩。
                    img(track_cover_image(track_cover_url(
                        playlist.pic_url.as_deref(),
                        300,
                    )))
                    .size_full()
                    .object_fit(ObjectFit::Cover),
                )
                // 播放量在封面内部，从顶部向下渐淡才看得清（同歌单详情页封面）。
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(COVER_RADIUS)
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(black().alpha(0.2), 0.),
                            linear_color_stop(black().alpha(0.), 0.35),
                        )),
                )
                .child(
                    div()
                        .absolute()
                        .top_1()
                        .right_2()
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .text_color(white())
                        .text_size(px(12.))
                        .font_family(DOLPHIN_FAMILY)
                        .child(
                            svg()
                                .path("icons/headphone.svg")
                                .size(px(13.))
                                .flex_none()
                                .text_color(white()),
                        )
                        .child(play_count_label(playlist.play_count)),
                ),
        )
        .child(
            div()
                .h(CARD_TEXT_HEIGHT)
                .pt(px(2.))
                .text_size(px(13.))
                .line_height(px(18.))
                .line_clamp(2)
                .text_color(colors.secondary_foreground)
                .child(playlist.name.clone()),
        )
        .on_click(cx.listener(move |_, _, _, cx| {
            NavigationBus::emit(cx, Navigate::Playlist { id: playlist_id, summary: None });
        }))
}

/// 「精选活动」区块：三张横版活动卡，标题压在封面底部（官方同款渐变托底）。
/// 同样是占位内容，等运营位接口接好再换真数据。
fn featured_activities(colors: ColorTokens) -> impl IntoElement {
    let mut row = div().flex().gap(px(CARD_GAP));
    for title in FEATURED_ACTIVITIES {
        row = row.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(150.))
                .relative()
                .rounded(COVER_RADIUS)
                .overflow_hidden()
                .bg(colors.muted)
                .child(
                    img(PLACEHOLDER_COVER)
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(COVER_RADIUS)
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(black().alpha(0.), 0.55),
                            linear_color_stop(black().alpha(0.45), 1.),
                        )),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(14.))
                        .right(px(14.))
                        .bottom(px(12.))
                        .text_size(px(14.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(white())
                        .line_clamp(1)
                        .child(title),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(section_title("精选活动", colors))
        .child(row)
}

#[cfg(test)]
mod tests {
    use super::{QUICK_ENTRIES, columns_for, px};

    /// 实际显示列数：宽度算出来后再封顶到入口数量。
    fn shown_columns(width: f32) -> usize {
        columns_for(px(width)).min(QUICK_ENTRIES.len())
    }

    /// 窗口越宽列数越多，且不超过入口数量；极窄也至少有一列。
    #[test]
    fn columns_grow_with_width_and_never_exceed_entry_count() {
        assert!(shown_columns(1300.) > shown_columns(700.));
        assert!(shown_columns(5000.) <= QUICK_ENTRIES.len());
        assert!(shown_columns(100.) >= 1);
    }
}
