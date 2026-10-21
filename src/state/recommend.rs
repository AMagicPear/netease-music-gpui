//! 推荐页的独立区块加载、账号隔离和顶部入口组装。

use gpui::{Context, Entity, ReadGlobal, SharedString, Subscription};

use super::account::AccountState;
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
    pub entries: Vec<QuickEntry>,
    pub radar_first_song_cover: Option<SharedString>,
    pub radar: Vec<RecommendedPlaylist>,
    pub playlists: Vec<RecommendedPlaylist>,
    pub featured: Vec<FeaturedActivity>,
    /// 推荐歌单是否已经为当前身份拿到过内容。
    pub loaded: bool,
    /// 推荐歌单正在加载（刷新按钮转圈）。
    pub loading: bool,
    pub error: Option<String>,
    /// 已加载过的身份：`Some(0)` 表示游客，`None` 表示还没取过。
    user_id: Option<u64>,
    generation: u64,
    entries_request: Option<tokio::task::AbortHandle>,
    radar_request: Option<tokio::task::AbortHandle>,
    playlist_request: Option<tokio::task::AbortHandle>,
    featured_request: Option<tokio::task::AbortHandle>,
    _user_subscription: Option<Subscription>,
}

impl RecommendStore {
    pub fn new(user: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&user, |this, user, cx| {
            this.load(user.read(cx).profile.user_id, false, cx);
        });
        let mut this = Self {
            entries: default_entries(),
            radar_first_song_cover: None,
            radar: Vec::new(),
            playlists: Vec::new(),
            featured: Vec::new(),
            loaded: false,
            loading: false,
            error: None,
            user_id: None,
            generation: 0,
            entries_request: None,
            radar_request: None,
            playlist_request: None,
            featured_request: None,
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

    /// 强制刷新；取消旧请求，并用 generation 丢弃已返回的旧结果。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(self.user_id.unwrap_or_default(), true, cx);
    }

