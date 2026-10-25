use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Scrollbar, ScrollbarMode, Theme};

use crate::models::{Playlist, Song};
use crate::playback::{PlaybackController, PlaybackEvent};
use crate::state::library::MusicLibrary;
use crate::state::search_results::SearchResults;
use crate::ui::assets::{track_cover_image, track_cover_url};
use crate::ui::components::{
    ACTION_BUTTON_HEIGHT, CELL_PADDING, DURATION_COLUMN_WIDTH, KeepRowHover, LIKE_COLUMN_WIDTH,
    PAGE_SIDE_PADDING, PlayingIndicator, ROW_HEIGHT, SECONDARY_FONT_WEIGHT, SongDisplay, TabBar,
    TabChanged, TabItem, TableColumn, artist_label, centered_empty, hovered_row_after,
    index_column_width, like_icon_path, play_all_button, play_count_label, quality_badge_path,
    row_hover_icon, spinner, title_with_subtitle, virtual_table,
};
use crate::ui::events::{Navigate, NavigationBus};
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize};

/// 歌单表「歌曲数」「播放量」的固定宽度。两列内容最长是「1234万」「1163首」
/// （约 44px），给到 56px 留一点余量；再宽只会挤占标题和创建者的弹性空间，
/// 不会再产生右缘空洞（两列弹性权重已和为 1，整表铺满）。
const TRACK_COUNT_COLUMN_WIDTH: Pixels = px(56.);
const PLAY_COUNT_COLUMN_WIDTH: Pixels = px(56.);

/// 一个分类 tab 的语义。下标只是「第几栏」，不表达「这栏该显示什么」，
/// 这里补上这份对应关系。
#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchTab {
    /// 综合 / 单曲：共用同一份单曲结果。
    Songs,
    /// 歌单：单独一份结果，切到这栏时才去取。
    Playlists,
    /// 还没接的分类，先给占位空状态。
    Unsupported,
}

/// 与官方客户端一致的分类 tab 顺序。
///
/// 标签和语义写在同一个数组里：下标就是 `TabBar` 的 `selected_index`，
/// 两份平行数组迟早会各自加一条、悄悄错位。
const SEARCH_TABS: [(SearchTab, &str); 10] = [
    (SearchTab::Songs, "综合"),
    (SearchTab::Songs, "单曲"),
    (SearchTab::Playlists, "歌单"),
    (SearchTab::Unsupported, "播客"),
    (SearchTab::Unsupported, "专辑"),
    (SearchTab::Unsupported, "歌手"),
    (SearchTab::Unsupported, "MV"),
    (SearchTab::Unsupported, "用户"),
    (SearchTab::Unsupported, "歌词"),
    (SearchTab::Unsupported, "声音"),
];

/// 单曲 tab 的下标：只有它带「播放全部」。
const SONG_TAB: usize = 1;

/// 搜索结果页。和歌单详情页一样只是「一个用 `virtual_table` 的列表页」，两者互不依赖：
/// 共用的表格长相和行内零件都在 `components::virtual_table` 里。
///
/// 布局上的区别：这里整页不滚动，搜索词标题和分类 tab 常驻，只有表格区域滚动。
pub struct SearchPage {
    results: Entity<SearchResults>,
    _results_subscription: Subscription,
    tabs: Entity<TabBar>,
    _tabs_subscription: Subscription,
    /// 上次见到的关键词：换词时把 tab 复位到「综合」。
    keywords: Option<String>,
    library: Entity<MusicLibrary>,
    playback: Entity<PlaybackController>,
    _playback_subscription: Subscription,
    columns: Rc<Vec<TableColumn>>,
    playlist_columns: Rc<Vec<TableColumn>>,
    /// 表格区域自己的滚动位置。只包住表格，不包标题和 tab。
    table_scroll: ScrollHandle,
    /// hover 中的行，用 id 标记，不随列表刷新错位。
    hovered_row: Option<u64>,
    playing_indicator: PlayingIndicator,
}

