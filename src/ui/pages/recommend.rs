use std::time::Duration;

use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use crate::models::{FeaturedActivity, Playlist, RecommendedPlaylist};
use crate::state::daily::DailySongsStore;
use crate::state::library::MusicLibrary;
use crate::state::recommend::{QuickEntry, QuickEntryKind, RecommendStore};
use crate::ui::assets::{track_cover_image, track_cover_url};
use crate::ui::components::{centered_empty, play_count_label, spinner};
use crate::ui::cover_color::{CoverColor, dark_gradient};
use crate::ui::events::{Navigate, NavigationBus};
use crate::ui::theme::{DOLPHIN_FAMILY, PRESSED_ICON_ALPHA};

const SECTION_GAP: Pixels = px(28.);
const CARD_GAP: f32 = 16.;
const COVER_RADIUS: Pixels = px(8.);
const SPIN_PERIOD: Duration = Duration::from_millis(900);
const CARD_TEXT_HEIGHT: Pixels = px(48.);
const TARGET_CARD_WIDTH: f32 = 135.;
const PLACEHOLDER_COVER: &str = "images/trackBlank.png";

/// shell 通过 content 注入可用宽度，所有区块共用同一列数。
pub struct RecommendPage {
    store: Entity<RecommendStore>,
    _store_subscription: Subscription,
    _daily_subscription: Subscription,
    daily: Entity<DailySongsStore>,
    library: Entity<MusicLibrary>,
    /// 与 entries 按下标对齐，新封面加载中保留上一张的颜色。
    entry_colors: Vec<CoverColor>,
}

impl RecommendPage {
    pub fn new(
        store: Entity<RecommendStore>,
        daily: Entity<DailySongsStore>,
        library: Entity<MusicLibrary>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&store, |_, _, cx| cx.notify());
        let daily_subscription = cx.observe(&daily, |_, _, cx| cx.notify());
        Self {
            store,
            _store_subscription: subscription,
            _daily_subscription: daily_subscription,
            daily,
            library,
            entry_colors: Vec::new(),
        }
    }

    /// 切回本页时静默刷新一次：离开期间推荐内容可能变了。
    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.revalidate(cx));
        self.daily.update(cx, |daily, cx| daily.revalidate(cx));
    }

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

    fn entry_covers(&self, cx: &Context<Self>) -> Vec<SharedString> {
        let store = self.store.read(cx);
        let daily_cover = self
            .daily
            .read(cx)
            .songs()
            .first()
            .and_then(|song| song.al.pic_url.as_deref());
        store
            .entries
            .value()
            .map(|entries| entries.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|entry| match entry.kind {
                QuickEntryKind::DailySongs if daily_cover.is_some() => {
                    track_cover_url(daily_cover, 300).into()
                }
                QuickEntryKind::HeartMode => {
                    // 心动模式的封面就是「我喜欢的音乐」那张歌单封面，
                    // 音乐库里已经有了，不必为它单独请求。
                    let cover = self
                        .library
                        .read(cx)
                        .favorite_playlist()
                        .and_then(|playlist| playlist.cover_img_url.as_deref());
                    cover.map_or_else(
                        || PLACEHOLDER_COVER.into(),
                        |cover| track_cover_url(Some(cover), 300).into(),
                    )
                }
                QuickEntryKind::Radar
                    if store
                        .radar_first_song_cover
                        .value()
                        .is_some_and(Option::is_some) =>
                {
                    track_cover_url(
                        store
                            .radar_first_song_cover
                            .value()
                            .and_then(|cover| cover.as_deref()),
                        300,
                    )
                    .into()
                }
                _ => match &entry.cover {
                    Some(cover) => track_cover_url(Some(cover), 300).into(),
                    None => PLACEHOLDER_COVER.into(),
                },
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
        let today = time::OffsetDateTime::now_local()
            .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
            .day();
        let store = self.store.read(cx);
        let mut entries = store.entries.value().cloned().unwrap_or_default();
        for entry in &mut entries {
            if entry.kind == QuickEntryKind::HeartMode {
                // 推荐流是点开时现取的，卡片上预告不了第一首，
                // 只说明它从哪儿起播。
                entry.caption = if self.library.read(cx).user_id == 0 {
                    "登录后从你喜欢的歌听起".into()
                } else {
                    "你的红心歌曲和更多相似推荐".into()
                };
            }
            if entry.kind == QuickEntryKind::DailySongs {
                entry.caption = self.daily.read(cx).songs().first().map_or_else(
                    || "每日推荐".into(),
                    |song| format!("每日推荐 | 从「{}」听起", song.name).into(),
                );
            }
        }
        let natural_columns = columns_for(available_width);
        let columns = if entries.is_empty() {
            natural_columns
        } else {
            natural_columns.min(entries.len())
        };
        let mut page = div().w_full().flex().flex_col().gap(SECTION_GAP);
        if !entries.is_empty() {
            page = page.child(quick_entries(
                today,
                columns,
                &entries,
                &self.entry_colors,
                covers,
                store.radar.value().and_then(|radar| radar.first()),
                cx,
            ));
        }
        page = page.child(self.recommended_section(columns, colors, cx));
        let radar = store
            .radar
            .value()
            .map(|radar| radar.as_slice())
            .unwrap_or(&[]);
        if !radar.is_empty() {
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(section_title("雷达歌单", colors))
                    .child(playlist_row(radar, columns, colors, "recommend-radar", cx)),
            );
        }
        let featured = store
            .featured
            .value()
            .map(|featured| featured.as_slice())
            .unwrap_or(&[]);
        if !featured.is_empty() {
            page = page.child(featured_activities(
                colors,
                featured,
                svg_font_family(&Theme::global(cx).tokens.typography.sans),
            ));
        }
        page.into_any_element()
    }
}

