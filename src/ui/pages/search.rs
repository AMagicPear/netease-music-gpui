use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use crate::models::Song;
use crate::playback::PlaybackController;
use crate::state::library::MusicLibrary;
use crate::state::search_results::SearchResults;
use crate::ui::assets::track_cover_image;
use crate::ui::components::{
    artist_label, like_icon_path, quality_badge_path, spinner, virtual_table, TabBar, TabChanged,
    TabItem, TableColumn, CELL_PADDING,
};
use crate::ui::theme::IconSize;

use super::playlist::{
    centered_empty, play_all_button, row_hover_icon, title_with_subtitle, SongDisplay,
    ACTION_BUTTON_HEIGHT, SECONDARY_FONT_WEIGHT,
};

/// 「喜欢」「时长」列的固定宽度，与歌单页保持一致。
const LIKE_COLUMN_WIDTH: Pixels = px(42.);
const DURATION_COLUMN_WIDTH: Pixels = px(66.);
/// 序号列固定宽度：本页不做排序，行数不超过三位数。
const INDEX_COLUMN_WIDTH: Pixels = px(34.);

/// 与官方客户端一致的分类 tab 顺序。
const SEARCH_TABS: [&str; 10] = [
    "综合", "单曲", "歌单", "播客", "专辑", "歌手", "MV", "用户", "歌词", "声音",
];
/// 目前只有这两类能展示内容（都是同一份单曲结果）。
const SONG_TABS: [usize; 2] = [0, 1];
/// 单曲 tab 的下标。
const SONG_TAB: usize = 1;

/// 所有搜索共用一个 View；关键词和结果由独立的 Entity 管理。
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
    /// hover 中的行，用歌曲 id 标记，不随列表刷新错位。
    hovered_row: Option<u64>,
    /// 播放状态快照：变化时才重绘，播放进度不会频繁刷新整页。
    playback_state: (Option<u64>, bool, bool),
}

/// 列定义。搜索结果不做排序，表头就是纯文字。
fn table_columns() -> Rc<Vec<TableColumn>> {
    Rc::new(vec![
        TableColumn::new("#", Some(INDEX_COLUMN_WIDTH)).align_right(),
        TableColumn::new("标题", None).weight(0.65),
        TableColumn::new("专辑", None).weight(0.35),
        TableColumn::new("喜欢", Some(LIKE_COLUMN_WIDTH)),
        TableColumn::new("时长", Some(DURATION_COLUMN_WIDTH)),
    ])
}