/// 单曲 / 综合的列定义。搜索结果不做排序，表头就是纯文字。
/// 序号列宽跟着行数算（和歌单页同一个函数），两边的序号列才一样宽。
fn table_columns(row_count: usize) -> Rc<Vec<TableColumn>> {
    Rc::new(vec![
        TableColumn::new("#", Some(index_column_width(row_count))).align_right(),
        TableColumn::new("标题", None).weight(0.65),
        TableColumn::new("专辑", None).weight(0.35),
        TableColumn::new("喜欢", Some(LIKE_COLUMN_WIDTH)),
        TableColumn::new("时长", Some(DURATION_COLUMN_WIDTH)),
    ])
}

/// 歌单的列定义，与官方客户端一致。
///
/// 「歌曲数」和「播放量」都是短数字，给固定宽度；其余宽度按权重分给标题和创建者，
/// 标题权重更大 —— 歌单名比创建者更容易长到需要截断。
///
/// 两列权重和必须是 1：GPUI 走 taffy 布局，而 taffy 沿用 CSS flexbox 的规矩——
/// 「权重和小于 1」时不会把剩余宽度分光，漏掉的 `(1 − 和)` 会以空洞堆在最后一列
/// 「播放量」右侧。和刚好为 1，整表才铺满、播放量紧贴右缘。
fn playlist_table_columns(row_count: usize) -> Rc<Vec<TableColumn>> {
    Rc::new(vec![
        TableColumn::new("#", Some(index_column_width(row_count))).align_right(),
        TableColumn::new("标题", None).weight(0.7),
        TableColumn::new("歌曲数", Some(TRACK_COUNT_COLUMN_WIDTH)),
        TableColumn::new("创建者", None).weight(0.3),
        TableColumn::new("播放量", Some(PLAY_COUNT_COLUMN_WIDTH)),
    ])
}

impl SearchPage {
    pub fn new(
        library: Entity<MusicLibrary>,
        playback: Entity<PlaybackController>,
        results: Entity<SearchResults>,
        playing_indicator: PlayingIndicator,
        cx: &mut Context<Self>,
    ) -> Self {
        let tabs = cx.new(|_| {
            TabBar::new(
                SEARCH_TABS
                    .iter()
                    .map(|(_, label)| TabItem::new(*label))
                    .collect(),
            )
        });
        let tabs_subscription = cx.subscribe(&tabs, |this, _, _: &TabChanged, cx| {
            // 单曲和歌单是两套编号，留着上一栏的行号会让新栏里同号的行凭空高亮。
            this.hovered_row = None;
            this.load_tab_results(cx);
        });
        // 换了关键词就把 tab 复位到「综合」，避免新结果挂在旧分类下。
        let results_subscription = cx.observe(&results, |this, results, cx| {
            let keywords = results.read(cx).keywords.clone();
            if this.keywords != keywords {
                this.keywords = keywords;
                this.tabs.update(cx, |tabs, cx| tabs.select(0, cx));
            }
            // 序号列宽跟着行数走，结果换了要重新算一遍列定义。
            let results = results.read(cx);
            this.columns = table_columns(results.songs().len());
            this.playlist_columns = playlist_table_columns(results.playlists().len());
            cx.notify();
        });
        // 行首的播放指示只跟「哪首在播、是否在播」走，都是 SnapshotChanged 的份内事；
        // 位置推进（10 次/秒）不打到这，列表不会跟着进度条白闪。
        let playback_subscription = cx.subscribe(&playback, move |_, _, event, cx| {
            if matches!(event, PlaybackEvent::SnapshotChanged) {
                cx.notify();
            }
        });
        Self {
            results,
            _results_subscription: results_subscription,
            tabs,
            _tabs_subscription: tabs_subscription,
            keywords: None,
            library,
            playback,
            _playback_subscription: playback_subscription,
            columns: table_columns(0),
            playlist_columns: playlist_table_columns(0),
            table_scroll: ScrollHandle::default(),
            hovered_row: None,
            playing_indicator,
        }
    }

    /// 用当前结果整表作为播放队列，从 `song_id` 开始播放。
    fn play_from(&mut self, song_id: u64, cx: &mut Context<Self>) {
        let songs = self.results.read(cx).songs().to_vec();
        self.playback.update(cx, |playback, cx| {
            playback.play_list(0, songs, song_id, cx);
        });
    }

