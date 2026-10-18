use std::time::Duration;

use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use crate::models::{FeaturedActivity, RecommendedPlaylist};
use crate::state::recommend::{QuickEntry, QuickEntryKind, RecommendStore};
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

/// 还没有内容的卡片 / 入口用的兜底封面：应用自带的空白专辑图。
const PLACEHOLDER_COVER: &str = "images/trackBlank.png";

/// 推荐页。
///
/// 四块内容：「顶部入口」「推荐歌单」「雷达歌单」「精选活动」，全部来自官方
/// 3.1.41 的接口。数据都在 `RecommendStore` 里（跟随登录状态加载）。
///
/// 卡片布局依赖内容区宽度（一排放几张随宽度增长），所以不实现 [`Render`]，
/// 由 shell 渲染时通过 [`Self::content`] 把宽度传进来——同歌单页 `compact` 模式的路子。
pub struct RecommendPage {
    store: Entity<RecommendStore>,
    /// 顶部入口各自的封面取色缓存，和 store 里的 `entries` 按下标对齐：
    /// caption 区的底色从封面取色压暗得来，与 img 共用 GPUI 图片缓存，
    /// 新封面加载中保留上一张的颜色。
    entry_colors: Vec<CoverColor>,
}

impl RecommendPage {
    pub fn new(store: Entity<RecommendStore>) -> Self {
        Self {
            store,
            entry_colors: Vec::new(),
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
            let covers = this.entry_covers(cx);
            this.entry_colors
                .resize_with(covers.len(), CoverColor::default);
            for (cover, color) in covers.iter().zip(this.entry_colors.iter_mut()) {
                color.load(cover, window, cx);
            }
            this.render_page(available_width, &covers, cx)
        })
    }

    /// 每个入口的封面：用接口给的 `coverUrl`（私人雷达用雷达歌单封面），这里拼上
    /// CDN 缩放参数；没有内容就用静态占位图。
    fn entry_covers(&self, cx: &Context<Self>) -> Vec<SharedString> {
        self.store
            .read(cx)
            .entries
            .iter()
            .map(|entry| match &entry.cover {
                Some(cover) => track_cover_url(Some(cover), 300).into(),
                None => PLACEHOLDER_COVER.into(),
            })
            .collect()
    }

    fn render_page(
        &self,
        available_width: Pixels,
        covers: &[SharedString],
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = Theme::global(cx).tokens.colors;
        // 「每日推荐」角标显示当天日期；取本地时间，拿不到退回 UTC。
        let today = time::OffsetDateTime::now_local()
            .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
            .day();
        // 先把 store 里的内容拷出来，后面渲染各区块时不能再长期持有读锁。
        let (entries, radar, featured) = {
            let store = self.store.read(cx);
            (
                store.entries.clone(),
                store.radar.clone(),
                store.featured.clone(),
            )
        };
        // 官方首屏顶部入口是每日推荐 + 若干风格日推（当前账号 5 个）。
        // 推荐歌单 / 雷达歌单沿用同一列数，卡片宽度才对得齐；入口还没回来时
        // 先按窗口能放下的列数排，避免整页空着。
        let natural_columns = columns_for(available_width);
        let columns = if entries.is_empty() {
            natural_columns
        } else {
            natural_columns.min(entries.len())
        };
        // caption 区底色现读取色缓存：渲染期间图片可能还没到，取到再随重绘换上。
        let strip_colors: Vec<Option<Hsla>> =
            self.entry_colors.iter().map(CoverColor::current).collect();
        let mut page = div().w_full().flex().flex_col().gap(SECTION_GAP);
        if !entries.is_empty() {
            page = page.child(quick_entries(
                today,
                columns,
                &entries,
                &strip_colors,
                covers,
            ));
        }
        page = page.child(self.recommended_section(columns, colors, cx));
        if !radar.is_empty() {
            page = page.child(Self::radar_section(&radar, columns, colors, cx));
        }
        if !featured.is_empty() {
            page = page.child(featured_activities(colors, &featured));
        }
        page.into_any_element()
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
    entries: &[QuickEntry],
    strip_colors: &[Option<Hsla>],
    covers: &[SharedString],
) -> impl IntoElement {
    div()
        .flex()
        .gap(px(CARD_GAP))
        .children(
            entries
                .iter()
                .take(columns)
                .enumerate()
                .map(|(index, entry)| {
                    let strip = strip_color(strip_colors.get(index).copied().flatten());
                    let cover = covers
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| PLACEHOLDER_COVER.into());
                    quick_entry(entry, today, strip, cover)
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
    cover: SharedString,
) -> impl IntoElement {
    div().flex_1().min_w(px(0.)).flex().flex_col().child(
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
                        img(cover)
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
                            // 每日推荐带日期角标，私人漫游带漫游图标，其余入口只留标题。
                            .children(match entry.kind {
                                QuickEntryKind::DailySongs => Some(date_badge(today)),
                                QuickEntryKind::Roam => Some(roam_badge()),
                                _ => None,
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(white())
                                    .truncate()
                                    .child(entry.title.clone()),
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
                            .child(entry.caption.clone()),
                    ),
            ),
    )
}

/// 「每日推荐」的日历图标正文，直接编译进二进制：这张图要在渲染前把当日数字
/// 拼进遮罩里，走 `AssetSource`（每次 `fs::read`）不如编译期嵌入省事；
/// 图标本身仍是 `assets/icons/calendar.svg`，改图要重新编译。
const CALENDAR_BADGE_SVG: &str = include_str!("../../../assets/icons/calendar.svg");
/// 图标里给数字留的插入点（见该 SVG 的 `<mask id="date-cut">`）。
const CALENDAR_DATE_SLOT: &str = "<!--DATE_TEXT-->";
/// 角标边长，与 SVG 的 viewBox（22×22）一一对应，换尺寸时下面的几何常量要一起看。
const CALENDAR_BADGE_SIZE: f32 = 22.;
/// 数字的视觉中心：图标上半有一条横向镂空的装订带（y 5.3~8.8），
/// 剩下那块白色空白区是 y 8.8~19.6，中心即 14.2。
const CALENDAR_DIGIT_CENTER_Y: f32 = 14.2;
/// 一位数只有一个字形，放大些才撑满同一块空白；两位数并排，小一号刚好放下。
const CALENDAR_DIGIT_SIZE_ONE: f32 = 11.5;
const CALENDAR_DIGIT_SIZE_TWO: f32 = 9.5;

/// 「每日推荐」的当日日期：白色日历图标，日期是从图标本体上「镂空」出来的
/// ——数字位置是透明的洞，直接透出下面的封面。
///
/// 做法是 SVG 遮罩：`assets/icons/calendar.svg` 里那张 `date-cut` 遮罩铺满白色
/// （白 = 保留），渲染前把当日数字用黑色画进遮罩（黑 = 抹掉），两者相乘就得到
/// 「日历减去数字」的形状。所以数字不是压在图标上的文字，而是被挖掉的一块，
/// 背后是什么颜色就露出什么颜色，不必去采样封面取色。
fn date_badge(day: u8) -> AnyElement {
    let size = if day < 10 {
        CALENDAR_DIGIT_SIZE_ONE
    } else {
        CALENDAR_DIGIT_SIZE_TWO
    };
    // `<text>` 的 y 是基线；Dolphin 的 capHeight 是 0.7em，想让数字的中缝落在
    // `CALENDAR_DIGIT_CENTER_Y`，基线就得再往下挪半个字高。
    let baseline = CALENDAR_DIGIT_CENTER_Y + size * 0.35;
    let digits = format!(
        r##"<text x="11" y="{baseline:.3}" font-family="Dolphin" font-size="{size}" font-weight="bold" text-anchor="middle" fill="#000">{day}</text>"##
    );
    let source = CALENDAR_BADGE_SVG.replace(CALENDAR_DATE_SLOT, &digits);
    svg()
        // `.data` 按内容哈希做缓存键：日期不变时拿到的还是同一份栅格结果。
        .data(source.as_bytes())
        .size(px(CALENDAR_BADGE_SIZE))
        .flex_none()
        // 单色图标由 text_color 染色（取的是形状的 alpha），镂空的洞保持透明。
        .text_color(white())
        .into_any_element()
}

/// 「私人漫游」标题前的图标，和每日推荐的日历角标同尺寸、同白色。
/// SVG 里用 `currentColor` 填充，`text_color` 就能控制颜色。
const ROAM_BADGE_SIZE: f32 = 22.;

fn roam_badge() -> AnyElement {
    svg()
        .path("icons/radio_in_recommend.svg")
        .size(px(ROAM_BADGE_SIZE))
        .flex_none()
        .text_color(white())
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
        let loading = self.store.read(cx).loading;
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
                this.store.update(cx, |store, cx| store.reload(cx));
            }))
            .into_any_element()
    }

    fn recommended_body(
        &self,
        columns: usize,
        colors: ColorTokens,
        cx: &Context<Self>,
    ) -> AnyElement {
        let store = self.store.read(cx);
        if store.loading && store.playlists.is_empty() {
            return div()
                .w_full()
                .py(px(36.))
                .flex()
                .justify_center()
                .child(spinner("recommend-loading", 20., colors.muted_foreground))
                .into_any_element();
        }
        // 没登录（或还没取到）才给登录引导；登录了但服务端没推荐就只是空一行。
        if store.playlists.is_empty() && !store.loaded {
            return match &store.error {
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
                                this.store.update(cx, |store, cx| store.reload(cx));
                            })),
                    )
                    .into_any_element(),
                // 登录和游客都会取首页，走到这里基本是真的没内容。
                None => {
                    centered_empty("images/empty/cm_empty_wuzuizen.png", "暂无推荐歌单", colors)
                        .into_any_element()
                }
            };
        }
        // 只铺满一行，列数和顶部入口一致（两张一行才对得齐）：窗口越宽列数越多，
        // 一行放下的推荐歌单也跟着变多。卡片 flex_1 平分整行，不换行也不会溢出。
        let mut row = div().flex().gap(px(CARD_GAP));
        for playlist in store.playlists.iter().take(columns) {
            row = row.child(recommended_card(playlist, colors, cx, false));
        }
        row.into_any_element()
    }

    /// 「雷达歌单」区块：官方是独立区块，不挤在顶部入口里。卡片复用推荐歌单的
    /// 样式（封面 + 播放量 + 两行文案），标题来自雷达接口里的 `今天从《歌名》听起`。
    fn radar_section(
        radar: &[RecommendedPlaylist],
        columns: usize,
        colors: ColorTokens,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let mut row = div().flex().gap(px(CARD_GAP));
        for playlist in radar.iter().take(columns) {
            row = row.child(recommended_card(playlist, colors, cx, true));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(section_title("雷达歌单", colors))
            .child(row)
    }
}

