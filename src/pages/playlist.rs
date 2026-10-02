use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::native_menu::NativeMenu;
use std::cell::Cell;
use std::rc::Rc;

/// 操作按钮的统一高度
const ACTION_BUTTON_HEIGHT: Pixels = px(36.);
/// 带文字按钮的左右内边距，宽度靠它 + 内容自适应
const ACTION_BUTTON_PADDING: Pixels = px(12.);

/// Dolphin 里最宽的数字是 600/1000 em。
const DOLPHIN_WIDEST_DIGIT_EM: f32 = 0.6;

/// 表头和虚拟行独立布局，序号列必须共用宽度，并容得下播放图标。
fn index_column_width(row_count: usize) -> Pixels {
    // 末行渲染的是 `format!("{:02}", row_count)`，它就是最长的那个序号。
    let digits = format!("{:02}", row_count).chars().count() as f32;
    // 字号跟行内保持一致（ROW_TEXT_SIZE），以后改字号不用回来动这里。
    let width = digits * DOLPHIN_WIDEST_DIGIT_EM * f32::from(ROW_TEXT_SIZE);
    // 向上取整：小数宽度按设备像素取整后可能差一丁点，而这点差距就够让数字折行。
    px(width.ceil().max(20.))
}

/// 标题、专辑按剩余宽度的比例分配；拖动后保留该比例。
const ALBUM_SHARE_MIN: f32 = 0.1;
const ALBUM_SHARE_MAX: f32 = 0.8;

/// 次要文字使用常规字重。
const SECONDARY_FONT_WEIGHT: FontWeight = FontWeight::NORMAL;

/// 「喜欢」「时长」的固定宽度：两列都在分界线右边，反推分界线位置要用到它们。
const LIKE_COLUMN_WIDTH: Pixels = px(42.);
const DURATION_COLUMN_WIDTH: Pixels = px(66.);

/// 分界线的元素 id，同时用作它作为 hover group 的名字。
/// 两处必须完全一致 `group_hover` 才会响应，所以抽成常量而不是写两遍字面量。
const DIVIDER_ID: &str = "playlist-title-divider";

/// 起拖时的鼠标 x、专辑占比、两列可用宽度。
type DividerDragAnchor = Rc<Cell<(Pixels, f32, Pixels)>>;

use crate::assets::thumbnail_url;
use crate::components::{
    CELL_PADDING, COLUMN_GAP, HEADER_HEIGHT, ROW_TEXT_SIZE, ResizeDragPreview, TabBar, TabChanged,
    TabItem, TableColumn, artist_label, virtual_table,
};
use crate::state::{
    library::MusicLibrary, playback::PlaybackState, playlist_detail::PlaylistDetail, song::Song,
};
use crate::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA, PRESSED_OPACITY};

// 「更多」菜单里的三个命令。
actions!(playlist, [Share, BatchOperation, AddAllToPlaylist]);

#[derive(Clone, Copy)]
enum PlaylistTab {
    Songs,
    Comments,
    Collectors,
}

impl PlaylistTab {
    fn content_title(self) -> &'static str {
        match self {
            Self::Songs => "歌曲列表",
            Self::Comments => "评论",
            Self::Collectors => "收藏者",
        }
    }
}

/// 所有歌单共享一个 View；歌单数据由独立的 Entity 管理。
pub struct PlaylistPage {
    detail: Entity<PlaylistDetail>,
    _detail_subscription: Subscription,
    playlist_id: Option<u64>,
    tabs: Entity<TabBar>,
    _tabs_subscription: Subscription,
    library: Entity<MusicLibrary>,
    playback: Entity<PlaybackState>,
    _library_subscription: Subscription,
    /// 原始歌单顺序保存在 detail.songs；这里只保存显示顺序。
    display_order: Vec<usize>,
    sort: Option<(SongSort, bool)>,
    album_share: f32,
    hovered_row: Option<u64>,
}