impl SearchPage {
    pub fn new(
        library: Entity<MusicLibrary>,
        playback: Entity<PlaybackController>,
        results: Entity<SearchResults>,
        cx: &mut Context<Self>,
    ) -> Self {
        let tabs = cx.new(|_| {
            TabBar::new(SEARCH_TABS.iter().map(|label| TabItem::new(*label)).collect())
        });
        let tabs_subscription =
            cx.subscribe(&tabs, |_, _, _: &TabChanged, cx| cx.notify());
        // 换了关键词就把 tab 复位到「综合」，避免新结果挂在旧分类下。
        let results_subscription = cx.observe(&results, |this, results, cx| {
            let keywords = results.read(cx).keywords.clone();
            if this.keywords != keywords {
                this.keywords = keywords;
                this.tabs.update(cx, |tabs, cx| tabs.select(0, cx));
            }
            cx.notify();
        });
        let mut playback_state = Self::playback_state(&playback, cx);
        let playback_subscription = cx.observe(&playback, move |_, playback, cx| {
            let next = Self::playback_state(&playback, cx);
            if next != playback_state {
                playback_state = next;
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
            columns: table_columns(),
            hovered_row: None,
            playback_state,
        }
    }

    fn playback_state(playback: &Entity<PlaybackController>, cx: &App) -> (Option<u64>, bool, bool) {
        let playback = playback.read(cx);
        let snapshot = playback.snapshot();
        (
            snapshot.current_song.as_ref().map(|song| song.id),
            snapshot.is_playing,
            playback.is_play_requested(),
        )
    }

    /// 用当前结果整表作为播放队列，从 `song_id` 开始播放。
    fn play_from(&mut self, song_id: u64, cx: &mut Context<Self>) {
        let songs = self.results.read(cx).songs.clone();
        self.playback.update(cx, |playback, cx| {
            playback.play_list(0, songs, song_id, cx);
        });
    }

    fn song_cells(&mut self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = Theme::global(cx).tokens.colors;
        let library = self.library.read(cx);
        let song = self.results.read(cx).songs[index].clone();
        let display = SongDisplay::new(&song);
        let song_id = song.id;
        let liked = library.liked_song_ids.contains(&song_id);
        let (current_song_id, is_playing, play_requested) = self.playback_state;
        let is_current_song = current_song_id == Some(song_id);
        let hovered = self.hovered_row == Some(song_id);
        // 序号格的固定部分：两种内容（序号 / 播放键）共用同一套宽度和对齐方式。
        let index_cell = div()
            .w_full()
            .flex()
            .justify_end()
            .whitespace_nowrap()
            .font_family(crate::ui::theme::DOLPHIN_FAMILY);
        vec![
            if is_current_song && play_requested && hovered {
                index_cell
                    .child(
                        div()
                            .id(("search-pause-song", song_id))
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
                // 正在播放的行：主题红高亮标题已经足够，序号位置留空。
                index_cell
                    .text_color(colors.primary)
                    .child("·")
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
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .child(
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
                                    .child(title_with_subtitle(display, colors)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .min_w(px(0.))
                                    .when_some(song.best_quality_level(), |row, level| {
                                        row.child(
                                            img(quality_badge_path(level)).h(px(13.)).flex_none(),
                                        )
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
                                            .child(artist_label(song, colors)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for SearchPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let results = self.results.read(cx);
        let keywords = results.keywords.clone();
        let content = if results.loading {
            div()
                .id("search-loading")
                .aria_label("加载中")
                .w_full()
                .py(px(36.))
                .flex()
                .justify_center()
                .child(spinner("search-loading-animation", 20., colors.muted_foreground))
                .into_any_element()
        } else if let Some(error) = results.error.clone() {
            div()
                .flex()
                .items_center()
                .text_color(colors.muted_foreground)
                .child(error)
                .child(
                    Button::new("search-retry")
                        .ml_2()
                        .child("重试")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let keywords = this.results.read(cx).keywords.clone();
                            if let Some(keywords) = keywords {
                                this.results
                                    .update(cx, |results, cx| results.search(keywords, cx));
                            }
                        })),
                )
                .into_any_element()
        } else if keywords.is_none() || results.songs.is_empty() {
            centered_empty(
                "images/empty/cm_empty_wuzuopin.png",
                if keywords.is_none() {
                    "搜索你感兴趣的音乐"
                } else {
                    "没有找到相关内容"
                },
                colors,
            )
            .into_any_element()
        } else {
            // 虚拟表格只渲染可见行，结果再多也不拖慢滚动。
            let table = virtual_table(
                cx.entity(),
                "search-songs",
                self.columns.clone(),
                results.songs.len(),
                |this, index, _, cx| this.song_cells(index, cx),
                |this, index, cx| this.results.read(cx).songs[index].id,
                |column, _| {
                    div()
                        .px(CELL_PADDING)
                        .child(column.title.clone())
                        .into_any_element()
                },
                |_| {},
                |this, key, hovered, cx| {
                    let next = hovered.then_some(key);
                    if this.hovered_row != next {
                        this.hovered_row = next;
                        cx.notify();
                    }
                },
                // 行上右键与歌单页一致：弹出同一份歌曲原生菜单。
                |_, _, position, window, cx| {
                    crate::ui::components::show_song_menu(position, window, cx)
                },
                cx,
            );
            // 分类 tab 目前只有「综合 / 单曲」有内容：都是同一份单曲结果；
            // 其余分类先给占位空状态，等后续接入对应接口。
            let is_songs_tab = SONG_TABS.contains(&self.tabs.read(cx).selected_index());
            // 「播放全部」只挂在单曲 tab：综合页是聚合入口，不承担整表播放。
            let is_song_list_tab = self.tabs.read(cx).selected_index() == SONG_TAB;
            let tab_content = if is_songs_tab {
                table.into_any_element()
            } else {
                centered_empty(
                    "images/empty/cm_empty_wuzuopin.png",
                    "该分类暂未支持，先看看单曲吧",
                    colors,
                )
                .into_any_element()
            };
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(12.))
                // 搜索词大标题，与官方客户端一致。
                .child(
                    div()
                        .text_size(px(24.))
                        .line_clamp(1)
                        .font_weight(FontWeight::BOLD)
                        .text_color(colors.foreground)
                        .child(keywords.clone().unwrap_or_default()),
                )
                // tab 行：左侧分类，右侧「播放全部」（只在单曲分类显示）。
                // 容器固定为按钮同一高度：按钮时有时无时行高不会跟着塌陷。
                .child(
                    div()
                        .h(ACTION_BUTTON_HEIGHT)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(self.tabs.clone())
                        .when(is_song_list_tab, |row| {
                            row.child(play_all_button(colors, false).on_click(cx.listener(
                                |this, _, _, cx| {
                                    let songs = this.results.read(cx).songs.clone();
                                    this.playback.update(cx, |playback, cx| {
                                        playback.play_list_from_start(0, songs, cx);
                                    });
                                },
                            )))
                        }),
                )
                .child(tab_content)
                .into_any_element()
        };

        div().w_full().flex().flex_col().child(content)
    }
}