/// 推荐歌单卡片：正方形封面 + 播放量（耳机图标 + Dolphin 数字）+ 两行歌单名。
/// 整卡是入口：点它发 typed event（消息层），导航和详情加载归 `MainContent`。
fn recommended_card(
    playlist: &RecommendedPlaylist,
    colors: ColorTokens,
    cx: &Context<RecommendPage>,
    radar: bool,
) -> impl IntoElement {
    let playlist_id = playlist.id;
    // 先固化成拥有值，避免 `move` 闭包捕获 `playlist` 的短生命周期引用。
    let summary = playlist.to_summary();
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        // 雷达和推荐歌单可能点到同一张歌单，id 前缀分开，避免交互状态串台。
        .id((
            if radar {
                "recommend-radar"
            } else {
                "recommend-playlist"
            },
            playlist_id,
        ))
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
                    .object_fit(ObjectFit::Cover)
                    .rounded(COVER_RADIUS),
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
                .pt(px(5.))
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(px(18.))
                .line_clamp(2)
                .text_color(colors.secondary_foreground)
                .child(playlist.name.clone()),
        )
        .on_click(cx.listener(move |_, _, _, cx| {
            NavigationBus::emit(
                cx,
                Navigate::Playlist {
                    id: playlist_id,
                    // 卡片上歌单名 / 封面 / 播放量已知，先作为摘要交详情页画页头。
                    summary: Some(summary.clone()),
                },
            );
        }))
}