/// 使用同一段 StyledText，标题和副标题一起截断，只出现一个省略号。
fn title_with_subtitle(song: &Song, colors: ColorTokens) -> StyledText {
    let Some(subtitle) = song.tns.first().or_else(|| song.alia.first()) else {
        return StyledText::new(song.name.clone());
    };

    let subtitle = format!("（{subtitle}）");
    let title_len = song.name.len();
    let mut line = song.name.clone();
    line.push_str(&subtitle);

    // highlight 先 blend 再 fade_out：先替换 RGB，再恢复 alpha。
    // 因此播放中的红色标题不会污染副标题，也无需预合成背景色。
    let subtitle_color = colors.muted_foreground;

    StyledText::new(line).with_highlights([(
        title_len..title_len + subtitle.len(),
        HighlightStyle {
            color: Some(subtitle_color.alpha(1.)),
            font_weight: Some(SECONDARY_FONT_WEIGHT),
            fade_out: Some(1. - subtitle_color.a),
            ..Default::default()
        },
    )])
}

fn quality_badge(song: &Song) -> Option<&'static str> {
    if song.hr.is_some() {
        Some("icons/音质选项/Hi-Res.svg")
    } else if song.sq.is_some() {
        Some("icons/音质选项/sq.svg")
    } else if song.h.as_ref().is_some_and(|quality| quality.br >= 320000) {
        Some("icons/音质选项/HQ.svg")
    } else {
        None
    }
}

