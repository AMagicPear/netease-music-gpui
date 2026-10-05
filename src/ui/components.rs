mod drag_preview;
mod player_bar;
mod progress_bar;
mod tab_bar;
mod virtual_table;

pub use drag_preview::ResizeDragPreview;
pub use player_bar::PlayerBar;
pub use tab_bar::{TabBar, TabChanged, TabItem};
pub use virtual_table::{
    CELL_PADDING, COLUMN_GAP, HEADER_HEIGHT, ROW_TEXT_SIZE, TableColumn, virtual_table,
};

// 下面是跨页面共用的展示规则。同一个规则只在歌单列表和播放栏各写一遍很容易漂移
// （比如换了素材只改一处），所以集中在这里，由调用方决定怎么渲染。

/// 音质徽章的图标。五个档位共用三套素材：标准 / 较高 / 极高 都是 HQ。
pub(super) fn quality_badge_path(quality: crate::models::AudioQualityLevel) -> &'static str {
    match quality {
        crate::models::AudioQualityLevel::HiRes => "icons/音质选项/Hi-Res.svg",
        crate::models::AudioQualityLevel::Lossless => "icons/音质选项/sq.svg",
        _ => "icons/音质选项/HQ.svg",
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