fn columns_for(available_width: Pixels) -> usize {
    let per_column = TARGET_CARD_WIDTH + CARD_GAP;
    ((f32::from(available_width) + CARD_GAP) / per_column)
        .floor()
        .max(1.) as usize
}

fn section_title(text: &'static str, colors: ColorTokens) -> impl IntoElement {
    div()
        .text_size(px(16.))
        .font_weight(FontWeight::BOLD)
        .text_color(colors.foreground)
        .child(text)
}

/// 第一排里真正能点进去的入口。私人雷达的落点就是雷达区块的首个歌单，
/// 两个区块的卡片指向同一份数据，不在页面里另存一份。
enum QuickTarget {
    DailySongs,
    HeartMode,
    /// 摘要用于先画歌单页头；`Playlist` 很大，这里只影响展示，装箱避免拉大整个枚举。
    Playlist {
        id: u64,
        summary: Box<Playlist>,
    },
}

/// 卡片平分一行；窗口较窄时只显示前 columns 张。
fn quick_entries(
    today: u8,
    columns: usize,
    entries: &[QuickEntry],
    entry_colors: &[CoverColor],
    covers: &[SharedString],
    radar: Option<&RecommendedPlaylist>,
    cx: &Context<RecommendPage>,
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
                    let strip =
                        dark_gradient(entry_colors.get(index).and_then(CoverColor::current)).0[0]
                            .into();
                    let cover = covers
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| PLACEHOLDER_COVER.into());
                    let target = match entry.kind {
                        QuickEntryKind::DailySongs => Some(QuickTarget::DailySongs),
                        QuickEntryKind::HeartMode => Some(QuickTarget::HeartMode),
                        QuickEntryKind::Radar => radar.map(|playlist| QuickTarget::Playlist {
                            id: playlist.id,
                            summary: Box::new(playlist.to_summary()),
                        }),
                        _ => None,
                    };
                    quick_entry(entry, today, strip, cover, index, target, cx)
                }),
        )
}

fn quick_entry(
    entry: &QuickEntry,
    today: u8,
    strip: Hsla,
    cover: SharedString,
    index: usize,
    target: Option<QuickTarget>,
    cx: &Context<RecommendPage>,
) -> impl IntoElement {
    let card = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .id(("recommend-entry", index))
        .child(
            // overflow_hidden 只按矩形裁剪，顶角和底角需分别设置在子元素上。
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
                        .aspect_ratio(1.)
                        .relative()
                        .child(
                            img(cover)
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .rounded_t(COVER_RADIUS),
                        )
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded_t(COVER_RADIUS)
                                .bg(cover_shade()),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(6.))
                                .left(px(8.))
                                .right(px(8.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
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
        );
    match target {
        Some(target) => card
            .role(Role::Button)
            .aria_label(entry.title.clone())
            .cursor_pointer()
            .on_click(cx.listener(move |_, _, _, cx| match &target {
                QuickTarget::DailySongs => NavigationBus::emit(cx, Navigate::DailySongs),
                // 心动模式交给上层统一发起：卡片只提交意图。
                QuickTarget::HeartMode => NavigationBus::emit(cx, Navigate::HeartMode),
                QuickTarget::Playlist { id, summary } => NavigationBus::emit(
                    cx,
                    Navigate::Playlist {
                        id: *id,
                        summary: Some(summary.as_ref().clone()),
                    },
                ),
            })),
        None => card,
    }
}

/// 在编译期嵌入的日历遮罩中插入当天数字，数字处透明透出封面。
const CALENDAR_BADGE_SVG: &str = include_str!("../../../assets/icons/calendar.svg");
const CALENDAR_DATE_SLOT: &str = "<!--DATE_TEXT-->";
const ENTRY_BADGE_SIZE: f32 = 22.;
/// 数字的视觉中心：图标上半有一条横向镂空的装订带（y 5.3~8.8），
/// 剩下那块白色空白区是 y 8.8~19.6，中心即 14.2。
const CALENDAR_DIGIT_CENTER_Y: f32 = 14.2;
const CALENDAR_DIGIT_SIZE_ONE: f32 = 11.5;
const CALENDAR_DIGIT_SIZE_TWO: f32 = 9.5;

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
        .data(source.as_bytes())
        .size(px(ENTRY_BADGE_SIZE))
        .flex_none()
        .text_color(white())
        .into_any_element()
}

