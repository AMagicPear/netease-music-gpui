//! 每日推荐页：日期标题 + 「播放全部」+ 虚拟歌曲表。
//!
//! 与歌单页的区别只在数据来源和页头：这里没有详情、评论和收藏者，
//! 所以直接复用搜索页的「表格 + 行」写法，不引入歌单页那套详情状态机。

use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};
use gpui_kit::component::native_menu::NativeMenu;

use crate::models::Song;
use crate::playback::{PlaybackController, PlaybackEvent};
use crate::state::{daily::DailySongsStore, library::MusicLibrary};
use crate::ui::assets::track_cover_image;
use crate::ui::components::{
    ACTION_BUTTON_HEIGHT, ACTION_BUTTON_PADDING, CELL_PADDING, DURATION_COLUMN_WIDTH, KeepRowHover,
    LIKE_COLUMN_WIDTH, PlayingIndicator, SECONDARY_FONT_WEIGHT, SongDisplay, TableColumn,
    artist_label, centered_empty, hovered_row_after, index_column_width, like_icon_path,
    play_all_button, quality_badge_path, row_hover_icon, spinner, title_with_subtitle,
    virtual_table,
};
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_OPACITY};

actions!(daily, [Share, BatchOperation, AddAllToPlaylist]);

/// 日推不是歌单，没有可用的歌单 ID；队列来源用 0，与搜索页一致。
const DAILY_SOURCE: u64 = 0;

fn table_columns(row_count: usize) -> Rc<Vec<TableColumn>> {
    Rc::new(vec![
        TableColumn::new("#", Some(index_column_width(row_count))).align_right(),
        TableColumn::new("标题", None).weight(0.65),
        TableColumn::new("专辑", None).weight(0.35),
        TableColumn::new("喜欢", Some(LIKE_COLUMN_WIDTH)),
        TableColumn::new("时长", Some(DURATION_COLUMN_WIDTH)),
    ])
}

pub struct DailySongsPage {
    store: Entity<DailySongsStore>,
    _store_subscription: Subscription,
    library: Entity<MusicLibrary>,
    _library_subscription: Subscription,
    playback: Entity<PlaybackController>,
    _playback_subscription: Subscription,
    playing_indicator: PlayingIndicator,
    columns: Rc<Vec<TableColumn>>,
    hovered_row: Option<u64>,
}

impl DailySongsPage {
    pub fn new(
        store: Entity<DailySongsStore>,
        library: Entity<MusicLibrary>,
        playback: Entity<PlaybackController>,
        playing_indicator: PlayingIndicator,
        cx: &mut Context<Self>,
    ) -> Self {
        let store_subscription = cx.observe(&store, |this, store, cx| {
            // 序号列宽跟着行数走，数据换了要重新算列定义。
            this.columns = table_columns(store.read(cx).songs.len());
            cx.notify();
        });
        // 喜欢状态、当前播放歌曲都在行渲染时现读；这里只负责触发重绘。
        let library_subscription = cx.observe(&library, |_, _, cx| cx.notify());
        let playback_subscription = cx.subscribe(&playback, |_, _, event, cx| {
            if matches!(event, PlaybackEvent::SnapshotChanged) {
                cx.notify();
            }
        });
        let row_count = store.read(cx).songs.len();
        Self {
            store,
            _store_subscription: store_subscription,
            library,
            _library_subscription: library_subscription,
            playback,
            _playback_subscription: playback_subscription,
            playing_indicator,
            columns: table_columns(row_count),
            hovered_row: None,
        }
    }

    fn play_from(&mut self, song_id: u64, cx: &mut Context<Self>) {
        let songs = self.store.read(cx).songs.clone();
        self.playback.update(cx, |playback, cx| {
            playback.play_list(DAILY_SOURCE, songs, song_id, cx);
        });
    }

    fn set_hovered_row(&mut self, id: u64, hovered: bool, cx: &mut Context<Self>) {
        let next = hovered_row_after(self.hovered_row, id, hovered);
        if next != self.hovered_row {
            self.hovered_row = next;
            cx.notify();
        }
    }

