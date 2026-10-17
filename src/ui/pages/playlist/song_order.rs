//! 歌单内的本地搜索匹配与排序。

use crate::models::Song;
use crate::ui::components::keyword_ranges;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SongSort {
    Title,
    Artist,
    Album,
}

// 标题循环：默认 → 标题升序 → 标题降序 → 歌手升序 → 歌手降序。
pub(super) fn next_sort(
    current: Option<(SongSort, bool)>,
    column: SongSort,
) -> Option<(SongSort, bool)> {
    match (column, current) {
        (SongSort::Title, Some((SongSort::Title, false))) => Some((SongSort::Title, true)),
        (SongSort::Title, Some((SongSort::Title, true))) => Some((SongSort::Artist, false)),
        (SongSort::Title, Some((SongSort::Artist, false))) => Some((SongSort::Artist, true)),
        (SongSort::Title, Some((SongSort::Artist, true))) => None,
        (SongSort::Album, Some((SongSort::Album, false))) => Some((SongSort::Album, true)),
        (SongSort::Album, Some((SongSort::Album, true))) => None,
        _ => Some((column, false)),
    }
}

pub(super) fn sort_label(
    current: Option<(SongSort, bool)>,
    column: SongSort,
) -> (&'static str, &'static str) {
    let label = match (column, current) {
        (SongSort::Title, Some((SongSort::Title, false))) => "标题升序",
        (SongSort::Title, Some((SongSort::Title, true))) => "标题降序",
        (SongSort::Title, Some((SongSort::Artist, false))) => "歌手升序",
        (SongSort::Title, Some((SongSort::Artist, true))) => "歌手降序",
        (SongSort::Album, Some((SongSort::Album, false))) => "升序",
        (SongSort::Album, Some((SongSort::Album, true))) => "降序",
        _ => "默认排序",
    };
    let icon = if label.ends_with("升序") {
        "icons/列表排序/升序上箭头.svg"
    } else if label.ends_with("降序") {
        "icons/列表排序/降序下箭头.svg"
    } else {
        "icons/列表排序/默认排序.svg"
    };
    (icon, label)
}

/// 关键词命中：标题、任一歌手、专辑任一包含即算命中。
///
/// 判定直接复用 `keyword_ranges`，所以「过滤」和「高亮」是同一套匹配，
/// 不会出现某行搜出来了、高亮却找不到位置的情况。搜的是子串不是前缀，
/// 所以「周」能命中歌名中间带「周」字的那首。
///
/// `tns`（音译）和 `alia`（别名）也算：它们显示成标题的副标题，
/// 用户看得到就一定会搜它 —— 用「daoxiang」搜「稻香」应该找得到。
pub(super) fn song_matches(song: &Song, keyword: &str) -> bool {
    let hit = |text: &str| !keyword_ranges(text, keyword).is_empty();
    hit(&song.name)
        || song.tns.iter().any(|t| hit(t))
        || song.alia.iter().any(|a| hit(a))
        || song
            .ar
            .iter()
            .any(|artist| artist.name.as_deref().is_some_and(hit))
        || song.al.name.as_deref().is_some_and(hit)
}

