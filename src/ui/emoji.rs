//! 将评论中的网易云表情标记转换为 Unicode，沿用文本的排版、选中与复制行为。

use std::borrow::Cow;

// 资源编号供本地表情采集脚本读取，渲染只使用 Unicode 字符。
const EMOJIS: &[(&str, u32, &str)] = &[
    ("大笑", 86, "😃"),
    ("可爱", 85, "😊"),
    ("憨笑", 359, "☺️"),
    ("色", 95, "😍"),
    ("亲亲", 363, "😘"),
    ("惊恐", 96, "😱"),
    ("流泪", 356, "😭"),
    ("亲", 362, "😚"),
    ("呆", 352, "😳"),
    ("哀伤", 342, "😔"),
    ("呲牙", 343, "😁"),
    ("吐舌", 348, "😝"),
    ("撇嘴", 353, "😒"),
    ("怒", 361, "😡"),
    ("奸笑", 341, "😏"),
    ("汗", 97, "😓"),
    ("痛苦", 346, "😖"),
    ("惶恐", 354, "😰"),
    ("生病", 350, "😨"),
    ("口罩", 351, "😷"),
    ("大哭", 357, "😂"),
    ("晕", 355, "😵"),
    ("发怒", 115, "👿"),
    ("开心", 360, "😄"),
    ("鬼脸", 94, "😜"),
    ("皱眉", 87, "😞"),
    // 名字写成「流感」，图其实是皱眉 + 单滴泪，即 😢；以官方图为准，不按名字猜。
    ("流感", 358, "😢"),
    ("爱心", 33, "❤️"),
    ("心碎", 34, "💔"),
    ("钟情", 303, "💘"),
    ("星星", 309, "⭐"),
    ("生气", 314, "💢"),
    ("便便", 89, "💩"),
    ("强", 13, "👍"),
    ("弱", 372, "👎"),
    ("拜", 14, "🙏"),
    ("牵手", 379, "👫"),
    ("跳舞", 380, "👯"),
    ("禁止", 374, "🙅"),
    ("这边", 262, "👉"),
    ("爱意", 106, "💏"),
    ("示爱", 376, "💑"),
    ("嘴唇", 367, "👄"),
    ("狗", 81, "🐶"),
    ("猫", 78, "🐱"),
    ("猪", 100, "🐷"),
    ("兔子", 459, "🐰"),
    ("小鸡", 450, "🐤"),
    ("公鸡", 461, "🐔"),
    ("幽灵", 116, "👻"),
    ("圣诞", 411, "🎅"),
    ("外星", 101, "👽"),
    ("钻石", 52, "💎"),
    ("礼物", 107, "🎁"),
    ("男孩", 0, "👦"),
    ("女孩", 1, "👧"),
    ("蛋糕", 337, "🎂"),
    ("18", 186, "🔞"),
    ("圈", 312, "⭕"),
    ("叉", 313, "❌"),
];

pub fn expand(text: &str) -> Cow<'_, str> {
    let mut expanded = String::new();
    let mut start = 0;
    for (open, _) in text.match_indices('[') {
        if open < start {
            continue;
        }
        let Some(offset) = text[open + 1..].find(']') else {
            break;
        };
        let close = open + 1 + offset;
        let name = &text[open + 1..close];
        if let Some((_, _, ch)) = EMOJIS.iter().find(|(known, _, _)| *known == name) {
            expanded.push_str(&text[start..open]);
            expanded.push_str(ch);
            start = close + 1;
        }
    }
    if start == 0 {
        Cow::Borrowed(text)
    } else {
        expanded.push_str(&text[start..]);
        Cow::Owned(expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_known_markers_and_preserves_surrounding_text() {
        for (text, expected) in [
            ("笑死[大笑]", "笑死😃"),
            ("[强][弱]", "👍👎"),
            ("开头[狗]中间[猫]结尾", "开头🐶中间🐱结尾"),
            ("[18]", "🔞"),
            ("[a]b[大笑]", "[a]b😃"),
            ("[未闭合[强]", "[未闭合👍"),
            ("[强][大笑", "👍[大笑"),
        ] {
            assert_eq!(expand(text), expected);
        }
    }

    #[test]
    fn borrows_unchanged_text() {
        for text in ["", "普通评论", "[不存在]", "[已删除] 这条", "[大笑"] {
            assert!(matches!(expand(text), Cow::Borrowed(value) if value == text));
        }
    }

    #[test]
    fn mapping_has_unique_names_and_nonempty_characters() {
        let names: std::collections::HashSet<_> = EMOJIS.iter().map(|entry| entry.0).collect();
        assert_eq!(names.len(), EMOJIS.len());
        for (name, _, ch) in EMOJIS {
            assert!(!ch.is_empty(), "{name} 没有对应字符");
            assert_eq!(expand(&format!("[{name}]")), *ch);
        }
    }
}