    /// 切分类时才去取这一栏的结果。歌单是懒加载的：搜索时顺手多发一次请求，
    /// 对只看单曲的人是纯浪费往返。
    fn load_tab_results(&mut self, cx: &mut Context<Self>) {
        match SEARCH_TABS[self.tabs.read(cx).selected_index()].0 {
            SearchTab::Songs => self
                .results
                .update(cx, |results, cx| results.refresh_songs(cx)),
            SearchTab::Playlists => self
                .results
                .update(cx, |results, cx| results.search_playlists(cx)),
            SearchTab::Unsupported => {}
        }
        cx.notify();
    }

    /// 搜索词大标题 + 分类 tab 行。没搜过时整块都不出现。
    fn result_header(
        &mut self,
        kind: SearchTab,
        tab: usize,
        colors: ColorTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(keywords) = self.results.read(cx).keywords.clone() else {
            return div().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            // 和表格用同一个留白常量，标题、tab、表格左边缘才对得齐。
            .px(PAGE_SIDE_PADDING)
            .child(
                div()
                    .text_size(px(24.))
                    .line_clamp(1)
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.foreground)
                    .child(keywords),
            )
            // tab 行：左侧分类，右侧「播放全部」（只在单曲分类显示）。
            // 按按钮高度定行高，按钮时有时无时不会塌陷。
            .child(
                div()
                    .h(ACTION_BUTTON_HEIGHT)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(self.tabs.clone())
                    .when(kind == SearchTab::Songs && tab == SONG_TAB, |row| {
                        row.child(play_all_button(colors, false).on_click(cx.listener(
                            |this, _, _, cx| {
                                let songs = this.results.read(cx).songs().to_vec();
                                this.playback.update(cx, |playback, cx| {
                                    playback.play_list_from_start(0, songs, cx);
                                });
                            },
                        )))
                    }),
            )
            .into_any_element()
    }

    /// 分类 tab 下面的列表区：加载中 / 出错 / 空结果 / 表格都在这里收敛。
    /// 状态按分类各存一份，所以切栏不会把另一栏的结果也变成加载中。
    fn tab_list(
        &mut self,
        kind: SearchTab,
        colors: ColorTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 还没接的分类直接给占位，和搜没搜过无关。
        if kind == SearchTab::Unsupported {
            return centered_empty(
                "images/empty/cm_empty_wuzuopin.png",
                "该分类暂未支持，先看看单曲吧",
                colors,
            )
            .into_any_element();
        }
        let (loading, error, empty) = {
            let results = self.results.read(cx);
            match kind {
                SearchTab::Songs => (
                    results.songs.is_loading() && results.songs.value().is_none(),
                    results
                        .songs
                        .value()
                        .is_none()
                        .then(|| results.songs.error())
                        .flatten()
                        .map(|_| "搜索加载失败，请重试".to_string()),
                    results.songs().is_empty(),
                ),
                _ => (
                    results.playlists.is_loading() && results.playlists.value().is_none(),
                    results
                        .playlists
                        .value()
                        .is_none()
                        .then(|| results.playlists.error())
                        .flatten()
                        .map(|_| "搜索加载失败，请重试".to_string()),
                    results.playlists().is_empty(),
                ),
            }
        };
        let searched = self.results.read(cx).keywords.is_some();
        if loading {
            return div()
                .w_full()
                .py(px(36.))
                .flex()
                .justify_center()
                .child(spinner(
                    "search-loading-animation",
                    20.,
                    colors.muted_foreground,
                ))
                .into_any_element();
        }
        if let Some(error) = error {
            return div()
                .flex()
                .items_center()
                .text_color(colors.muted_foreground)
                .child(error)
                .child(
                    Button::new("search-retry")
                        .ml_2()
                        .child("重试")
                        .on_click(cx.listener(move |this, _, _, cx| this.retry(kind, cx))),
                )
                .into_any_element();
        }
        if !searched || empty {
            return centered_empty(
                "images/empty/cm_empty_wuzuopin.png",
                if searched {
                    "没有找到相关内容"
                } else {
                    "搜索你感兴趣的音乐"
                },
                colors,
            )
            .into_any_element();
        }
        let table = if kind == SearchTab::Songs {
            self.song_table(cx)
        } else {
            self.playlist_table(cx)
        };
        // 表格区域自己滚动。留白加在滚动容器自己身上而不是父级：padding 在裁剪框内部，
        // 行的 hover 投影才有地方画到行外面去；父级不留白，滚动条才落在页面右边缘。
        div()
            .relative()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .id("search-table-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .px(PAGE_SIDE_PADDING)
                    .overflow_y_scroll()
                    .track_scroll(&self.table_scroll)
                    .child(table),
            )
            .child(Scrollbar::vertical(&self.table_scroll).mode(ScrollbarMode::Always))
            .into_any_element()
    }