/// 把给定的一批下标按列排序。
///
/// 过滤与排序拆成两步：过滤先决定「哪些行出现」，排序只在这批下标内部重排，
/// 所以 `detail.songs` 的原始顺序始终不动，搜索结果也不会因为换排序而变多变少。
pub(super) fn sort_indices(
    mut order: Vec<usize>,
    songs: &[Song],
    column: SongSort,
    descending: bool,
) -> Vec<usize> {
    // shortcut: 按 Unicode 字符排序；需要拼音或语言区域排序时再接入 collation。
    order.sort_by(|&left, &right| {
        let ordering = match column {
            SongSort::Title => songs[left].name.cmp(&songs[right].name),
            SongSort::Artist => songs[left]
                .ar
                .iter()
                .map(|artist| &artist.name)
                .cmp(songs[right].ar.iter().map(|artist| &artist.name)),
            SongSort::Album => songs[left].al.name.cmp(&songs[right].al.name),
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    order
}

#[cfg(test)]
mod tests {
    use super::{Song, SongSort, next_sort, song_matches, sort_indices};

    /// 全部下标，等价于「没有关键词」时的显示顺序。
    fn song_order(songs: &[Song], column: SongSort, descending: bool) -> Vec<usize> {
        sort_indices((0..songs.len()).collect(), songs, column, descending)
    }

    /// `Artist` / `Album` 没有从 `crate::models` 导出，测试就用最小 JSON 造条目，
    /// 顺带也覆盖了真实响应的字段形状（和 playlist_detail 的测试同一套做法）。
    fn song(name: &str, artist: &str, album: Option<&str>) -> Song {
        serde_json::from_value(serde_json::json!({
            "id": 1,
            "name": name,
            "ar": [{"id": 1, "name": artist}],
            "al": {"id": 1, "name": album},
            "dt": 0,
        }))
        .unwrap()
    }

    #[test]
    fn sorting_keeps_source_order_and_returns_to_default() {
        let songs = vec![
            Song {
                id: 1,
                name: "b".into(),
                ..Default::default()
            },
            Song {
                id: 2,
                name: "a".into(),
                ..Default::default()
            },
        ];
        assert_eq!(song_order(&songs, SongSort::Title, false), [1, 0]);
        assert_eq!(song_order(&songs, SongSort::Title, true), [0, 1]);
        assert_eq!(songs[0].id, 1);
        let mut sort = None;
        for _ in 0..4 {
            sort = next_sort(sort, SongSort::Title);
            assert!(sort.is_some());
        }
        assert!(next_sort(sort, SongSort::Title).is_none());
    }

    #[test]
    fn keyword_matches_title_artist_and_album_case_insensitively() {
        let track = song("Sunny Day", "Taylor Swift", Some("Fearless"));
        assert!(song_matches(&track, "sunny"));
        assert!(song_matches(&track, "taylor"));
        assert!(song_matches(&track, "fearless"));
        // 专辑缺失不应 panic，只是不命中。
        assert!(!song_matches(&song("a", "b", None), "fearless"));
        assert!(!song_matches(&track, "周"));
    }

    #[test]
    fn sorting_a_filtered_subset_only_reorders_matched_rows() {
        let songs = vec![
            song("b", "z", None),
            song("a", "y", None),
            song("c", "x", None),
        ];
        let matched: Vec<usize> = (0..songs.len()).filter(|&index| index != 1).collect();
        // 排序只在筛出来的下标里重排，结果就是全量排序去掉被滤掉的那行 ——
        // 被滤掉的歌不会因为换排序又冒回来。
        assert_eq!(song_order(&songs, SongSort::Title, false), [1, 0, 2]);
        assert_eq!(
            sort_indices(matched.clone(), &songs, SongSort::Title, false),
            [0, 2]
        );
        assert_eq!(sort_indices(matched, &songs, SongSort::Title, true), [2, 0]);
    }

    /// `tns`（音译）和 `alia`（别名）会显示成标题的副标题，用户看得见就一定会搜它。
    /// 「稻香」的副标题是 `（daoxiang）`，所以 `daoxiang` 必须能搜到它。
    #[test]
    fn keyword_also_matches_the_rendered_subtitle() {
        let mut track = song("稻香", "周杰伦", Some("叶惠美"));
        track.tns = vec!["dao xiang".into()];
        track.alia = vec!["稻香 Theme".into()];

        // 标题 / 歌手 / 专辑照旧。
        assert!(song_matches(&track, "稻香"));
        assert!(song_matches(&track, "周杰伦"));
        assert!(song_matches(&track, "叶惠美"));
        // 副标题里的命中也算，大小写不敏感。
        assert!(song_matches(&track, "xiang"));
        assert!(song_matches(&track, "DAO"));
        assert!(song_matches(&track, "theme"));
        assert!(!song_matches(&track, "不存在"));
    }
}