/// 行 hover 时才出现的图标：序号位置的播放键、标题右侧那排操作按钮，共用这一套样式。
fn row_hover_icon(
    id: (&'static str, u64),
    path: &'static str,
    label: &'static str,
    size: Pixels,
    colors: ColorTokens,
) -> AnyElement {
    svg()
        .path(path)
        .size(size)
        .flex_none()
        .text_color(colors.foreground.alpha(0.6))
        .hover(|style| style.text_color(colors.foreground))
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        // active 属于 StatefulInteractiveElement，必须跟在 `.id()` 之后（此时是 Stateful<Svg>）。
        .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
        .into_any_element()
}

/// 行 hover 时贴在标题右侧的那排操作图标：下载 / 收藏 / 评论 / 更多。
fn row_actions(index: u64, colors: ColorTokens) -> AnyElement {
    // 四个图标只有 id / 路径 / 无障碍名不同，其余全都一样：尺寸用 header 那排的
    // `IconSize::Small`，灰度和三态交互由 `row_hover_icon` 统一给。
    let icon = |id: &'static str, path: &'static str, label: &'static str| {
        row_hover_icon((id, index), path, label, IconSize::Small.pixels(), colors)
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        // 间距抄 header 那排图标（图标之间 10px），左侧再加 10px，免得贴着标题文字。
        .gap(px(10.))
        .ml(px(10.))
        .child(icon(
            "playlist-song-download",
            "icons/download_outline.svg",
            "下载",
        ))
        .child(icon("playlist-song-collect", "icons/collect.svg", "收藏"))
        .child(icon("playlist-song-comment", "icons/comment.svg", "评论"))
        .child(icon("playlist-song-more", "icons/xpoint.svg", "更多"))
        .into_any_element()
}

impl PlaylistPage {
    pub fn new(
        library: Entity<MusicLibrary>,
        playback: Entity<PlaybackState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let detail = cx.new(|_| PlaylistDetail::default());
        let tabs = cx.new(|_| {
            TabBar::new(vec![
                TabItem::new("歌曲"),
                TabItem::new("评论"),
                TabItem::new("收藏者"),
            ])
        });
        let tabs_subscription = cx.subscribe(&tabs, |_, _, _: &TabChanged, cx| cx.notify());
        let library_subscription = cx.observe(&library, |_, _, cx| cx.notify());
        let detail_subscription = cx.observe(&detail, |this, detail, cx| {
            let detail = detail.read(cx);
            let changed = this.playlist_id != detail.id;
            if changed {
                this.playlist_id = detail.id;
                this.sort = None;
                this.hovered_row = None;
            }
            this.display_order = this.sort.map_or_else(
                || (0..detail.songs.len()).collect(),
                |(key, descending)| song_order(&detail.songs, key, descending),
            );
            let playlist = detail.playlist.as_ref();
            let counts = [
                playlist.map(|playlist| playlist.track_count.to_string()),
                playlist
                    .and_then(|playlist| playlist.comment_count)
                    .map(|count| count.to_string()),
                playlist.map(|playlist| playlist.subscribed_count.to_string()),
            ];
            this.tabs.update(cx, |tabs, cx| {
                if changed {
                    tabs.select(0, cx);
                }
                for (index, count) in counts.into_iter().enumerate() {
                    tabs.set_count(index, count, cx);
                }
            });
            cx.notify();
        });
        Self {
            detail,
            _detail_subscription: detail_subscription,
            playlist_id: None,
            tabs,
            _tabs_subscription: tabs_subscription,
            library,
            playback,
            _library_subscription: library_subscription,
            display_order: Vec::new(),
            sort: None,
            album_share: 0.35,
            hovered_row: None,
        }
    }

    pub fn open(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        let summary = self
            .library
            .read(cx)
            .playlists
            .iter()
            .find(|playlist| Some(playlist.id) == id)
            .cloned();
        self.detail
            .update(cx, |detail, cx| detail.open(id, summary, cx));
    }

    fn columns(&self) -> Rc<Vec<TableColumn>> {
        Rc::new(vec![
            TableColumn::new("#", Some(index_column_width(self.display_order.len()))).align_right(),
            TableColumn::new("标题", None).weight(1. - self.album_share),
            TableColumn::new("专辑", None).weight(self.album_share),
            TableColumn::new("喜欢", Some(LIKE_COLUMN_WIDTH)),
            TableColumn::new("时长", Some(DURATION_COLUMN_WIDTH)),
        ])
    }

    fn toggle_sort(&mut self, column: SongSort, cx: &mut Context<Self>) {
        self.sort = next_sort(self.sort, column);
        let songs = &self.detail.read(cx).songs;
        self.display_order = self.sort.map_or_else(
            || (0..songs.len()).collect(),
            |(key, descending)| song_order(songs, key, descending),
        );
        cx.notify();
    }

    fn select_song(&mut self, song_id: u64, cx: &mut Context<Self>) {
        let song = self
            .detail
            .read(cx)
            .songs
            .iter()
            .find(|song| song.id == song_id)
            .cloned();
        if let Some(song) = song {
            self.playback.update(cx, |playback, cx| {
                playback.select_song(song);
                cx.notify();
            });
        }
    }

    /// 布局后用实际宽度定位分界线，不保存会随窗口变化的像素宽度。
    fn title_divider(
        &self,
        colors: ColorTokens,
        layout: Rc<Cell<[Bounds<Pixels>; 2]>>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let share = self.album_share;
        let view = cx.entity();
        canvas(
            move |bounds, window, cx| {
                let [title, album] = layout.get();
                let available =
                    (title.size.width + album.size.width - CELL_PADDING * 4.).max(px(0.));
                let anchor: DividerDragAnchor = Rc::new(Cell::new((px(0.), share, available)));
                let mut divider = div()
                    .id(DIVIDER_ID)
                    .group(DIVIDER_ID)
                    .w(COLUMN_GAP)
                    .h(HEADER_HEIGHT)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_col_resize()
                    .child(
                        div()
                            .w(px(3.))
                            .h(px(16.))
                            .rounded(px(1.5))
                            .group_hover(DIVIDER_ID, |style| style.bg(colors.border)),
                    )
                    .on_drag(anchor.clone(), move |_, _, window, cx| {
                        let (_, share, width) = anchor.get();
                        anchor.set((window.mouse_position().x, share, width));
                        cx.new(|_| ResizeDragPreview)
                    })
                    .on_drag_move(move |event: &DragMoveEvent<DividerDragAnchor>, _, cx| {
                        let (start_x, start_share, width) = event.drag(cx).get();
                        let share = resized_album_share(
                            start_share,
                            f32::from(event.event.position.x - start_x),
                            f32::from(width),
                        );
                        view.update(cx, |this, cx| {
                            if this.album_share != share {
                                this.album_share = share;
                                cx.notify();
                            }
                        });
                    })
                    .into_any_element();
                divider.layout_as_root(
                    size(
                        AvailableSpace::Definite(COLUMN_GAP),
                        AvailableSpace::Definite(HEADER_HEIGHT),
                    ),
                    window,
                    cx,
                );
                divider.prepaint_at(point(title.right(), bounds.top()), window, cx);
                divider
            },
            |_, mut divider, window, cx| divider.paint(window, cx),
        )
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(HEADER_HEIGHT)
        .into_any_element()
    }

    /// 离开事件只清除自身，避免跨行事件顺序导致 hover 闪烁。
    fn set_hovered_row(&mut self, index: u64, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(index)
        } else if self.hovered_row == Some(index) {
            None
        } else {
            self.hovered_row
        };
        if next != self.hovered_row {
            self.hovered_row = next;
            cx.notify();
        }
    }

    fn song_cells(&mut self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = Theme::global(cx).tokens.colors;
        let library = self.library.read(cx);
        let detail = self.detail.read(cx);
        let song = &detail.songs[self.display_order[index]];
        let song_id = song.id;
        let liked = library.liked_song_ids.contains(&song_id);
        let album_name = song
            .al
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty());
        let hovered = self.hovered_row == Some(song_id);
        // 序号格的固定部分：两种内容（序号 / 播放键）共用同一套宽度和对齐方式，
        // 靠右统一走 flex 的 `justify_end`，不给谁单独算一套。
        let index_cell = div()
            .w_full()
            .flex()
            .justify_end()
            .whitespace_nowrap()
            .font_family(DOLPHIN_FAMILY);
        vec![
            if hovered {
                // hover 时序号让位给播放键
                // `play.svg` 的三角在画布里右边自带空白，光靠 `justify_end` 贴不到边
                index_cell
                    .child(
                        div()
                            .id(("playlist-select-song", song_id))
                            .mr(px(-3.))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_song(song_id, cx)),
                            )
                            .child(row_hover_icon(
                                ("playlist-song-play", song_id),
                                "icons/play.svg",
                                "播放",
                                px(20.),
                                colors,
                            )),
                    )
                    .into_any_element()
            } else {
                index_cell
                    .text_color(colors.muted_foreground)
                    .child(format!("{:02}", index + 1))
                    .into_any_element()
            },
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
                        // picUrl 可以直接使用 API 返回的远程地址。
                        .when_some(song.al.pic_url.clone(), |cover, url| {
                            // GPUI 的 overflow_hidden 按矩形裁剪，圆角需要直接设置在图片上。
                            cover.child(
                                img(thumbnail_url(&url, 72))
                                    .size_full()
                                    .rounded(px(4.))
                                    .object_fit(ObjectFit::Cover),
                            )
                        }),
                )
                .child(
                    // 标题这一格横向分成两块：左边是两行文字，右边是 hover 时才出现的操作图标。
                    // 文字那块保持 `flex_1 + min_w(0)`，所以宽度不够时先截断文字、列宽不变。
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                // 标题行：标题和副标题是同一行文字（见 title_with_subtitle），
                                // 所以窄了只会出现一个省略号。
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .truncate()
                                        .child(title_with_subtitle(song, colors)),
                                )
                                .child(
                                    // 歌手/制作人：音质徽章 + 名字。名字不写字号，继承行内的 ROW_TEXT_SIZE。
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .min_w(px(0.))
                                        // 徽章用 img() 才保得住原始配色；宽度由 img() 按图片比例自动定，
                                        // flex_none 是为了让它别被压缩（空间不够时该截断的是歌手名）。
                                        .when_some(quality_badge(song), |row, badge| {
                                            row.child(img(badge).h(px(13.)).flex_none())
                                        })
                                        .child(
                                            div()
                                                .min_w(px(0.))
                                                .truncate()
                                                .font_weight(SECONDARY_FONT_WEIGHT)
                                                .text_color(colors.muted_foreground)
                                                .child(artist_label(song, colors)),
                                        ),
                                ),
                        )
                        // 只在悬浮这一行时才把图标挂上。这和"渲染出来但隐形"是两回事：
                        // 隐形元素照样占宽度、也照样能被点到，会凭空吃掉标题宽度和一片点击热区。
                        .when(hovered, |title| title.child(row_actions(song_id, colors))),
                )
                .into_any_element(),
            div()
                .truncate()
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(if album_name.is_some() {
                    colors.secondary_foreground
                } else {
                    colors.foreground.alpha(0.45)
                })
                .child(album_name.unwrap_or("未知专辑").to_owned())
                .into_any_element(),
            // 裸 SVG，没有圆形底色：已喜欢是实心红心，未喜欢是勾线灰心。
            svg()
                .path(if liked {
                    "icons/heart.svg"
                } else {
                    "icons/heart_outline.svg"
                })
                // 和 header 右上角那排图标同一个尺寸。
                .size(IconSize::Small.pixels())
                .text_color(if liked {
                    colors.primary
                } else {
                    // 和表头那排图标同一档灰。
                    colors.foreground.alpha(0.6)
                })
                // 和其它图标一样，hover 只改颜色：没点亮时变深，已点亮时按主题红的惯例淡一档。
                .hover(move |style| {
                    if liked {
                        style.text_color(colors.primary.alpha(0.88))
                    } else {
                        style.text_color(colors.foreground)
                    }
                })
                .id(("playlist-song-like", song_id))
                .aria_label(if liked { "已喜欢" } else { "未喜欢" })
                .into_any_element(),
            div()
                // 时长是辅助信息：比 `muted_foreground`(60%) 再淡一档（45%），字重也细一档。
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(colors.foreground.alpha(0.45))
                .child(format!("{:02}:{:02}", song.dt / 60000, song.dt / 1000 % 60))
                .into_any_element(),
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SongSort {
    Title,
    Artist,
    Album,
}