    /// 出错后的重试。关键词没变，所以只重取当前这一栏，
    /// 不该把已经拿到手的另一种结果也一起清掉。
    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        self.load_tab_results(cx);
    }

    fn retry(&mut self, kind: SearchTab, cx: &mut Context<Self>) {
        match kind {
            SearchTab::Songs => {
                if let Some(keywords) = self.results.read(cx).keywords.clone() {
                    self.results
                        .update(cx, |results, cx| results.search(keywords, cx));
                }
            }
            SearchTab::Playlists => self
                .results
                .update(cx, |results, cx| results.search_playlists(cx)),
            SearchTab::Unsupported => {}
        }
    }

    fn song_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        virtual_table(
            cx.entity(),
            "search-songs",
            self.columns.clone(),
            self.results.read(cx).songs().len(),
            |this, index, _, cx| this.song_cells(index, cx),
            |this, index, cx| this.results.read(cx).songs()[index].id,
            |column, _| {
                div()
                    .px(CELL_PADDING)
                    .child(column.title.clone())
                    .into_any_element()
            },
            |_| {},
            |this, key, hovered, cx| this.set_hovered_row(key, hovered, cx),
            |this, key, cx| this.play_from(key, cx),
            // 行上右键与歌单页一致：弹出同一份歌曲原生菜单。
            |this, key, position, window, cx| {
                let Some(song) = this
                    .results
                    .read(cx)
                    .songs()
                    .iter()
                    .find(|song| song.id == key)
                    .cloned()
                else {
                    return;
                };
                crate::ui::components::show_song_menu(position, song, window, cx)
            },
            cx,
        )
        .into_any_element()
    }

    fn playlist_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        virtual_table(
            cx.entity(),
            "search-playlists",
            self.playlist_columns.clone(),
            self.results.read(cx).playlists().len(),
            |this, index, _, cx| this.playlist_cells(index, cx),
            |this, index, cx| this.results.read(cx).playlists()[index].id,
            |column, _| {
                div()
                    .px(CELL_PADDING)
                    .child(column.title.clone())
                    .into_any_element()
            },
            |_| {},
            |this, key, hovered, cx| this.set_hovered_row(key, hovered, cx),
            |_, _, _| {},
            // 歌单行还没有对应的原生菜单（官方客户端给的是「分享 / 收藏」），
            // 先不接右键，等菜单真做好了再挂上。
            |_, _, _, _, _| {},
            cx,
        )
        .into_any_element()
    }

    /// hover 转移规则和歌单页共用一份，避免两边的表现分叉。
    fn set_hovered_row(&mut self, id: u64, hovered: bool, cx: &mut Context<Self>) {
        let next = hovered_row_after(self.hovered_row, id, hovered);
        if next != self.hovered_row {
            self.hovered_row = next;
            cx.notify();
        }
    }

    /// 离开本页时把表格滚回顶部，否则下次回来会停在上次滚到的那一行。
    pub fn reset_scroll(&mut self) {
        self.table_scroll.set_offset(point(px(0.), px(0.)));
    }

    fn playlist_cells(&mut self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = Theme::global(cx).tokens.colors;
        let playlist = self.results.read(cx).playlists()[index].clone();
        vec![
            div()
                .w_full()
                .flex()
                .justify_end()
                .whitespace_nowrap()
                .font_family(DOLPHIN_FAMILY)
                .text_color(colors.muted_foreground)
                .child(format!("{:02}", index + 1))
                .into_any_element(),
            self.playlist_title_cell(&playlist, colors, cx),
            // 「歌曲数」和「播放量」都是短数字，用和时长一样的弱化灰。
            // nowrap：定宽列宁可裁掉也不折行，折行会顶破 56px 的固定行高。
            div()
                .whitespace_nowrap()
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(colors.foreground.alpha(0.45))
                .child(format!("{}首", playlist.track_count))
                .into_any_element(),
            div()
                .truncate()
                .text_color(colors.secondary_foreground)
                .child(playlist.creator.nickname.clone())
                .into_any_element(),
            // 表头和单元格都左对齐（列定义没加 align_right），数字贴着列左缘，
            // 和「歌曲数」一列同行对齐更整齐。
            div()
                .w_full()
                .whitespace_nowrap()
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(colors.foreground.alpha(0.45))
                .child(play_count_label(playlist.play_count))
                .into_any_element(),
        ]
    }

    /// 歌单标题格：方形封面 + 歌单名。整格都是入口，和官方客户端一样点封面或名字进歌单；
    /// 撑到 `ROW_HEIGHT` 是为了让上下留白也算点击区。
    fn playlist_title_cell(
        &mut self,
        playlist: &Playlist,
        colors: ColorTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let playlist_id = playlist.id;
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .min_w(px(0.))
            .h(ROW_HEIGHT)
            .id(("search-playlist-open", playlist_id))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.open_playlist(playlist_id, cx)))
            .child(
                div()
                    .size(px(40.))
                    .flex_none()
                    .rounded(px(4.))
                    .overflow_hidden()
                    .bg(colors.muted)
                    .child(
                        // track_cover_url 顺手做两件事：空地址换默认封面，真实地址拼上
                        // CDN 缩放参数，不必下载原图再在本地缩。
                        img(track_cover_image(track_cover_url(
                            playlist.cover_img_url.as_deref(),
                            80,
                        )))
                        .size_full()
                        // GPUI 的 overflow_hidden 按矩形裁剪，圆角要直接设在图片上。
                        .rounded(px(4.))
                        .object_fit(ObjectFit::Cover),
                    ),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(px(14.))
                    .text_color(colors.foreground)
                    .child(playlist.name.clone()),
            )
            .into_any_element()
    }

    /// 点开搜索结果里的歌单。摘要按 id 现取：闭包要活到点击那一刻，
    /// 在渲染行时就克隆一份挂进去，可能挂着的正好是上一次搜索的旧副本。
    fn open_playlist(&mut self, playlist_id: u64, cx: &mut Context<Self>) {
        let Some(summary) = self
            .results
            .read(cx)
            .playlists()
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .cloned()
        else {
            return;
        };
        NavigationBus::emit(
            cx,
            Navigate::Playlist {
                id: playlist_id,
                summary: Some(summary),
            },
        );
    }

    fn song_cells(&mut self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = Theme::global(cx).tokens.colors;
        let library = self.library.read(cx);
        let song = self.results.read(cx).songs()[index].clone();
        let display = SongDisplay::new(&song);
        let song_id = song.id;
        let liked = library.is_liked(song_id);
        // 播放指示渲染时现读快照：变化由 PlaybackEvent::SnapshotChanged 触发重绘，
        // 不再本地存一份元组镜像。
        let playback = self.playback.read(cx);
        let snapshot = playback.snapshot();
        let current_song_id = snapshot.current_song.as_ref().map(|song| song.id);
        let is_playing = snapshot.is_playing;
        let play_requested = playback.is_play_requested();
        let is_current_song = current_song_id == Some(song_id);
        let hovered = self.hovered_row == Some(song_id);
        // 序号格的固定部分：两种内容（序号 / 播放键）共用同一套宽度和对齐方式。
        let index_cell = div()
            .w_full()
            .flex()
            .justify_end()
            .whitespace_nowrap()
            .font_family(DOLPHIN_FAMILY);
        vec![
            if is_current_song && play_requested && hovered {
                index_cell
                    .child(
                        div()
                            .id(("search-pause-song", song_id))
                            .keep_row_hover()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.playback.update(cx, |playback, cx| playback.pause(cx));
                            }))
                            .child(row_hover_icon(
                                ("search-song-pause", song_id),
                                "icons/pause.svg",
                                "暂停",
                                px(20.),
                                colors,
                            )),
                    )
                    .into_any_element()
            } else if hovered || (is_current_song && !play_requested) {
                // hover 时序号让位给播放键
                index_cell
                    .child(
                        div()
                            .id(("search-select-song", song_id))
                            .keep_row_hover()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.play_from(song_id, cx)),
                            )
                            .child(row_hover_icon(
                                ("search-song-play", song_id),
                                "icons/play.svg",
                                "播放",
                                px(20.),
                                colors,
                            )),
                    )
                    .into_any_element()
            } else if is_current_song && is_playing {
                // 与歌单、日推共用同一个播放动效。
                index_cell
                    .child(self.playing_indicator.anchor())
                    .into_any_element()
            } else {
                index_cell
                    .text_color(colors.muted_foreground)
                    .child(format!("{:02}", index + 1))
                    .into_any_element()
            },
            self.song_title_cell(&song, &display, is_current_song, colors),
            div()
                .truncate()
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(if song.al.name.is_some() {
                    colors.secondary_foreground
                } else {
                    colors.foreground.alpha(0.45)
                })
                .child(display.album.clone())
                .into_any_element(),
            // 裸 SVG：已喜欢是实心红心，未喜欢是勾线灰心。
            svg()
                .path(like_icon_path(liked))
                .size(IconSize::Small.pixels())
                .text_color(if liked {
                    colors.primary
                } else {
                    colors.foreground.alpha(0.6)
                })
                .hover(move |style| {
                    if liked {
                        style.text_color(colors.primary.alpha(0.88))
                    } else {
                        style.text_color(colors.foreground)
                    }
                })
                .id(("search-song-like", song_id))
                .aria_label(if liked { "已喜欢" } else { "未喜欢" })
                .keep_row_hover()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.library
                        .update(cx, |library, cx| library.toggle_like(song.clone(), cx));
                }))
                .into_any_element(),
            div()
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(colors.foreground.alpha(0.45))
                .child(display.duration.clone())
                .into_any_element(),
        ]
    }

    /// 标题单元格：封面 + 标题/歌手两行 + hover 时的操作区，视觉与歌单页一致。
    fn song_title_cell(
        &self,
        song: &Song,
        display: &SongDisplay,
        is_current_song: bool,
        colors: ColorTokens,
    ) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .min_w(px(0.))
            .child(
                div()
                    .size(px(36.))
                    .flex_none()
                    .rounded(px(4.))
                    .overflow_hidden()
                    .bg(colors.muted)
                    .when_some(display.cover.clone(), |cover, source| {
                        // GPUI 的 overflow_hidden 按矩形裁剪，圆角需要直接设置在图片上。
                        cover.child(
                            img(track_cover_image(source))
                                .size_full()
                                .rounded(px(4.))
                                .object_fit(ObjectFit::Cover),
                        )
                    }),
            )
            .child(
                // 文字保持 flex_1 + min_w(0)，空间不足时截断文字，不改变列宽。
                div().min_w(px(0.)).flex_1().flex().items_center().child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(if is_current_song {
                                    colors.primary
                                } else {
                                    colors.foreground
                                })
                                .truncate()
                                // 搜索页的「命中」是服务端的模糊匹配，没有用户输入的
                                // 关键词可标，所以传空串只保留副标题样式。
                                .child(title_with_subtitle(display, "", colors)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .min_w(px(0.))
                                .when_some(song.best_quality_level(), |row, level| {
                                    row.child(img(quality_badge_path(level)).h(px(13.)).flex_none())
                                })
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .truncate()
                                        .font_weight(SECONDARY_FONT_WEIGHT)
                                        .text_color(if is_current_song {
                                            colors.primary
                                        } else {
                                            colors.muted_foreground
                                        })
                                        .child(artist_label(song, "", colors)),
                                ),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for SearchPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.playing_indicator.clear();
        let colors = Theme::global(cx).tokens.colors;
        let tab = self.tabs.read(cx).selected_index();
        let kind = SEARCH_TABS[tab].0;
        // 整页撑满内容区：只有表格区域滚动，标题和 tab 行必须拿到确定高度，
        // 否则下面那个 flex_1 的滚动容器会塌成 0。shell.rs 对本页关掉了页面滚动容器。
        div()
            .size_full()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(self.result_header(kind, tab, colors, cx))
            .child(self.tab_list(kind, colors, cx))
    }
}