    fn header(&self, colors: ColorTokens, cx: &mut Context<Self>) -> AnyElement {
        let count = self.store.read(cx).songs.len();
        let today =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(5.))
                            .font_family(DOLPHIN_FAMILY)
                            .text_color(colors.foreground)
                            .child(div().text_size(px(40.)).child(today.day().to_string()))
                            .child(
                                div()
                                    .mb(px(5.))
                                    .flex()
                                    .items_end()
                                    .gap(px(4.))
                                    .text_size(px(20.))
                                    .child("/")
                                    .child(u8::from(today.month()).to_string()),
                            ),
                    )
                    .child(
                        div()
                            .mb(px(4.))
                            .text_size(px(22.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(colors.foreground)
                            .child("每日推荐"),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(colors.muted_foreground)
                    .child("根据你的音乐口味生成，每天6:00更新"),
            )
            .when(count > 0, |row| {
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(play_all_button(colors, false).on_click(cx.listener(
                            |this, _, _, cx| {
                                let songs = this.store.read(cx).songs.clone();
                                this.playback.update(cx, |playback, cx| {
                                    playback.play_list_from_start(DAILY_SOURCE, songs, cx);
                                });
                            },
                        )))
                        .child(
                            Button::new("daily-download-button")
                                .h(ACTION_BUTTON_HEIGHT)
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(px(4.))
                                .px(ACTION_BUTTON_PADDING)
                                .border_1()
                                .border_color(colors.foreground.alpha(0.06))
                                .rounded_lg()
                                .bg(colors.foreground.alpha(0.03))
                                .hover(|style| style.bg(colors.foreground.alpha(0.08)))
                                .active(|style| style.opacity(PRESSED_OPACITY))
                                .text_size(px(13.))
                                .text_color(colors.secondary_foreground)
                                .child(
                                    svg()
                                        .path("icons/download.svg")
                                        .size(px(18.))
                                        .flex_none()
                                        .text_color(colors.secondary_foreground),
                                )
                                .child("下载"),
                        )
                        .child(
                            daily_icon_button(
                                "daily-more-button",
                                "icons/xpoint.svg",
                                "更多",
                                colors,
                            )
                            .on_click(|event, window, cx| {
                                NativeMenu::new()
                                    .menu("分享…", Box::new(Share))
                                    .menu("批量操作", Box::new(BatchOperation))
                                    .menu("添加全部至播放列表", Box::new(AddAllToPlaylist))
                                    .show(event.position(), window, cx);
                            }),
                        ),
                )
            })
            .into_any_element()
    }

    fn body(&mut self, colors: ColorTokens, cx: &mut Context<Self>) -> AnyElement {
        let (loading, loaded, guest, error, empty) = {
            let store = self.store.read(cx);
            (
                store.loading,
                store.loaded,
                store.needs_login(),
                store.error.clone(),
                store.songs.is_empty(),
            )
        };
        if loading && empty {
            return div()
                .w_full()
                .py(px(36.))
                .flex()
                .justify_center()
                .child(spinner("daily-loading", 20., colors.muted_foreground))
                .into_any_element();
        }
        if guest {
            return centered_empty(
                "images/empty/cm_empty_wuzuopin.png",
                "登录后为你生成每日推荐",
                colors,
            )
            .into_any_element();
        }
        if !loaded && empty {
            return match error {
                Some(error) => div()
                    .w_full()
                    .py(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .text_size(px(14.))
                    .text_color(colors.muted_foreground)
                    .child(error)
                    .child(
                        Button::new("daily-retry")
                            .child("重试")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |store, cx| store.reload(cx));
                            })),
                    )
                    .into_any_element(),
                None => {
                    centered_empty("images/empty/cm_empty_wuzuopin.png", "暂无每日推荐", colors)
                        .into_any_element()
                }
            };
        }
        if empty {
            return centered_empty("images/empty/cm_empty_wuzuopin.png", "暂无每日推荐", colors)
                .into_any_element();
        }
        self.song_table(cx)
    }

    fn song_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        virtual_table(
            cx.entity(),
            "daily-songs",
            self.columns.clone(),
            self.store.read(cx).songs.len(),
            |this, index, _, cx| this.song_cells(index, cx),
            |this, index, cx| this.store.read(cx).songs[index].id,
            |column, _| {
                div()
                    .px(CELL_PADDING)
                    .child(column.title.clone())
                    .into_any_element()
            },
            |_| {},
            |this, id, hovered, cx| this.set_hovered_row(id, hovered, cx),
            |this, key, cx| this.play_from(key, cx),
            |this, key, position, window, cx| {
                let Some(song) = this
                    .store
                    .read(cx)
                    .songs
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

    fn song_cells(&mut self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = Theme::global(cx).tokens.colors;
        let song = self.store.read(cx).songs[index].clone();
        let display = SongDisplay::new(&song);
        let song_id = song.id;
        let liked = self.library.read(cx).liked_song_ids.contains(&song_id);
        // 播放指示现读快照：变化由 PlaybackEvent::SnapshotChanged 触发重绘。
        let playback = self.playback.read(cx);
        let snapshot = playback.snapshot();
        let is_current_song = snapshot
            .current_song
            .as_ref()
            .is_some_and(|current| current.id == song_id);
        let is_playing = snapshot.is_playing;
        let play_requested = playback.is_play_requested();
        let hovered = self.hovered_row == Some(song_id);
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
                            .id(("daily-pause-song", song_id))
                            .keep_row_hover()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.playback.update(cx, |playback, cx| playback.pause(cx));
                            }))
                            .child(row_hover_icon(
                                ("daily-song-pause", song_id),
                                "icons/pause.svg",
                                "暂停",
                                px(20.),
                                colors,
                            )),
                    )
                    .into_any_element()
            } else if hovered || (is_current_song && !play_requested) {
                // hover 时序号让位给播放键。
                index_cell
                    .child(
                        div()
                            .id(("daily-select-song", song_id))
                            .keep_row_hover()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.play_from(song_id, cx)),
                            )
                            .child(row_hover_icon(
                                ("daily-song-play", song_id),
                                "icons/play.svg",
                                "播放",
                                px(20.),
                                colors,
                            )),
                    )
                    .into_any_element()
            } else if is_current_song && is_playing {
                index_cell
                    .child(self.playing_indicator.anchor())
                    .into_any_element()
            } else {
                index_cell
                    .text_color(colors.muted_foreground)
                    .child(format!("{:02}", index + 1))
                    .into_any_element()
            },
            self.title_cell(&song, &display, is_current_song, colors),
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
                .id(("daily-song-like", song_id))
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

    /// 标题格：封面 + 标题/副标题 + 音质徽章，视觉与歌单、搜索页一致。
    fn title_cell(
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
                        cover.child(
                            img(track_cover_image(source))
                                .size_full()
                                .rounded(px(4.))
                                .object_fit(ObjectFit::Cover),
                        )
                    }),
            )
            .child(
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

fn daily_icon_button(
    id: &'static str,
    path: &'static str,
    label: &'static str,
    colors: ColorTokens,
) -> Button {
    Button::new(id)
        .aria_label(label)
        .size(ACTION_BUTTON_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(colors.foreground.alpha(0.06))
        .rounded_lg()
        .bg(colors.foreground.alpha(0.03))
        .hover(|style| style.bg(colors.foreground.alpha(0.08)))
        .active(|style| style.opacity(PRESSED_OPACITY))
        .child(
            svg()
                .path(path)
                .size(px(16.))
                .flex_none()
                .text_color(colors.secondary_foreground),
        )
}

impl Render for DailySongsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.playing_indicator.clear();
        let colors = Theme::global(cx).tokens.colors;
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(self.header(colors, cx))
            .child(self.body(colors, cx))
    }
}
