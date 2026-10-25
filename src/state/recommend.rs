//! 推荐页的独立区块加载、账号隔离和顶部入口组装。

use gpui::{Context, Entity, ReadGlobal, SharedString, Subscription};

use super::account::AccountState;
use super::resource::{Resource, request};
use crate::api::MusicApi;
use crate::models::{
    FeaturedActivity, PLAYLIST_BLOCK_CODE, RecommendEntry, RecommendedPlaylist,
    playlists_from_creatives,
};

/// 顶部入口的展示类型；每日推荐和私人漫游有独立角标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickEntryKind {
    DailySongs,
    HeartMode,
    Radar,
    Roam,
    SimilarSongs,
    StyleDaily,
    FrequentArtists,
    Other,
}

/// 顺序即显示顺序，同一种类型可以出现多次（不同曲风的日推）。
#[derive(Debug, Clone)]
pub struct QuickEntry {
    pub kind: QuickEntryKind,
    pub title: SharedString,
    pub caption: SharedString,
    pub cover: Option<SharedString>,
}

/// 前五个固定入口 + 风格日推占位 + 常听歌手。
fn default_entries() -> Vec<QuickEntry> {
    vec![
        entry(
            QuickEntryKind::DailySongs,
            "每日推荐",
            "每天更新的个性化推荐",
        ),
        entry(
            QuickEntryKind::HeartMode,
            "心动模式",
            "你的红心歌曲和更多相似推荐",
        ),
        entry(QuickEntryKind::Radar, "私人雷达", "今日份的私人雷达歌单"),
        entry(QuickEntryKind::Roam, "私人漫游", "开启一场私人漫游之旅"),
        entry(QuickEntryKind::SimilarSongs, "相似歌曲", "从你喜欢的歌听起"),
        entry(
            QuickEntryKind::StyleDaily,
            "风格日推",
            "根据你的听歌风格生成",
        ),
        entry(
            QuickEntryKind::FrequentArtists,
            "常听歌手",
            "从你常听的歌手听起",
        ),
    ]
}

fn entry(kind: QuickEntryKind, title: &'static str, caption: &'static str) -> QuickEntry {
    QuickEntry {
        kind,
        title: title.into(),
        caption: caption.into(),
        cover: None,
    }
}

impl QuickEntry {
    /// 每日推荐由固定卡片承接，其余接口条目替换风格日推占位。
    fn from_api(entry: &RecommendEntry) -> Option<Self> {
        if entry.sub_resource_type == "dailySong" {
            return None;
        }
        let kind = if entry.sub_resource_type == "style_dailySong" {
            QuickEntryKind::StyleDaily
        } else {
            match entry.resource_type.as_str() {
                "similarSong" => QuickEntryKind::SimilarSongs,
                "star" => QuickEntryKind::HeartMode,
                "fm" => QuickEntryKind::Roam,
                _ => QuickEntryKind::Other,
            }
        };
        let title = if entry.cover_text.is_empty() {
            &entry.title
        } else {
            &entry.cover_text
        };
        if title.is_empty() {
            return None;
        }
        Some(Self {
            kind,
            title: title.clone().into(),
            caption: if entry.title == *title {
                SharedString::default()
            } else {
                entry.title.clone().into()
            },
            cover: entry.cover_url.clone().map(Into::into),
        })
    }
}

/// 推荐页的 store。四块内容各发各的请求，互不阻塞：
/// 顶部入口和精选活动游客也能取；雷达歌单和个性化推荐歌单要登录态。
pub struct RecommendStore {
    /// 顶部入口卡片。游客也能取。
    pub entries: Resource<Vec<QuickEntry>>,
    /// 私人雷达歌单（仅登录）。
    pub radar: Resource<Vec<RecommendedPlaylist>>,
    /// 推荐歌单。页面上的加载态和错误都取自它。
    pub playlists: Resource<Vec<RecommendedPlaylist>>,
    /// 精选活动横幅。
    pub featured: Resource<Vec<FeaturedActivity>>,
    /// 雷达歌单第一首的封面。
    pub radar_first_song_cover: Resource<Option<SharedString>>,
    /// 已加载过的身份：`Some(0)` 表示游客，`None` 表示还没取过。
    user_id: Option<u64>,
    _user_subscription: Option<Subscription>,
}

