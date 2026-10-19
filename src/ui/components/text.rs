use std::ops::Range;

use gpui::SharedString;

use crate::ui::theme::link_blue;

/// 过滤与高亮共用大小写不敏感的匹配，返回原文的 UTF-8 字节区间。
pub(in crate::ui) fn keyword_ranges(text: &str, keyword: &str) -> Vec<Range<usize>> {
    let keyword = keyword.trim();
    if keyword.is_empty() || text.is_empty() {
        return Vec::new();
    }
    let mut lowered = String::with_capacity(text.len());
    // `origins[i]` = `lowered` 第 i 个字节所属原文字符的起始偏移，按字节补齐。
    let mut origins: Vec<usize> = Vec::with_capacity(text.len());
    for (index, ch) in text.char_indices() {
        for folded in ch.to_lowercase() {
            lowered.push(folded);
            origins.resize(origins.len() + folded.len_utf8(), index);
        }
    }
    lowered
        .match_indices(&keyword.to_lowercase())
        .map(|(start, matched)| {
            let from = origins[start];
            // 命中到末尾时没有「下一个字节」可查，退回原文长度。
            let to = origins
                .get(start + matched.len())
                .copied()
                .unwrap_or(text.len());
            if to > from {
                return from..to;
            }
            // 命中落在一个原文字符内部（小写化变长时区间塌成 `from == to`），
            // 标整个原文字符，别让这行「搜得到、却看不到高亮」。
            let width = text[from..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or_default();
            from..from + width
        })
        .collect()
}

pub(super) fn match_highlight() -> gpui::HighlightStyle {
    gpui::HighlightStyle {
        color: Some(link_blue()),
        ..Default::default()
    }
}

/// 拼接文本时，将局部命中区间平移到整串中的位置。
pub(in crate::ui) fn match_highlights(
    text: &str,
    offset: usize,
    keyword: &str,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let style = match_highlight();
    keyword_ranges(text, keyword)
        .into_iter()
        .map(|range| (range.start + offset..range.end + offset, style))
        .collect()
}

pub(in crate::ui) fn text_with_matches(text: SharedString, keyword: &str) -> gpui::StyledText {
    let hits = match_highlights(&text, 0, keyword);
    gpui::StyledText::new(text).with_highlights(hits)
}

#[cfg(test)]
mod tests {
    use super::keyword_ranges;

    #[test]
    fn keyword_ranges_point_at_original_offsets() {
        assert_eq!(keyword_ranges("稻香", "稻"), vec![0..3]);
        assert_eq!(keyword_ranges("稻香", "香"), vec![3..6]);
        assert_eq!(keyword_ranges("ababa", "aba"), vec![0..3]);
        assert_eq!(keyword_ranges("哈哈哈", "哈"), vec![0..3, 3..6, 6..9]);
        assert_eq!(keyword_ranges("aXbXc", "X"), vec![1..2, 3..4]);
        assert_eq!(keyword_ranges("Love Song", "ve S"), vec![2..6]);
        assert!(keyword_ranges("稻香", "").is_empty());
        assert!(keyword_ranges("稻香", "   ").is_empty());
        assert!(keyword_ranges("", "稻").is_empty());
        assert!(keyword_ranges("稻香", "不存在").is_empty());
    }

    #[test]
    fn keyword_ranges_are_case_insensitive_but_stay_on_original_bytes() {
        assert_eq!(
            keyword_ranges("Love love LOVE", "love"),
            vec![0..4, 5..9, 10..14]
        );
        assert_eq!(keyword_ranges("Sunny Day", "sunny"), vec![0..5]);
        assert_eq!(keyword_ranges("SUNNY DAY", "sunny"), vec![0..5]);
        assert_eq!(keyword_ranges("Taylor Swift", "taylor"), vec![0..6]);
        assert_eq!(keyword_ranges("ÄBC", "äbc"), vec![0..4]);
        assert_eq!(keyword_ranges("İstanbul", "i"), vec![0..2]);
        let text = "Ωİstanbul Straße";
        for range in keyword_ranges(text, "i") {
            assert!(text.is_char_boundary(range.start));
            assert!(text.is_char_boundary(range.end));
        }
    }

    #[test]
    fn keyword_ranges_survive_multibyte_text_with_a_non_literal_keyword() {
        let title = "稻香（daoxiang）";
        let ranges = keyword_ranges(title, "DAO");
        assert_eq!(ranges, vec![9..12]);
        assert!(title.is_char_boundary(ranges[0].start));
        assert!(title.is_char_boundary(ranges[0].end));
        assert_eq!(keyword_ranges("İstanbul（live）", "LIVE"), vec![12..16]);
        assert_eq!(keyword_ranges("（live）稻香", "LIVE"), vec![3..7]);
        assert_eq!(keyword_ranges("Sunny Day", "DAY"), vec![6..9]);
    }
}