    fn load(&mut self, user_id: u64, force: bool, cx: &mut Context<Self>) {
        if !self.should_load(user_id, force) {
            return;
        }
        self.abort_requests();
        // 换账号后立即清除上一位用户的内容。
        if self.user_id != Some(user_id) {
            self.entries = default_entries();
            self.radar_first_song_cover = None;
            self.radar.clear();
            self.playlists.clear();
            self.featured.clear();
            self.loaded = false;
        }
        self.user_id = Some(user_id);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.error = None;
        self.loading = true;
        let (runtime, client) = {
            let api = MusicApi::global(cx);
            (api.runtime.handle().clone(), api.client.clone())
        };

        let request = runtime.spawn(MusicApi::recommend_entries(client.clone()));
        self.entries_request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("推荐入口请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.apply_entries(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();

        // 雷达歌单只对登录用户开放。
        if user_id != 0 {
            let request = runtime.spawn(MusicApi::radar_playlists(client.clone()));
            self.radar_request = Some(request.abort_handle());
            cx.spawn(async move |this, cx| {
                let result = request
                    .await
                    .unwrap_or_else(|_| Err("雷达歌单请求任务失败".into()));
                let _ = this.update(cx, |this, cx| {
                    if this.apply_radar(generation, result) {
                        this.load_radar_first_song(cx);
                        cx.notify();
                    }
                });
            })
            .detach();
        }

        // 推荐歌单：登录后走官方新接口；游客回落到首页区块里的「推荐歌单」。
        let request = if user_id != 0 {
            runtime.spawn(MusicApi::recommended_playlists(client.clone()))
        } else {
            let client = client.clone();
            runtime.spawn(async move {
                MusicApi::homepage_blocks(client).await.map(|blocks| {
                    blocks
                        .into_iter()
                        .find(|block| block.code == PLAYLIST_BLOCK_CODE)
                        .map(|block| playlists_from_creatives(&block.creatives))
                        .unwrap_or_default()
                })
            })
        };
        self.playlist_request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("推荐歌单请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.apply_playlists(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();

        let request = runtime.spawn(MusicApi::featured_activities(client));
        self.featured_request = Some(request.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("精选活动请求任务失败".into()));
            let _ = this.update(cx, |this, cx| {
                if this.apply_featured(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();

        cx.notify();
    }

    /// 同一身份只取一次；失败或主动刷新时允许重试。
    fn should_load(&self, user_id: u64, force: bool) -> bool {
        force || self.user_id != Some(user_id) || self.error.is_some()
    }

    fn apply_entries(
        &mut self,
        generation: u64,
        result: Result<Vec<RecommendEntry>, String>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.entries_request = None;
        // 失败时保留上一批入口，不要把已经拿到的风格日推打回占位。
        let Ok(api_entries) = result else {
            return true;
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
        let dynamic: Vec<QuickEntry> = api_entries
            .iter()
            .filter_map(QuickEntry::from_api)
            .collect();
        if !dynamic.is_empty() {
            entries.splice(5..6, dynamic);
        }
        self.entries = entries;
        self.apply_radar_to_entry();
        true
    }

    fn apply_radar(
        &mut self,
        generation: u64,
        result: Result<Vec<RecommendedPlaylist>, String>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.radar_request = None;
        if let Ok(playlists) = result {
            self.radar = playlists;
            self.radar_first_song_cover = None;
            self.apply_radar_to_entry();
        }
        true
    }

    /// 两个请求到达顺序不定，入口和雷达更新后都要补封面和文案。
    fn apply_radar_to_entry(&mut self) {
        let Some(first) = self.radar.first() else {
            return;
        };
        let Some(entry) = self
            .entries
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

    pub fn load_radar_first_song(&mut self, cx: &mut Context<Self>) {
        let Some(playlist) = self.radar.first() else {
            return;
        };
        let playlist_id = playlist.id;
        let generation = self.generation;
        let (runtime, client) = {
            let api = MusicApi::global(cx);
            (api.runtime.handle().clone(), api.client.clone())
        };
        let request = runtime.spawn(MusicApi::playlist_first_song(client, playlist_id));
        cx.spawn(async move |this, cx| {
            let result = request.await.ok().and_then(Result::ok);
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation
                    && this
                        .radar
                        .first()
                        .is_some_and(|playlist| playlist.id == playlist_id)
                {
                    this.radar_first_song_cover =
                        result.and_then(|song| song.al.pic_url).map(Into::into);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 推荐歌单落地：这是页面唯一的 `loading` / `error` 来源（刷新按钮在它上面）。
    fn apply_playlists(
        &mut self,
        generation: u64,
        result: Result<Vec<RecommendedPlaylist>, String>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.playlist_request = None;
        self.loading = false;
        match result {
            Ok(playlists) => {
                self.playlists = playlists;
                self.loaded = true;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
        true
    }

    fn apply_featured(
        &mut self,
        generation: u64,
        result: Result<Vec<FeaturedActivity>, String>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.featured_request = None;
        if let Ok(activities) = result {
            self.featured = activities;
        }
        true
    }

    fn abort_requests(&mut self) {
        for request in [
            &mut self.entries_request,
            &mut self.radar_request,
            &mut self.playlist_request,
            &mut self.featured_request,
        ] {
            if let Some(request) = request.take() {
                request.abort();
            }
        }
    }
}

impl Drop for RecommendStore {
    fn drop(&mut self) {
        self.abort_requests();
    }
}

#[cfg(test)]
mod tests {
    use super::{QuickEntryKind, RecommendStore, default_entries};
    use crate::models::{FeaturedActivity, RecommendEntry, RecommendedPlaylist};

    fn store() -> RecommendStore {
        RecommendStore {
            entries: default_entries(),
            radar_first_song_cover: None,
            radar: Vec::new(),
            playlists: Vec::new(),
            featured: Vec::new(),
            loaded: false,
            loading: false,
            error: None,
            user_id: None,
            generation: 0,
            entries_request: None,
            radar_request: None,
            playlist_request: None,
            featured_request: None,
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

    fn activity(title: &str) -> FeaturedActivity {
        FeaturedActivity {
            title: title.into(),
            image_url: Some(format!("image-{title}")),
        }
    }

    #[test]
    fn same_identity_loads_once_until_it_fails() {
        let mut this = store();
        assert!(this.should_load(42, false));
        this.user_id = Some(42);
        assert!(!this.should_load(42, false));
        this.error = Some("网络错误".into());
        assert!(this.should_load(42, false));
        this.error = None;
        assert!(this.should_load(42, true));
        assert!(this.should_load(7, false));
        assert!(this.should_load(0, false));
    }

    #[test]
    fn entries_keep_fixed_five_and_append_style_daily() {
        let mut this = store();
        this.generation = 1;
        assert!(this.apply_entries(
            1,
            Ok(vec![
                entry("dailySong", "每日推荐 | 你的多种喜好推荐", "29"),
                entry("style_dailySong", "每日欧美流行推荐", "欧美流行日推"),
                entry("style_dailySong", "每日华语流行推荐", "华语流行日推"),
            ])
        ));
        assert_eq!(this.entries.len(), 8);
        let kinds: Vec<_> = this.entries.iter().map(|entry| entry.kind).collect();
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
        assert_eq!(this.entries[0].title.as_ref(), "每日推荐");
        assert_eq!(
            this.entries[0].caption.as_ref(),
            "每日推荐 | 你的多种喜好推荐"
        );
        assert_eq!(this.entries[0].cover.as_deref(), Some("cover-dailySong"));
        assert_eq!(this.entries[5].title.as_ref(), "欧美流行日推");
        assert_eq!(this.entries[5].caption.as_ref(), "每日欧美流行推荐");
        assert_eq!(this.entries[7].title.as_ref(), "常听歌手");
    }

    #[test]
    fn stale_results_do_not_change_any_section() {
        let mut this = store();
        this.generation = 4;
        this.loading = true;
        assert!(!this.apply_entries(3, Ok(vec![entry("dailySong", "旧的一批", "1")])));
        assert!(!this.apply_radar(3, Ok(vec![playlist(1, "旧雷达")])));
        assert!(!this.apply_playlists(3, Err("旧错误".into())));
        assert!(!this.apply_featured(3, Ok(vec![activity("旧活动")])));
        assert!(this.radar.is_empty());
        assert!(this.playlists.is_empty());
        assert!(this.featured.is_empty());
        assert!(this.loading);
        assert!(this.error.is_none());
        assert_eq!(this.entries.len(), 7);
        assert_eq!(this.entries[0].caption.as_ref(), "每天更新的个性化推荐");
        assert!(this.apply_entries(
            4,
            Ok(vec![
                entry("dailySong", "新的一批", "2"),
                entry("style_dailySong", "新风格", "新风格日推"),
            ])
        ));
        assert_eq!(this.entries[0].caption.as_ref(), "新的一批");
        assert_eq!(this.entries[5].title.as_ref(), "新风格日推");
        assert_eq!(this.entries[6].title.as_ref(), "常听歌手");
    }

    #[test]
    fn radar_and_playlists_land_independently() {
        let mut this = store();
        this.generation = 1;
        this.loading = true;
        assert!(this.apply_radar(1, Ok(vec![playlist(3, "今天从《歌》听起")])));
        assert_eq!(this.radar.len(), 1);
        let radar_entry = this
            .entries
            .iter()
            .find(|entry| entry.kind == QuickEntryKind::Radar)
            .expect("首行应当有私人雷达入口");
        assert_eq!(radar_entry.caption.as_ref(), "今天从《歌》听起");
        assert_eq!(radar_entry.cover.as_deref(), Some("cover-3"));
        assert!(this.loading);
        assert!(this.apply_playlists(1, Ok(vec![playlist(7, "歌单")])));
        assert!(!this.loading);
        assert!(this.loaded);
        assert_eq!(this.playlists.len(), 1);
        assert_eq!(this.radar.len(), 1);
    }

    #[test]
    fn radar_arriving_before_entries_still_fills_the_fixed_card() {
        let mut this = store();
        this.generation = 1;
        assert!(this.apply_radar(1, Ok(vec![playlist(9, "今天从《歌》听起")])));
        // 入口请求后到：重建固定入口时要把已有的雷达数据补回去。
        assert!(this.apply_entries(1, Ok(vec![entry("dailySong", "每日推荐", "29")])));
        let radar_entry = this
            .entries
            .iter()
            .find(|entry| entry.kind == QuickEntryKind::Radar)
            .expect("首行应当有私人雷达入口");
        assert_eq!(radar_entry.cover.as_deref(), Some("cover-9"));
    }

    #[test]
    fn failed_optional_requests_preserve_previous_content() {
        let mut this = store();
        this.generation = 1;
        assert!(this.apply_entries(1, Ok(vec![entry("dailySong", "旧推荐", "1")])));
        assert!(this.apply_radar(1, Ok(vec![playlist(9, "旧雷达")])));
        assert!(this.apply_featured(1, Ok(vec![activity("活动一")])));
        assert!(this.apply_entries(1, Err("失败".into())));
        assert!(this.apply_radar(1, Err("失败".into())));
        assert!(this.apply_featured(1, Err("失败".into())));
        assert_eq!(this.entries[0].caption.as_ref(), "旧推荐");
        assert_eq!(this.radar[0].id, 9);
        assert_eq!(this.featured.len(), 1);
    }

    #[test]
    fn failed_playlist_request_records_error_and_keeps_previous_list() {
        let mut this = store();
        this.user_id = Some(1);
        this.generation = 1;
        this.loading = true;
        this.playlists = vec![playlist(1, "旧的一批")];
        assert!(this.apply_playlists(1, Err("推荐歌单请求失败".into())));
        assert!(!this.loading);
        assert!(this.error.is_some());
        assert_eq!(this.playlists.len(), 1);
    }
}
