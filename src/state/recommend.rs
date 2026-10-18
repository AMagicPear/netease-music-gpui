//! 推荐页的数据：官方 3.1.41 的四块内容——顶部入口、雷达歌单、推荐歌单、
//! 精选活动。
//!
//! 页面只读快照；请求取消、新旧账号校验（generation）和「同一账号只取一次」
//! 的守卫都收在这里，分工同 `SearchResults`。

use gpui::{Context, Entity, ReadGlobal, SharedString, Subscription};

use super::account::AccountState;
use crate::api::MusicApi;
use crate::models::{
    FeaturedActivity, PLAYLIST_BLOCK_CODE, RecommendEntry, RecommendedPlaylist,
    playlists_from_creatives,
};

/// 入口点开之后做什么。入口并不都是歌单：每日推荐是当天的歌曲列表，心动模式 /
/// 私人漫游 / 相似歌曲各有自己的功能页；风格日推要带 tag/category/song 参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickEntryTarget {
    /// 按 id 打开歌单详情。
    Playlist(u64),
    DailySongs,
    HeartMode,
    Roam,
    SimilarSongs,
    StyleDaily {
        category_id: Option<u64>,
        tag_id: Option<u64>,
        song_id: Option<u64>,
    },
}

/// 顶部入口的语义角色。只影响卡片上少量表现（每日推荐显示日期角标），
/// 与点开去哪（[`QuickEntryTarget`]）是两件事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickEntryKind {
    DailySongs,
    HeartMode,
    Radar,
    Roam,
    SimilarSongs,
    StyleDaily,
    FrequentArtists,
    /// 接口里暂时没归类的类型，样式和普通入口一样。
    Other,
}

/// 顶部的一个入口。整行是列表而不是按 `kind` 建索引：顺序即显示顺序，
/// 同一个 `kind` 可以出现多次（几种不同曲风的日推）。
#[derive(Debug, Clone)]
pub struct QuickEntry {
    pub kind: QuickEntryKind,
    /// 封面左上角的标题：每日推荐固定是「每日推荐」，风格日推用 `coverText`
    /// （「欧美流行日推」）。
    pub title: SharedString,
    /// 标题下的一行说明：默认是入口自己的文案，接口回来后用内容的 `title`
    /// （「每日推荐 | 你的多种喜好推荐」/「每日欧美流行推荐」）。
    pub caption: SharedString,
    pub cover: Option<SharedString>,
    /// 点开去哪；数据还没回来时是 `None`。
    pub target: Option<QuickEntryTarget>,
}

/// 首行默认内容：官方版式的前五个固定入口 + 风格日推占位 + 常听歌手。
///
/// 官方接口的 `pcRcmdDemoteItems` 回来后：
/// - 每日推荐那条只用来补第一个卡片的封面 / 文案，不再重复插一张；
/// - `style_dailySong` 替换掉第 6 张「风格日推」占位，可能有多个；
/// - 「常听歌手」保持在最后。
fn default_entries() -> Vec<QuickEntry> {
    vec![
        entry(
            QuickEntryKind::DailySongs,
            "每日推荐",
            "每天更新的个性化推荐",
            Some(QuickEntryTarget::DailySongs),
        ),
        entry(
            QuickEntryKind::HeartMode,
            "心动模式",
            "你的红心歌曲和更多相似推荐",
            Some(QuickEntryTarget::HeartMode),
        ),
        // 私人雷达是歌单，id 要等雷达接口回来才填。
        entry(QuickEntryKind::Radar, "私人雷达", "今日份的私人雷达歌单", None),
        entry(
            QuickEntryKind::Roam,
            "私人漫游",
            "开启一场私人漫游之旅",
            Some(QuickEntryTarget::Roam),
        ),
        entry(
            QuickEntryKind::SimilarSongs,
            "相似歌曲",
            "从你喜欢的歌听起",
            Some(QuickEntryTarget::SimilarSongs),
        ),
        entry(
            QuickEntryKind::StyleDaily,
            "风格日推",
            "根据你的听歌风格生成",
            None,
        ),
        entry(
            QuickEntryKind::FrequentArtists,
            "常听歌手",
            "从你常听的歌手听起",
            None,
        ),
    ]
}

