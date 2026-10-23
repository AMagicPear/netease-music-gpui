use serde::{Deserialize, Serialize};

/// 播放方式。播放栏的单个按钮在 [`Self::ALL`] 上循环切换，选择结果持久化。
///
/// 这里只描述**策略**，不保存队列也不做下标推进：
/// - [`Self::shuffles`]：要不要先把播放列表洗一遍；
/// - [`Self::wraps`]：手动切歌和自动续播能否越过列表两端；
/// - [`Self::repeats_current`]：播完是重复当前这首还是往前走。
///
/// 下标的移动由播放列表负责，它知道当前曲目在哪一格。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayMode {
    /// 顺序播放：一首接一首，走到列表末尾就停下。
    #[default]
    Sequential,
    /// 单曲循环：反复播放当前这首。
    RepeatOne,
    /// 列表循环：播到队尾回到队首。
    RepeatAll,
    /// 随机播放：先把列表顺序打乱，再顺着打乱后的顺序播。
    Shuffle,
    /// 心动模式：在「我喜欢的音乐」上接着放推荐流，播到队尾自动续订一批。
    /// 只有播放「我喜欢的音乐」时，播放栏循环才包含这一档。
    Heart,
}

impl PlayMode {
    /// 全部播放方式，按钮循环就按这个顺序走。
    ///
    /// 心动模式排在最后：播放别的歌单时这一档并不存在，按钮直接跳过。
    pub const ALL: [Self; 5] = [
        Self::Sequential,
        Self::RepeatOne,
        Self::RepeatAll,
        Self::Shuffle,
        Self::Heart,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Sequential => "顺序播放",
            Self::RepeatOne => "单曲循环",
            Self::RepeatAll => "列表循环",
            Self::Shuffle => "随机播放",
            Self::Heart => "心动模式",
        }
    }

    /// 图标素材跟着模式走，文件名与 `assets/icons/播放顺序` 一一对应。
    pub fn icon_path(self) -> &'static str {
        match self {
            Self::Sequential => "icons/播放顺序/顺序.svg",
            Self::RepeatOne => "icons/播放顺序/单曲循环.svg",
            Self::RepeatAll => "icons/播放顺序/歌单循环.svg",
            Self::Shuffle => "icons/播放顺序/随机.svg",
            Self::Heart => "icons/播放顺序/心动模式.svg",
        }
    }

    /// 切换按钮依次前进到下一种方式，队尾回到队首。
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// 按钮循环真正要走的那一档。
    ///
    /// 心动模式建立在「我喜欢的音乐」上，所以只有正播放这张歌单时它才存在；
    /// 播放别的歌单时这一档直接跳过，按钮从随机播放绕回顺序播放。
    pub fn next_reachable(self, heart_reachable: bool) -> Self {
        let next = self.next();
        if next == Self::Heart && !heart_reachable {
            return next.next();
        }
        next
    }

    /// 是否用洗好的顺序替换列表本身的顺序；打乱后就按这个顺序播，
    /// 因此随机播放与其他方式的差别只在列表是怎么来的。
    pub fn shuffles(self) -> bool {
        matches!(self, Self::Shuffle)
    }

    /// 是否越过列表两端：队尾的下一首是队首，队首的上一首是队尾。
    pub fn wraps(self) -> bool {
        matches!(self, Self::RepeatAll)
    }

    /// 播完一首是否回到它的开头，而不是前进到下一首。
    pub fn repeats_current(self) -> bool {
        matches!(self, Self::RepeatOne)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_cycles_through_every_mode() {
        let mut mode = PlayMode::default();
        let mut visited = vec![mode];
        for _ in 0..PlayMode::ALL.len() - 1 {
            mode = mode.next();
            visited.push(mode);
        }
        assert_eq!(visited, PlayMode::ALL);
        assert_eq!(mode.next(), PlayMode::default());
    }

    #[test]
    fn every_mode_has_a_distinct_label_and_icon() {
        let mut labels = Vec::new();
        let mut icons = Vec::new();
        for mode in PlayMode::ALL {
            assert!(!mode.label().is_empty());
            labels.push(mode.label());
            icons.push(mode.icon_path());
        }
        labels.dedup();
        icons.dedup();
        assert_eq!(labels.len(), PlayMode::ALL.len());
        assert_eq!(icons.len(), PlayMode::ALL.len());
    }

    #[test]
    fn each_mode_declares_its_own_strategy() {
        let expected = [
            // 方式, 洗牌, 越过两端, 重复当前这首
            (PlayMode::Sequential, false, false, false),
            (PlayMode::RepeatOne, false, false, true),
            (PlayMode::RepeatAll, false, true, false),
            (PlayMode::Shuffle, true, false, false),
            // 心动模式按服务端给的顺序播，队尾靠自动续订补新的一批，不绕回队首。
            (PlayMode::Heart, false, false, false),
        ];
        assert_eq!(expected.len(), PlayMode::ALL.len());
        for (mode, shuffles, wraps, repeats) in expected {
            assert_eq!(mode.shuffles(), shuffles, "{mode:?} 洗牌策略不符");
            assert_eq!(mode.wraps(), wraps, "{mode:?} 边界策略不符");
            assert_eq!(mode.repeats_current(), repeats, "{mode:?} 续播策略不符");
        }
    }

    #[test]
    fn heart_is_only_reachable_while_the_favorite_playlist_plays() {
        // 随机播放之后就是心动模式。
        assert_eq!(PlayMode::Shuffle.next(), PlayMode::Heart);
        // 不在「我喜欢的音乐」上时这一档不存在，按钮直接绕回顺序播放。
        assert_eq!(
            PlayMode::Shuffle.next_reachable(false),
            PlayMode::Sequential
        );
        assert_eq!(PlayMode::Shuffle.next_reachable(true), PlayMode::Heart);
        // 心动模式本身也在循环里，离开它回到顺序播放。
        assert_eq!(PlayMode::Heart.next_reachable(true), PlayMode::Sequential);
    }
}
