//! 歌曲列表共用的行内容、标题高亮和操作按钮。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens};

use super::virtual_table::ROW_TEXT_SIZE;
use super::{empty_state, format_duration};
use crate::models::Song;
use crate::ui::assets::track_cover_url;
use crate::ui::theme::{PRESSED_ICON_ALPHA, PRESSED_OPACITY};

pub const SECONDARY_FONT_WEIGHT: FontWeight = FontWeight::NORMAL;

pub const ACTION_BUTTON_HEIGHT: Pixels = px(36.);
pub const ACTION_BUTTON_PADDING: Pixels = px(12.);

pub const LIKE_COLUMN_WIDTH: Pixels = px(42.);
pub const DURATION_COLUMN_WIDTH: Pixels = px(66.);

/// Dolphin 里最宽的数字是 600/1000 em。
const DOLPHIN_WIDEST_DIGIT_EM: f32 = 0.6;

/// 表头与行共用列宽，至少容纳最长序号和播放按钮。
pub fn index_column_width(row_count: usize) -> Pixels {
    let digits = format!("{:02}", row_count).chars().count() as f32;
    let width = digits * DOLPHIN_WIDEST_DIGIT_EM * f32::from(ROW_TEXT_SIZE);
    // 向上取整，避免像素舍入导致数字折行。
    px(width.ceil().max(20.))
}

/// 离开事件只清除自己，避免晚到的 A 离开事件清掉 B 的 hover。
pub fn hovered_row_after(current: Option<u64>, id: u64, hovered: bool) -> Option<u64> {
    if hovered {
        Some(id)
    } else if current == Some(id) {
        None
    } else {
        current
    }
}

/// 一首歌在列表行里要显示的内容。构造一次、行内多次复用，避免每帧重算。
pub struct SongDisplay {
    pub cover: Option<ImageSource>,
    pub title: SharedString,
    pub subtitle: std::ops::Range<usize>,
    pub album: SharedString,
    pub duration: SharedString,
}

impl SongDisplay {
    pub fn new(song: &Song) -> Self {
        let mut title = song.name.clone();
        let start = title.len();
        if let Some(subtitle) = song.tns.first().or_else(|| song.alia.first()) {
            title.push_str(&format!("（{subtitle}）"));
        }
        let end = title.len();
        Self {
            cover: Some(track_cover_url(song.al.pic_url.as_deref(), 72).into()),
            title: title.into(),
            subtitle: start..end,
            album: song
                .al
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("未知专辑")
                .to_owned()
                .into(),
            duration: format_duration(song.duration()).into(),
        }
    }
}

// 先替换 RGB 再恢复 alpha，避免播放中的红色标题污染副标题。
fn subtitle_style(colors: ColorTokens) -> HighlightStyle {
    let subtitle_color = colors.muted_foreground;
    HighlightStyle {
        color: Some(subtitle_color.alpha(1.)),
        font_weight: Some(SECONDARY_FONT_WEIGHT),
        fade_out: Some(1. - subtitle_color.a),
        ..Default::default()
    }
}

// GPUI 会混合重叠的高亮色，因此先按命中切开副标题，生成互不重叠的区间。
fn title_segments(
    title: &str,
    subtitle: &std::ops::Range<usize>,
    keyword: &str,
    colors: ColorTokens,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let subtitle_style = subtitle_style(colors);
    let hit_style = super::match_highlight();

    let mut segments: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    let mut cursor = subtitle.start;
    for range in super::keyword_ranges(title, keyword) {
        if range.end <= subtitle.start {
            segments.push((range, hit_style));
            continue;
        }
        let from = range.start.max(subtitle.start);
        let to = range.end.min(subtitle.end);
        if range.start < subtitle.start {
            segments.push((range.start..subtitle.start, hit_style));
        }
        if cursor < from {
            segments.push((cursor..from, subtitle_style));
        }
        segments.push((from..to, hit_style));
        cursor = to;
    }
    if cursor < subtitle.end {
        segments.push((cursor..subtitle.end, subtitle_style));
    }
    segments.sort_by_key(|(range, _)| range.start);
    segments
}

/// 使用同一段 StyledText，标题和副标题一起截断；颜色随当前主题计算。
/// `keyword` 非空时标出命中片段（搜索结果里传真实关键词，其余地方传空串）。
pub fn title_with_subtitle(
    display: &SongDisplay,
    keyword: &str,
    colors: ColorTokens,
) -> StyledText {
    let text = StyledText::new(display.title.clone());
    let segments = title_segments(&display.title, &display.subtitle, keyword, colors);
    if segments.is_empty() {
        text
    } else {
        text.with_highlights(segments)
    }
}

/// 阻止按下冒泡到整行，避免 GPUI 在按下期间取消行 hover、隐藏操作图标。
pub trait KeepRowHover: Sized {
    fn keep_row_hover(self) -> Self;
}

impl<E: InteractiveElement + Sized> KeepRowHover for E {
    fn keep_row_hover(self) -> Self {
        self.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }
}