fn entry(
    kind: QuickEntryKind,
    title: &'static str,
    caption: &'static str,
    target: Option<QuickEntryTarget>,
) -> QuickEntry {
    QuickEntry {
        kind,
        title: title.into(),
        caption: caption.into(),
        cover: None,
        target,
    }
}

impl QuickEntry {
    /// 把接口条目映射成尾部动态入口。官方对每日推荐和风格日推的标题取法不同：
    /// - 每日推荐：左上角固定「每日推荐」（日期角标由页面按 `kind` 画），
    ///   说明用 `title`；它只用来补固定第一张卡，不单独插入；
    /// - 风格日推：左上角用 `coverText`，说明用 `title`；
    /// - 其他类型：左上角优先 `coverText`，说明用 `title`，相同就不重复显示。
    fn from_api(entry: &RecommendEntry) -> Option<Self> {
        let daily = entry.sub_resource_type == "dailySong";
        let style = entry.sub_resource_type == "style_dailySong";
        let kind = if daily {
            QuickEntryKind::DailySongs
        } else if style {
            QuickEntryKind::StyleDaily
        } else {
            match entry.resource_type.as_str() {
                "similarSong" => QuickEntryKind::SimilarSongs,
                "star" => QuickEntryKind::HeartMode,
                "fm" => QuickEntryKind::Roam,
                _ => QuickEntryKind::Other,
            }
        };
        let target = if daily {
            Some(QuickEntryTarget::DailySongs)
        } else if style {
            Some(QuickEntryTarget::StyleDaily {
                category_id: entry.category_id,
                tag_id: entry.tag_id,
                song_id: entry.song_id,
            })
        } else {
            match entry.resource_type.as_str() {
                "similarSong" => Some(QuickEntryTarget::SimilarSongs),
                "star" => Some(QuickEntryTarget::HeartMode),
                "fm" => Some(QuickEntryTarget::Roam),
                "playList" => Some(QuickEntryTarget::Playlist(entry.resource_id)),
                _ => None,
            }
        };
        let (title, caption) = if daily {
            ("每日推荐".to_owned(), entry.title.clone())
        } else {
            let title = if entry.cover_text.is_empty() {
                entry.title.clone()
            } else {
                entry.cover_text.clone()
            };
            let caption = if entry.title == title {
                String::new()
            } else {
                entry.title.clone()
            };
            (title, caption)
        };
        if title.is_empty() {
            return None;
        }
        Some(Self {
            kind,
            title: title.into(),
            caption: caption.into(),
            cover: entry.cover_url.clone().map(Into::into),
            target,
        })
    }
}

/// 固定第一张「每日推荐」的封面 / 文案用接口里那条 daily 补，而不是再插一张。
fn apply_daily(entry: &mut QuickEntry, daily: &RecommendEntry) {
    if let Some(cover) = &daily.cover_url {
        entry.cover = Some(cover.clone().into());
    }
    if !daily.title.is_empty() {
        entry.caption = daily.title.clone().into();
    }
}

/// 推荐页的 store。四块内容各发各的请求，互不阻塞：
/// 顶部入口和精选活动游客也能取；雷达歌单和个性化推荐歌单要登录态。
pub struct RecommendStore {
    pub entries: Vec<QuickEntry>,
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
    /// 刷新按钮置位：绕过「同一账号只取一次」的守卫，强制重取。
    force_reload: bool,
    _user_subscription: Option<Subscription>,
}

impl RecommendStore {
    pub fn new(user: Entity<AccountState>, cx: &mut Context<Self>) -> Self {
        // 账号变化（扫码登录 / 退出 / 启动时会话恢复完成）时重新加载。
        let subscription = cx.observe(&user, |this, user, cx| {
            this.load(user.read(cx).profile.user_id, cx);
        });
        let mut this = Self {
            entries: default_entries(),
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
            force_reload: false,
            _user_subscription: Some(subscription),
        };
        let (user_id, loading) = {
            let account = user.read(cx);
            (account.profile.user_id, account.loading)
        };
        // 会话还在恢复中（user_id == 0 且 loading）时先不取：
        // observe 会在账号落地后补上这一次加载。
        if user_id != 0 || !loading {
            this.load(user_id, cx);
        }
        this
    }