/// 「精选活动」区块：横版运营大图，标题压在封面底部（官方同款渐变托底）。
/// 官方一次给十几张，这里按现有版式只铺前 3 张。
fn featured_activities(colors: ColorTokens, activities: &[FeaturedActivity]) -> impl IntoElement {
    let mut row = div().flex().gap(px(CARD_GAP));
    for activity in activities.iter().take(3) {
        // 运营大图是横幅，不能走封面的正方形 `param=800y800` 裁剪，否则两侧的文字
        // 会被裁掉；直接用原图，交给 `object_fit(Cover)` 按卡片比例裁。
        let image = activity
            .image_url
            .as_deref()
            .map(|url| url.replacen("http://", "https://", 1))
            .unwrap_or_else(|| PLACEHOLDER_COVER.to_owned());
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
                    img(image)
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .rounded(COVER_RADIUS),
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
                        .child(activity.title.clone()),
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
    use super::{columns_for, px};

    /// 实际显示列数：宽度算出来后再封顶到入口数量（入口数由 store 决定）。
    fn shown_columns(width: f32, entries: usize) -> usize {
        columns_for(px(width)).min(entries)
    }

    /// 窗口越宽列数越多，且不超过入口数量；极窄也至少有一列。
    #[test]
    fn columns_grow_with_width_and_never_exceed_entry_count() {
        let entries = 7;
        assert!(shown_columns(1300., entries) > shown_columns(700., entries));
        assert!(shown_columns(5000., entries) <= entries);
        assert!(shown_columns(100., entries) >= 1);
    }
}