fn roam_badge() -> AnyElement {
    svg()
        .path("icons/radio_in_recommend.svg")
        .size(px(ENTRY_BADGE_SIZE))
        .flex_none()
        .text_color(white())
        .into_any_element()
}

impl RecommendPage {
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

    fn refresh_button(&self, colors: ColorTokens, cx: &Context<Self>) -> AnyElement {
        let loading = self.store.read(cx).playlists.is_loading();
        if loading {
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
        let playlists = store
            .playlists
            .value()
            .map(|list| list.as_slice())
            .unwrap_or(&[]);
        if store.playlists.is_loading() && playlists.is_empty() {
            return div()
                .w_full()
                .py(px(36.))
                .flex()
                .justify_center()
                .child(spinner("recommend-loading", 20., colors.muted_foreground))
                .into_any_element();
        }
        if playlists.is_empty() && store.playlists.value().is_none() {
            return match store.playlists.error() {
                Some(_) => div()
                    .w_full()
                    .py(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .text_size(px(14.))
                    .text_color(colors.muted_foreground)
                    .child("推荐加载失败，请重试")
                    .child(
                        Button::new("recommend-retry")
                            .child("重试")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |store, cx| store.reload(cx));
                            })),
                    )
                    .into_any_element(),
                None => {
                    centered_empty("images/empty/cm_empty_wuzuopin.png", "暂无推荐歌单", colors)
                        .into_any_element()
                }
            };
        }
        playlist_row(playlists, columns, colors, "recommend-playlist", cx).into_any_element()
    }
}

fn playlist_row(
    playlists: &[RecommendedPlaylist],
    columns: usize,
    colors: ColorTokens,
    id_prefix: &'static str,
    cx: &Context<RecommendPage>,
) -> impl IntoElement {
    div().flex().gap(px(CARD_GAP)).children(
        playlists
            .iter()
            .take(columns)
            .map(|playlist| recommended_card(playlist, colors, id_prefix, cx)),
    )
}

fn cover_shade() -> Background {
    linear_gradient(
        180.,
        linear_color_stop(black().alpha(0.2), 0.),
        linear_color_stop(black().alpha(0.), 0.35),
    )
}

fn recommended_card(
    playlist: &RecommendedPlaylist,
    colors: ColorTokens,
    id_prefix: &'static str,
    cx: &Context<RecommendPage>,
) -> impl IntoElement {
    let playlist_id = playlist.id;
    let summary = playlist.to_summary();
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        // 同一歌单可出现在两个区块，交互状态按区块隔离。
        .id((id_prefix, playlist_id))
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
                    img(track_cover_image(track_cover_url(
                        playlist.pic_url.as_deref(),
                        300,
                    )))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded(COVER_RADIUS),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(COVER_RADIUS)
                        .bg(cover_shade()),
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
                    summary: Some(summary.clone()),
                },
            );
        }))
}

fn featured_activities(
    colors: ColorTokens,
    activities: &[FeaturedActivity],
    font_family: &str,
) -> impl IntoElement {
    let mut row = div().flex().gap(px(CARD_GAP));
    for activity in activities.iter().take(3) {
        // 横幅直接使用原图，避免 CDN 的正方形裁剪截掉两侧文字。
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
                        .right(px(5.))
                        .bottom(px(5.))
                        .child(activity_label(&activity.title, font_family)),
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

fn activity_label(title: &str, font_family: &str) -> AnyElement {
    let escape_xml = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    };
    let title = escape_xml(title);
    let font_family = escape_xml(font_family);
    let width = title.chars().count() as f32 * 10. + 6.;
    let source = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="16" viewBox="0 0 {width} 16"><defs><mask id="title-cut"><rect width="100%" height="100%" fill="white"/><text x="50%" y="11.5" text-anchor="middle" font-family="{font_family}" font-size="10" font-weight="600" fill="black">{title}</text></mask></defs><rect x="0" y="0" width="100%" height="100%" rx="4" fill="white" fill-opacity="0.902" mask="url(#title-cut)"/></svg>"##
    );
    svg()
        .data(source.as_bytes())
        .w(px(width))
        .h(px(16.))
        .text_color(white())
        .into_any_element()
}

fn svg_font_family(global: &str) -> &str {
    if global != ".SystemUIFont" {
        return global;
    }
    if cfg!(target_os = "macos") {
        "PingFang SC"
    } else {
        "sans-serif"
    }
}
