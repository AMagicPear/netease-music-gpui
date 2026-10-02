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