    /// 刷新按钮：重新拉一轮推荐内容。进行中的旧请求会被取消，
    /// 结果由 generation 挡掉，不会覆盖新一批。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.force_reload = true;
        self.load(self.user_id.unwrap_or_default(), cx);
    }

    fn load(&mut self, user_id: u64, cx: &mut Context<Self>) {
        let force = std::mem::take(&mut self.force_reload);
        if !self.should_load(user_id, force) {
            return;
        }
        self.abort_requests();
        // 换身份（登录 / 退出 / 换账号）：上一批内容不再属于现在这个人。
        if self.user_id != Some(user_id) {
            // 前五个固定入口始终在，先回到占位；接口回来后再补内容和风格日推。
            self.entries = default_entries();
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
        // `client` 同时被多个 spawn 用，先取成拥有值再进 spawn。
        let (runtime, client) = {
            let api = MusicApi::global(cx);
            (api.runtime.handle().clone(), api.client.clone())
        };

        // 顶部入口：官方首屏聚合接口，游客也能取，拿不到就保持空行。
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

        // 雷达歌单要登录态；未登录时入口内容为空，不显示区块。
        if user_id != 0 {
            let request = runtime.spawn(MusicApi::radar_playlists(client.clone()));
            self.radar_request = Some(request.abort_handle());
            cx.spawn(async move |this, cx| {
                let result = request
                    .await
                    .unwrap_or_else(|_| Err("雷达歌单请求任务失败".into()));
                let _ = this.update(cx, |this, cx| {
                    if this.apply_radar(generation, result) {
                        cx.notify();
                    }
                });
            })
            .detach();
        }

        // 推荐歌单：登录后走官方新接口；游客回落到首页区块里的「推荐歌单」。
        let guest_client = client.clone();
        let request = if user_id != 0 {
            runtime.spawn(MusicApi::recommended_playlists(client.clone()))
        } else {
            runtime.spawn(async move {
                MusicApi::homepage_blocks(guest_client).await.map(|blocks| {
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

        // 精选活动：运营位，游客也能取。
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

    /// 是否该为当前身份取一次：同一身份取过（或正在取）就不再发，
    /// 只有上次失败才允许在下次账号通知时重试；`force` 供刷新按钮绕过守卫。
    fn should_load(&self, user_id: u64, force: bool) -> bool {
        force || self.user_id != Some(user_id) || self.error.is_some()
    }

    /// 顶部入口落地。`generation` 过期（切账号 / 刷新取消了旧请求）时整份丢弃。
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
        // 前五个固定入口始终保留；接口只做两件事：
        // 1. 用 daily 那条补第一张「每日推荐」的封面 / 文案；
        // 2. 用 style_dailySong（可能多条）替换第 6 张「风格日推」占位。
        // 「常听歌手」保持在最后。
        let mut entries = default_entries();
        if let Some(daily) = api_entries
            .iter()
            .find(|entry| entry.sub_resource_type == "dailySong")
        {
            apply_daily(&mut entries[0], daily);
        }
        let dynamic: Vec<QuickEntry> = api_entries
            .iter()
            .filter(|entry| entry.sub_resource_type != "dailySong")
            .filter_map(QuickEntry::from_api)
            .collect();
        if !dynamic.is_empty() {
            entries.splice(5..6, dynamic);
        }
        self.entries = entries;
        self.apply_radar_to_entry();
        true
    }

    /// 雷达歌单落地；失败不报错，区块保持空。
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
            self.apply_radar_to_entry();
        }
        true
    }

    /// 把雷达接口的第一张歌单填进固定的「私人雷达」入口。雷达和入口是两条
    /// 独立的请求，谁先回来都要能补上，所以两边落地时都调一次。
    fn apply_radar_to_entry(&mut self) {
        let Some(first) = self.radar.first().cloned() else {
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
        entry.target = Some(QuickEntryTarget::Playlist(first.id));
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

    /// 精选活动落地；失败不报错，区块保持空。
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
    use super::{QuickEntryKind, QuickEntryTarget, RecommendStore, default_entries};
    use crate::models::{FeaturedActivity, RecommendEntry, RecommendedPlaylist};

    fn store() -> RecommendStore {
        RecommendStore {
            entries: default_entries(),
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
            force_reload: false,
            _user_subscription: None,
        }
    }

    fn entry(sub_resource_type: &str, title: &str, cover_text: &str) -> RecommendEntry {
        RecommendEntry {
            title: title.into(),
            cover_text: cover_text.into(),
            cover_url: Some(format!("cover-{sub_resource_type}")),
            target_url: Some("orpheus://songrcmd?tab=default".into()),
            resource_type: "dailySongs".into(),
            sub_resource_type: sub_resource_type.into(),
            resource_id: 1,
            ..Default::default()
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
            link: Some("orpheus://album/1".into()),
        }
    }

    #[test]
    fn same_identity_loads_once_until_it_fails() {
        let mut this = store();
        // 没取过：该取。
        assert!(this.should_load(42, false));
        this.user_id = Some(42);
        // 同一身份（账号通知回来、会员信息变化）：不重复发。
        assert!(!this.should_load(42, false));
        // 上次失败：允许在下次通知时重试。
        this.error = Some("网络错误".into());
        assert!(this.should_load(42, false));
        // 刷新按钮强制重取，与成败无关。
        this.error = None;
        assert!(this.should_load(42, true));
        // 换账号要取。
        assert!(this.should_load(7, false));
        // 退出登录变成游客（0）也要取：顶部入口和公开推荐不需要登录态。
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
        // 5 个固定入口 + 2 个风格日推 + 常听歌手；daily 不再重复插一张。
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
        // 固定第一张「每日推荐」用接口的封面 / 文案补，但标题不变。
        assert_eq!(this.entries[0].title.as_ref(), "每日推荐");
        assert_eq!(
            this.entries[0].caption.as_ref(),
            "每日推荐 | 你的多种喜好推荐"
        );
        assert_eq!(this.entries[0].cover.as_deref(), Some("cover-dailySong"));
        // 风格日推：左上角用 coverText，说明用 title。
        assert_eq!(this.entries[5].title.as_ref(), "欧美流行日推");
        assert_eq!(this.entries[5].caption.as_ref(), "每日欧美流行推荐");
        assert!(matches!(
            this.entries[5].target,
            Some(QuickEntryTarget::StyleDaily { .. })
        ));
        // 常听歌手保持在最后。
        assert_eq!(this.entries[7].title.as_ref(), "常听歌手");
    }

    #[test]
    fn entries_are_dropped_when_the_generation_is_stale() {
        let mut this = store();
        this.generation = 4;
        assert!(!this.apply_entries(3, Ok(vec![entry("dailySong", "旧的一批", "1")])));
        // 旧结果被丢弃，首行仍是固定的 7 个占位。
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
        // 固定的「私人雷达」入口跟着雷达数据一起更新。
        let radar_entry = this
            .entries
            .iter()
            .find(|entry| entry.kind == QuickEntryKind::Radar)
            .expect("首行应当有私人雷达入口");
        assert_eq!(radar_entry.caption.as_ref(), "今天从《歌》听起");
        assert_eq!(radar_entry.cover.as_deref(), Some("cover-3"));
        assert_eq!(
            radar_entry.target,
            Some(QuickEntryTarget::Playlist(3))
        );
        // 雷达先回来不会把推荐歌单的 loading 提前关掉。
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
        assert!(this.apply_entries(
            1,
            Ok(vec![entry("dailySong", "每日推荐", "29")])
        ));
        let radar_entry = this
            .entries
            .iter()
            .find(|entry| entry.kind == QuickEntryKind::Radar)
            .expect("首行应当有私人雷达入口");
        assert_eq!(radar_entry.cover.as_deref(), Some("cover-9"));
        assert_eq!(
            radar_entry.target,
            Some(QuickEntryTarget::Playlist(9))
        );
    }

    #[test]
    fn featured_activities_only_replace_on_success() {
        let mut this = store();
        this.generation = 1;
        assert!(this.apply_featured(1, Ok(vec![activity("活动一")])));
        assert_eq!(this.featured.len(), 1);
        // 刷新失败保留上一批运营位，不要把页面打空。
        assert!(this.apply_featured(1, Err("失败".into())));
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
        // 刷新失败不清空已有列表：旧内容比空列表好。
        assert_eq!(this.playlists.len(), 1);
    }
}