/// 行 hover 时才出现的图标：序号位置的播放键、标题右侧那排操作按钮，共用这一套样式。
pub fn row_hover_icon(
    id: (&'static str, u64),
    path: &'static str,
    label: &'static str,
    size: Pixels,
    colors: ColorTokens,
) -> AnyElement {
    svg()
        .path(path)
        .size(size)
        .flex_none()
        .text_color(colors.foreground.alpha(0.6))
        .hover(|style| style.text_color(colors.foreground))
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
        .into_any_element()
}

/// 播放量：十万以下照写，十万起折算成万，万位满四位不再带小数。
pub fn play_count_label(count: u64) -> String {
    const WAN: u64 = 10_000;
    if count < 10 * WAN {
        return count.to_string();
    }
    let wan = count / WAN;
    let tenth = count % WAN / 1_000;
    if wan >= 1_000 || tenth == 0 {
        format!("{wan}万")
    } else {
        format!("{wan}.{tenth}万")
    }
}

/// 「播放全部」：主题红实心按钮，白字白图标，无边框。
pub fn play_all_button(colors: ColorTokens, compact: bool) -> Button {
    Button::new(if compact {
        "playlist-floating-play"
    } else {
        "playlist-play-all-button"
    })
    .h(ACTION_BUTTON_HEIGHT)
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .gap(px(4.))
    .px(ACTION_BUTTON_PADDING)
    .when(compact, |button| button.w(ACTION_BUTTON_HEIGHT).px(px(0.)))
    .rounded_lg()
    .bg(colors.primary)
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
    .aria_label("播放全部")
    .when(!compact, |button| button.child("播放全部"))
}

/// 空状态在列表区水平居中，顶部对齐（不另加边距）。
pub fn centered_empty(image: &'static str, tip: &str, colors: ColorTokens) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .child(empty_state(image, tip, colors))
}

#[cfg(test)]
mod tests {
    use gpui::{Hsla, rgb};
    use gpui_kit::base::ColorTokens;

    use super::{SongDisplay, hovered_row_after, play_count_label, title_segments};
    use crate::models::Song;
    use crate::ui::theme::link_blue;

    #[test]
    fn title_segments_split_the_subtitle_around_hits() {
        let colors = ColorTokens {
            foreground: rgb(0x283248).into(),
            muted_foreground: Hsla {
                a: 0.6,
                ..Default::default()
            },
            ..Default::default()
        };
        let display = SongDisplay::new(&song_with_subtitle("稻香", "daoxiang"));
        let subtitle = display.subtitle.clone();
        assert_eq!(&display.title, "稻香（daoxiang）");
        assert_eq!(subtitle, 6..20);
        let shape = |keyword: &str| {
            title_segments(&display.title, &subtitle, keyword, colors)
                .into_iter()
                .map(|(range, style)| (range, style.color == Some(link_blue())))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shape("xiang"),
            vec![(6..12, false), (12..17, true), (17..20, false)]
        );
        assert_eq!(shape("稻"), vec![(0..3, true), (6..20, false)]);
        assert_eq!(shape(""), vec![(6..20, false)]);
        assert_eq!(
            shape("香（"),
            vec![(3..6, true), (6..9, true), (9..20, false)]
        );
        for keyword in ["xiang", "稻", "", "g", "稻香", "dao", "a", "香", "（", "）"] {
            let segments = shape(keyword);
            for pair in segments.windows(2) {
                assert!(
                    pair[0].0.end <= pair[1].0.start,
                    "区间重叠或乱序: {pair:?}（keyword={keyword:?}）"
                );
            }
            for (range, _) in &segments {
                assert!(display.title.is_char_boundary(range.start), "{keyword:?}");
                assert!(display.title.is_char_boundary(range.end), "{keyword:?}");
            }
        }
    }

    #[test]
    fn leaving_a_row_only_clears_that_row() {
        assert_eq!(hovered_row_after(Some(1), 2, true), Some(2));
        assert_eq!(hovered_row_after(Some(2), 1, false), Some(2));
        assert_eq!(hovered_row_after(Some(3), 9, false), Some(3));
        assert_eq!(hovered_row_after(None, 7, true), Some(7));
        assert_eq!(hovered_row_after(Some(7), 7, false), None);
        assert_eq!(hovered_row_after(None, 1, false), None);
    }

    #[test]
    fn play_count_switches_to_wan_and_truncates_rather_than_rounds() {
        assert_eq!(play_count_label(0), "0");
        assert_eq!(play_count_label(99_999), "99999");
        assert_eq!(play_count_label(100_000), "10万");
        assert_eq!(play_count_label(812_568), "81.2万");
        assert_eq!(play_count_label(150_000), "15万");
        assert_eq!(play_count_label(12_345_678), "1234万");
    }

    fn song_with_subtitle(name: &str, tns: &str) -> Song {
        serde_json::from_value(serde_json::json!({
            "id": 1,
            "name": name,
            "ar": [{"id": 1, "name": "周杰伦"}],
            "al": {"id": 1, "name": "叶惠美"},
            "dt": 0,
            "tns": [tns],
        }))
        .unwrap()
    }
}
