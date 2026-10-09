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
}

impl PlayMode {
    /// 播放栏按钮依次经过的全部方式，顺序与官方一致。
    pub const ALL: [Self; 4] = [
        Self::Sequential,
        Self::RepeatOne,
        Self::RepeatAll,
        Self::Shuffle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Sequential => "顺序播放",
            Self::RepeatOne => "单曲循环",
            Self::RepeatAll => "列表循环",
            Self::Shuffle => "随机播放",
        }
    }

    /// 图标素材跟着模式走，文件名与 `assets/icons/播放顺序` 一一对应。
    pub fn icon_path(self) -> &'static str {
        match self {
            Self::Sequential => "icons/播放顺序/顺序.svg",
            Self::RepeatOne => "icons/播放顺序/单曲循环.svg",
            Self::RepeatAll => "icons/播放顺序/歌单循环.svg",
            Self::Shuffle => "icons/播放顺序/随机.svg",
        }
    }

    /// 切换按钮依次前进到下一种方式，队尾回到队首。
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
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
        ];
        assert_eq!(expected.len(), PlayMode::ALL.len());
        for (mode, shuffles, wraps, repeats) in expected {
            assert_eq!(mode.shuffles(), shuffles, "{mode:?} 洗牌策略不符");
            assert_eq!(mode.wraps(), wraps, "{mode:?} 边界策略不符");
            assert_eq!(mode.repeats_current(), repeats, "{mode:?} 续播策略不符");
        }
    }
}
