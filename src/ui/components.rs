use crate::models::AudioQualityLevel;
use gpui::{Hsla, InteractiveElement, MouseButton, div};
use gpui_kit::base::{ColorTokens, InteractiveElementExt};
use std::{cell::Cell, rc::Rc};

mod drag_preview;
mod player_bar;
mod popover;
mod progress_bar;
mod tab_bar;
mod virtual_table;
mod volume_control;

pub use drag_preview::ResizeDragPreview;
pub use player_bar::{OpenAlbumLyrics, PlayerBar};
pub use popover::Popover;
pub use tab_bar::{TabBar, TabChanged, TabItem};
pub use virtual_table::{
    CELL_PADDING, COLUMN_GAP, HEADER_HEIGHT, ROW_TEXT_SIZE, TableColumn, virtual_table,
};

// 浮层都用 deferred 绘制，数字大的画得更晚、盖在上面。
//
// 进度条在最底下；通用弹层要盖住进度条；音量气泡最小也最临时，
// 它从图标上方弹出时会和弹层重叠，所以压在最上面。
pub(super) const LAYER_PROGRESS_BAR: usize = 1;
pub(super) const LAYER_POPOVER: usize = 2;
pub(super) const LAYER_VOLUME_BALLOON: usize = 3;

/// 暗背景图标只稍微提亮，浅背景仍使用主题前景色。
pub(super) fn icon_hover_color(mut base: Hsla, colors: ColorTokens) -> Hsla {
    if colors.background.l >= 0.5 {
        return colors.foreground;
    }
    if base.a < 1. {
        base.a = (base.a + 0.08).min(1.);
    } else {
        base.l = (base.l + 0.03).min(1.);
    }
    base
}

/// 普通内容区的窗口拖拽，不创建 TitleBar 或窗口控制按钮。
pub(super) fn window_drag_area(id: &'static str) -> gpui::Stateful<gpui::Div> {
    let should_move = Rc::new(Cell::new(false));
    div()
        .id(id)
        .on_mouse_down_out({
            let should_move = should_move.clone();
            move |_, _, _| should_move.set(false)
        })
        .on_mouse_down(MouseButton::Left, {
            let should_move = should_move.clone();
            move |_, _, _| should_move.set(true)
        })
        .on_mouse_up(MouseButton::Left, {
            let should_move = should_move.clone();
            move |_, _, _| should_move.set(false)
        })
        .on_mouse_move(move |event, window, _| {
            if should_move.replace(false) && event.pressed_button == Some(MouseButton::Left) {
                window.start_window_move();
            }
        })
        .on_double_click(|_, window, _| {
            if cfg!(target_os = "macos") {
                window.titlebar_double_click();
            } else {
                window.zoom_window();
            }
        })
}

// 下面是跨页面共用的展示规则。同一个规则只在歌单列表和播放栏各写一遍很容易漂移
// （比如换了素材只改一处），所以集中在这里，由调用方决定怎么渲染。

/// 音质徽章的图标，素材与官方「音质选项」一一对应。
/// 标准 / 较高 / 极高 共用 HQ，其余档位各有专属素材。
pub(super) fn quality_badge_path(quality: AudioQualityLevel) -> &'static str {
    match quality {
        AudioQualityLevel::Standard | AudioQualityLevel::Higher | AudioQualityLevel::ExHigh => {
            "icons/音质选项/HQ.svg"
        }
        AudioQualityLevel::Lossless => "icons/音质选项/sq.svg",
        AudioQualityLevel::HiRes => "icons/音质选项/Hi-Res.svg",
        AudioQualityLevel::JyEffect => "icons/音质选项/高清臻音.svg",
        AudioQualityLevel::Sky => "icons/音质选项/沉浸声.svg",
        AudioQualityLevel::Dolby => "icons/音质选项/全景声.svg",
        AudioQualityLevel::JyMaster => "icons/音质选项/超清母带.svg",
    }
}

/// 喜欢状态的图标：已喜欢是实心红心，未喜欢是描线灰心。
/// 只定形状不定颜色——歌单行和播放栏的灰度和 hover 规则并不相同，颜色由调用方给。
pub(super) fn like_icon_path(liked: bool) -> &'static str {
    if liked {
        "icons/heart.svg"
    } else {
        "icons/heart_outline.svg"
    }
}

/// 歌曲时长和播放进度共用的 mm:ss 格式；超过一小时仍按分钟累计。
pub(super) fn format_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// 未知歌手单独变灰，多歌手中已有的名字保持原色。
pub(super) fn artist_label(
    song: &crate::models::Song,
    colors: gpui_kit::base::ColorTokens,
) -> gpui::StyledText {
    let mut text = String::new();
    let mut highlights = Vec::new();
    let names = song.ar.iter().map(|artist| artist.name.as_deref());
    // 空歌手列表也显示一个「未知歌手」。
    for name in names.chain(song.ar.is_empty().then_some(None)) {
        if !text.is_empty() {
            text.push_str(" / ");
        }
        let start = text.len();
        match name.filter(|name| !name.trim().is_empty()) {
            Some(name) => text.push_str(name),
            None => {
                text.push_str("未知歌手");
                highlights.push((
                    start..text.len(),
                    gpui::HighlightStyle {
                        color: Some(colors.foreground.alpha(1.)),
                        fade_out: Some(0.55),
                        ..Default::default()
                    },
                ));
            }
        }
    }
    gpui::StyledText::new(text).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::hsla;

    #[test]
    fn icon_hover_respects_background_and_caps_brightness() {
        let colors = ColorTokens {
            background: hsla(0., 0., 0.12, 1.),
            ..ColorTokens::default()
        };
        for (lightness, alpha, expected_lightness, expected_alpha) in [
            (1., 0.5, 1., 0.58),
            (1., 0.75, 1., 0.83),
            (1., 0.98, 1., 1.),
            (0.6, 1., 0.63, 1.),
            (0.99, 1., 1., 1.),
        ] {
            let hovered = icon_hover_color(hsla(0.2, 0.3, lightness, alpha), colors);
            assert!((hovered.l - expected_lightness).abs() < 1e-6);
            assert!((hovered.a - expected_alpha).abs() < 1e-6);
            assert_eq!((hovered.h, hovered.s), (0.2, 0.3));
        }
        let light_colors = ColorTokens {
            background: hsla(0., 0., 0.5, 1.),
            ..colors
        };
        assert_eq!(
            icon_hover_color(hsla(0., 0., 1., 0.5), light_colors),
            light_colors.foreground,
        );
    }
}