// 标题循环：默认 → 标题升序 → 标题降序 → 歌手升序 → 歌手降序。
fn next_sort(current: Option<(SongSort, bool)>, column: SongSort) -> Option<(SongSort, bool)> {
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

fn sort_label(current: Option<(SongSort, bool)>, column: SongSort) -> (&'static str, &'static str) {
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

fn song_order(songs: &[Song], column: SongSort, descending: bool) -> Vec<usize> {
    let mut order: Vec<_> = (0..songs.len()).collect();
    // ponytail: 按 Unicode 字符排序；需要拼音或语言区域排序时再接入 collation。
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

fn resized_album_share(start: f32, movement: f32, available: f32) -> f32 {
    if available <= 0. {
        return start;
    }
    (start - movement / available).clamp(ALBUM_SHARE_MIN, ALBUM_SHARE_MAX)
}

/// 「播放全部」：主题红实心按钮，白字白图标，无边框。
fn play_all_button(colors: ColorTokens) -> Button {
    Button::new("playlist-play-all-button")
        .h(ACTION_BUTTON_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        // 宽度交给内容：图标 + 文字 + 左右内边距
        .px(ACTION_BUTTON_PADDING)
        .rounded_lg()
        .bg(colors.primary)
        // hover 只改颜色（向背景靠一档），按下统一降整体透明度
        .hover(|style| style.bg(colors.primary.alpha(0.88)))
        .active(|style| style.opacity(PRESSED_OPACITY))
        .text_size(px(13.))
        .text_color(colors.primary_foreground)
        .child(
            svg()
                .path("icons/play.svg")
                .size(px(18.))
                .flex_none()
                .text_color(colors.primary_foreground),
        )
        .child("播放全部")
}

impl Render for PlaylistPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let active_tab = [
            PlaylistTab::Songs,
            PlaylistTab::Comments,
            PlaylistTab::Collectors,
        ][self.tabs.read(cx).selected_index()];
        let library = self.library.read(cx);
        let detail = self.detail.read(cx);
        let playlist = detail.playlist.as_ref();
        let cover_url = playlist.and_then(|playlist| playlist.cover_img_url.clone());
        let play_count = playlist.map(|playlist| playlist.play_count);
        let created_date = playlist.and_then(|playlist| {
            time::OffsetDateTime::from_unix_timestamp(playlist.create_time / 1000)
                .ok()
                .map(|date| {
                    format!(
                        "{}-{:02}-{:02}创建",
                        date.year(),
                        u8::from(date.month()),
                        date.day()
                    )
                })
        });
        let error = detail
            .error
            .as_ref()
            .or_else(|| {
                if detail.id.is_none() {
                    library.error.as_ref()
                } else {
                    None
                }
            })
            .cloned();
        let content = match active_tab {
            _ if detail.loading || (detail.id.is_none() && library.loading) => div()
                .id("playlist-loading")
                .aria_label("加载中")
                .w_full()
                .py(px(36.))
                .flex()
                .justify_center()
                .child(
                    svg()
                        .path("icons/loading.svg")
                        .size(px(20.))
                        .text_color(colors.muted_foreground)
                        .with_animation(
                            "playlist-loading-animation",
                            Animation::new(std::time::Duration::from_millis(900)).repeat(),
                            |icon, progress| {
                                icon.with_transformation(Transformation::rotate(percentage(
                                    progress,
                                )))
                            },
                        ),
                )
                .into_any_element(),
            _ if error.is_some() => div()
                .text_color(colors.muted_foreground)
                .child(error.unwrap())
                .when(detail.id.is_some() || library.user_id != 0, |message| {
                    message.child(Button::new("playlist-retry").ml_2().child("重试").on_click(
                        cx.listener(|this, _, _, cx| {
                            if this.detail.read(cx).id.is_some() {
                                this.open(this.playlist_id, cx);
                            } else {
                                this.library.update(cx, |library, cx| library.retry(cx));
                            }
                        }),
                    ))
                })
                .into_any_element(),
            _ if detail.id.is_none() => div()
                .text_color(colors.muted_foreground)
                .child("暂无歌单")
                .into_any_element(),
            PlaylistTab::Songs if self.display_order.is_empty() => div()
                .py_8()
                .text_color(colors.muted_foreground)
                .child("这个歌单还没有歌曲")
                .into_any_element(),
            PlaylistTab::Songs => {
                let header_layout = Rc::new(Cell::new([Bounds::default(); 2]));
                let table = virtual_table(
                    cx.entity(),
                    "playlist-songs",
                    self.columns(),
                    self.display_order.len(),
                    |this, index, _, cx| this.song_cells(index, cx),
                    |this, index, cx| this.detail.read(cx).songs[this.display_order[index]].id,
                    {
                        let view = cx.entity();
                        let sort = self.sort;
                        move |column, index| {
                            let key = match index {
                                1 => Some(SongSort::Title),
                                2 => Some(SongSort::Album),
                                _ => None,
                            };
                            if let Some(key) = key {
                                let view = view.clone();
                                let group = if index == 1 {
                                    "playlist-title-header"
                                } else {
                                    "playlist-album-header"
                                };
                                let (icon, label) = sort_label(sort, key);
                                Button::new(("playlist-sort", index))
                                    .group(group)
                                    .w_full()
                                    .h(px(30.))
                                    .px(CELL_PADDING)
                                    .rounded(px(8.))
                                    .hover(|style| style.bg(colors.muted))
                                    .justify_start()
                                    .gap(px(12.))
                                    .text_color(colors.muted_foreground)
                                    .child(column.title.clone())
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .when(label == "默认排序", |label| {
                                                label
                                                    .opacity(0.)
                                                    .group_hover(group, |style| style.opacity(1.))
                                            })
                                            .child(
                                                svg()
                                                    .path(icon)
                                                    .size(px(16.))
                                                    .flex_none()
                                                    .text_color(colors.muted_foreground),
                                            )
                                            .child(label),
                                    )
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |this, cx| this.toggle_sort(key, cx))
                                    })
                                    .into_any_element()
                            } else {
                                div()
                                    .px(CELL_PADDING)
                                    .child(column.title.clone())
                                    .into_any_element()
                            }
                        }
                    },
                    {
                        let layout = header_layout.clone();
                        move |columns| layout.set([columns[1], columns[2]])
                    },
                    |this, index, hovered, cx| this.set_hovered_row(index, hovered, cx),
                    cx,
                )
                .into_any_element();
                // 分界线要绝对定位在列边界上，所以表格外面要有一层 position: relative。
                div()
                    .relative()
                    .child(table)
                    .child(self.title_divider(colors, header_layout, cx))
                    .into_any_element()
            }
            _ => div()
                .text_size(px(14.))
                .text_color(colors.muted_foreground)
                .child(active_tab.content_title())
                .into_any_element(),
        };

        let cover = div()
            .size(px(170.))
            .flex_none()
            .relative()
            .when_some(cover_url, |cover, url| {
                cover
                    .child(
                        img(thumbnail_url(&url, 340))
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .rounded(px(8.)),
                    )
                    // 在封面内部从顶部向下渐淡，播放量放在遮罩上方。
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(8.))
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(black().alpha(0.2), 0.),
                                linear_color_stop(black().alpha(0.), 0.35),
                            )),
                    )
            })
            .when_some(play_count, |cover, count| {
                cover.child(
                    div()
                        .absolute()
                        .top_1()
                        .right_2()
                        .flex()
                        .items_center()
                        .text_color(white())
                        .text_size(px(15.))
                        .font_family(DOLPHIN_FAMILY)
                        .child(
                            svg()
                                .path("icons/headphone.svg")
                                .size(px(15.))
                                .text_color(white()),
                        )
                        .child(count.to_string()),
                )
            });

        let title = div()
            .text_size(px(24.))
            .line_height(rems(3.))
            .line_clamp(2)
            .font_weight(FontWeight::BOLD)
            .text_color(colors.foreground)
            .child(
                playlist
                    .map(|playlist| playlist.name.clone())
                    .unwrap_or_else(|| "歌单".into()),
            );

        let author = div()
            .flex()
            .min_w(px(0.))
            .items_center()
            .gap_2()
            .when_some(playlist, |author, playlist| {
                author
                    .child(
                        Avatar::new()
                            .with_size(px(26.))
                            .flex_none()
                            .src(thumbnail_url(&playlist.creator.avatar_url, 52)),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors.secondary_foreground)
                            .child(playlist.creator.nickname.clone()),
                    )
            })
            .when_some(created_date, |row, date| {
                row.child(
                    div()
                        .ml_2()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(colors.muted_foreground)
                        .font_weight(FontWeight::LIGHT)
                        .child(date),
                )
            });

        div()
            // 封面标题区域
            .child(
                div().flex().gap_6().child(cover).child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .justify_between()
                        .child(div().child(title).child(author))
                        // 操作按钮组：播放全部 / 下载 / 更多
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .child(play_all_button(colors).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        let id = this
                                            .detail
                                            .read(cx)
                                            .songs
                                            .get(*this.display_order.first().unwrap_or(&0))
                                            .map(|song| song.id);
                                        if let Some(id) = id {
                                            this.select_song(id, cx);
                                        }
                                    },
                                )))
                                // 下载：宽度自适应，比 muted 更浅的底 + 比 muted 更浅的描边，
                                // 文字/图标用比 muted_foreground 深一档的灰保证可读
                                .child(
                                    Button::new("playlist-download-button")
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
                                        // 底色本身已有 3%，hover 只抬到 8%（+5 个点），
                                        // 观感与 header 返回键的 0 → accent(6%) 接近；
                                        // 按下不再换更深的颜色，统一降整体透明度
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
                                // 更多：固定 36×36 的纯图标按钮，靠 flex 居中
                                .child(
                                    Button::new("playlist-more-button")
                                        .size(px(36.))
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
                                        // 弹出系统原生菜单
                                        .on_click(|event, window, cx| {
                                            NativeMenu::new()
                                                .menu("分享…", Box::new(Share))
                                                .menu("批量操作", Box::new(BatchOperation))
                                                .menu(
                                                    "添加全部至播放列表",
                                                    Box::new(AddAllToPlaylist),
                                                )
                                                .show(event.position(), window, cx);
                                        })
                                        .child(
                                            svg()
                                                .path("icons/xpoint.svg")
                                                .size(px(16.))
                                                .flex_none()
                                                .text_color(colors.secondary_foreground),
                                        ),
                                ),
                        ),
                ),
            )
            // 控件区域
            .child(div().mt(px(28.)).child(self.tabs.clone()))
            // 具体内容区域
            .when_some(library.likes_error.as_ref(), |page, message| {
                page.child(
                    div()
                        .text_color(colors.muted_foreground)
                        .text_size(px(12.))
                        .child(format!("喜欢状态暂不可用：{message}")),
                )
            })
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ALBUM_SHARE_MAX, ALBUM_SHARE_MIN, Song, SongSort, next_sort, resized_album_share,
        song_order,
    };

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
    fn resize_preserves_ratio_and_bounds() {
        let share = resized_album_share(0.35, 50., 500.);
        assert!((share - 0.25).abs() < 0.00001);
        assert_eq!(resized_album_share(share, 0., 1000.), share);
        assert_eq!(resized_album_share(share, 10000., 500.), ALBUM_SHARE_MIN);
        assert_eq!(resized_album_share(share, -10000., 500.), ALBUM_SHARE_MAX);
        assert_eq!(resized_album_share(share, 50., 0.), share);
    }
}