impl RecommendStore {
    pub fn new(user: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&user, |this, user, cx| {
            let account = user.read(cx);
            if !account.loading {
                this.load(account.profile.user_id, false, cx);
            }
        });
        let mut this = Self {
            // 占位入口先摆上，数据到了再替换。
            entries: Resource::with_value(default_entries()),
            radar: Resource::new(),
            playlists: Resource::new(),
            featured: Resource::new(),
            radar_first_song_cover: Resource::new(),
            user_id: None,
            _user_subscription: Some(subscription),
        };
        let (user_id, loading) = {
            let account = user.read(cx);
            (account.profile.user_id, account.loading)
        };
        // 会话恢复完成后由账号通知触发加载，避免先请求一轮游客数据。
        if user_id != 0 || !loading {
            this.load(user_id, false, cx);
        }
        this
    }

    /// 进入页面时刷新，各区块合并已有的在途请求。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.revalidate(cx);
    }

    /// 切回页面时的静默刷新。
    ///
    /// 用户离开期间推荐内容可能变了，回来时该看到最新的。旧内容不清空，
    /// 新数据到了再替换——和评论列表一个路子。已经在拉就算了，不打断。
    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        if let Some(user_id) = self.user_id {
            self.load(user_id, true, cx);
        }
    }

    fn load(&mut self, user_id: u64, force: bool, cx: &mut Context<Self>) {
        if !self.should_load(user_id, force) {
            return;
        }
        // 换账号后立即清除上一位用户的内容。
        if self.user_id != Some(user_id) {
            self.entries.set(default_entries());
            self.radar.reset();
            self.playlists.reset();
            self.featured.reset();
            self.radar_first_song_cover.reset();
        }
        self.user_id = Some(user_id);
        // 四块内容各拉各的，互不等待：每块一个数据源，自己的代次。
        self.load_entries(cx);
        self.load_radar(user_id, cx);
        self.load_playlists(user_id, cx);
        self.load_featured(cx);
        cx.notify();
    }

    fn load_entries(&mut self, cx: &mut Context<Self>) {
        if self.entries.is_loading() {
            return;
        }
        let generation = self.entries.begin();
        let client = MusicApi::global(cx).client.clone();
        self.entries.track(request(
            cx,
            "recommend entries",
            MusicApi::recommend_entries(client),
            move |this, result, cx| {
                if this.apply_entries(generation, result) {
                    cx.notify();
                }
            },
        ));
    }

    fn load_radar(&mut self, user_id: u64, cx: &mut Context<Self>) {
        if user_id == 0 || self.radar.is_loading() {
            return;
        }
        let generation = self.radar.begin();
        let client = MusicApi::global(cx).client.clone();
        self.radar.track(request(
            cx,
            "radar playlists",
            MusicApi::radar_playlists(client),
            move |this, result, cx| {
                let success = result.is_ok();
                if this.apply_radar(generation, result) {
                    if success {
                        this.load_radar_first_song(cx);
                    }
                    cx.notify();
                }
            },
        ));
    }

    fn load_playlists(&mut self, user_id: u64, cx: &mut Context<Self>) {
        let client = MusicApi::global(cx).client.clone();
        self.playlists.fetch(
            "recommended playlists",
            cx,
            |this| &mut this.playlists,
            async move {
                if user_id != 0 {
                    MusicApi::recommended_playlists(client).await
                } else {
                    MusicApi::homepage_blocks(client).await.map(|blocks| {
                        blocks
                            .into_iter()
                            .find(|block| block.code == PLAYLIST_BLOCK_CODE)
                            .map(|block| playlists_from_creatives(&block.creatives))
                            .unwrap_or_default()
                    })
                }
            },
        );
    }

    fn load_featured(&mut self, cx: &mut Context<Self>) {
        let client = MusicApi::global(cx).client.clone();
        self.featured.fetch(
            "featured activities",
            cx,
            |this| &mut this.featured,
            MusicApi::featured_activities(client),
        );
    }

    /// 账号通知只响应身份变化，进入页面时另走主动刷新。
    fn should_load(&self, user_id: u64, force: bool) -> bool {
        force || self.user_id != Some(user_id)
    }

    fn apply_entries(
        &mut self,
        generation: u64,
        result: Result<Vec<RecommendEntry>, String>,
    ) -> bool {
        // 失败时保留上一批入口，不要把已经拿到的风格日推打回占位。
        let Ok(api_entries) = result else {
            return self
                .entries
                .apply(generation, Err("推荐入口加载失败".into()));
        };
        // daily 补固定首卡；动态入口替换风格占位，常听歌手保持在最后。
        let mut entries = default_entries();
        if let Some(daily) = api_entries
            .iter()
            .find(|entry| entry.sub_resource_type == "dailySong")
        {
            if let Some(cover) = &daily.cover_url {
                entries[0].cover = Some(cover.clone().into());
            }
            if !daily.title.is_empty() {
                entries[0].caption = daily.title.clone().into();
            }
        }
        if let Some(heart) = api_entries
            .iter()
            .find(|entry| entry.resource_type == "star")
            .and_then(QuickEntry::from_api)
        {
            entries[1] = heart;
        }
        let dynamic: Vec<QuickEntry> = api_entries
            .iter()
            .filter_map(QuickEntry::from_api)
            .filter(|entry| entry.kind != QuickEntryKind::HeartMode)
            .collect();
        if !dynamic.is_empty() {
            entries.splice(5..6, dynamic);
        }
        let applied = self.entries.apply(generation, Ok(entries));
        if applied {
            self.apply_radar_to_entry();
        }
        applied
    }

    fn apply_radar(
        &mut self,
        generation: u64,
        result: Result<Vec<RecommendedPlaylist>, String>,
    ) -> bool {
        let Ok(playlists) = result else {
            return self.radar.apply(generation, Err("雷达歌单加载失败".into()));
        };
        let applied = self.radar.apply(generation, Ok(playlists));
        if applied {
            self.radar_first_song_cover.reset();
            self.apply_radar_to_entry();
        }
        applied
    }

    /// 两个请求到达顺序不定，入口和雷达更新后都要补封面和文案。
    fn apply_radar_to_entry(&mut self) {
        let Some(first) = self.radar.value().and_then(|radar| radar.first()) else {
            return;
        };
        let Some(entries) = self.entries.value_mut() else {
            return;
        };
        let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.kind == QuickEntryKind::Radar)
        else {
            return;
        };
        entry.cover = first.pic_url.clone().map(Into::into);
        if !first.name.is_empty() {
            entry.caption = first.name.clone().into();
        }
    }

    fn load_radar_first_song(&mut self, cx: &mut Context<Self>) {
        let Some(playlist) = self.radar.value().and_then(|radar| radar.first()) else {
            return;
        };
        let id = playlist.id;
        let client = MusicApi::global(cx).client.clone();
        self.radar_first_song_cover.fetch(
            "radar first song cover",
            cx,
            |this| &mut this.radar_first_song_cover,
            async move {
                MusicApi::playlist_first_song(client, id)
                    .await
                    .map(|song| song.al.pic_url.map(Into::into))
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{QuickEntryKind, RecommendStore, default_entries};
    use crate::models::{RecommendEntry, RecommendedPlaylist};
    use crate::state::resource::Resource;

    fn store() -> RecommendStore {
        RecommendStore {
            entries: Resource::with_value(default_entries()),
            radar: Resource::new(),
            playlists: Resource::new(),
            featured: Resource::new(),
            radar_first_song_cover: Resource::new(),
            user_id: None,
            _user_subscription: None,
        }
    }

    fn entry(sub_resource_type: &str, title: &str, cover_text: &str) -> RecommendEntry {
        RecommendEntry {
            title: title.into(),
            cover_text: cover_text.into(),
            cover_url: Some(format!("cover-{sub_resource_type}")),
            resource_type: "dailySongs".into(),
            sub_resource_type: sub_resource_type.into(),
            resource_id: 1,
        }
    }

    fn playlist(id: u64, name: &str) -> RecommendedPlaylist {
        RecommendedPlaylist {
            id,
            name: name.into(),
            pic_url: Some(format!("cover-{id}")),
            play_count: 1000 + id,
        }
    }

    #[test]
    fn entries_keep_fixed_five_and_append_style_daily() {
        let mut this = store();
        let generation = this.entries.begin();
        assert!(this.apply_entries(
            generation,
            Ok(vec![
                entry("dailySong", "每日推荐 | 你的多种喜好推荐", "29"),
                entry("style_dailySong", "每日欧美流行推荐", "欧美流行日推"),
                entry("style_dailySong", "每日华语流行推荐", "华语流行日推"),
            ])
        ));
        let entries = this.entries.value().unwrap();
        assert_eq!(entries.len(), 8);
        let kinds: Vec<_> = entries.iter().map(|entry| entry.kind).collect();
        assert_eq!(
            kinds,
            vec![
                QuickEntryKind::DailySongs,
                QuickEntryKind::HeartMode,
                QuickEntryKind::Radar,
                QuickEntryKind::Roam,
                QuickEntryKind::SimilarSongs,
                QuickEntryKind::StyleDaily,
                QuickEntryKind::StyleDaily,
                QuickEntryKind::FrequentArtists,
            ]
        );
        assert_eq!(entries[0].title.as_ref(), "每日推荐");
        assert_eq!(entries[0].caption.as_ref(), "每日推荐 | 你的多种喜好推荐");
        assert_eq!(entries[0].cover.as_deref(), Some("cover-dailySong"));
        assert_eq!(entries[5].title.as_ref(), "欧美流行日推");
        assert_eq!(entries[5].caption.as_ref(), "每日欧美流行推荐");
        assert_eq!(entries[7].title.as_ref(), "常听歌手");
    }

    #[test]
    fn api_heart_entry_updates_fixed_card_without_duplicating_it() {
        let mut store = store();
        let mut heart = entry("", "心动推荐", "心动模式");
        heart.resource_type = "star".into();
        let generation = store.entries.begin();
        store.apply_entries(generation, Ok(vec![heart]));
        let entries = store.entries.value().unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == QuickEntryKind::HeartMode)
                .count(),
            1
        );
        assert_eq!(entries[1].caption.as_ref(), "心动推荐");
    }

    #[test]
    fn radar_and_playlists_land_independently() {
        let mut this = store();
        let radar = this.radar.begin();
        assert!(this.apply_radar(radar, Ok(vec![playlist(3, "今天从《歌》听起")])));
        assert_eq!(this.radar.value().unwrap().len(), 1);
        let radar_entry = this
            .entries
            .value()
            .unwrap()
            .iter()
            .find(|entry| entry.kind == QuickEntryKind::Radar)
            .expect("首行应当有私人雷达入口");
        assert_eq!(radar_entry.caption.as_ref(), "今天从《歌》听起");
        assert_eq!(radar_entry.cover.as_deref(), Some("cover-3"));
        let playlists = this.playlists.begin();
        assert!(
            this.playlists
                .apply(playlists, Ok(vec![playlist(7, "歌单")]))
        );
        assert_eq!(this.playlists.value().unwrap().len(), 1);
        assert_eq!(this.radar.value().unwrap().len(), 1);
    }

    #[test]
    fn radar_arriving_before_entries_still_fills_the_fixed_card() {
        let mut this = store();
        let radar = this.radar.begin();
        assert!(this.apply_radar(radar, Ok(vec![playlist(9, "今天从《歌》听起")])));
        // 入口请求后到：重建固定入口时要把已有的雷达数据补回去。
        let entries = this.entries.begin();
        assert!(this.apply_entries(entries, Ok(vec![entry("dailySong", "每日推荐", "29")])));
        let radar_entry = this
            .entries
            .value()
            .unwrap()
            .iter()
            .find(|entry| entry.kind == QuickEntryKind::Radar)
            .expect("首行应当有私人雷达入口");
        assert_eq!(radar_entry.cover.as_deref(), Some("cover-9"));
    }
}
