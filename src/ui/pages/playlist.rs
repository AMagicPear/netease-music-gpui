use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme, Transition, transition};
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::native_menu::NativeMenu;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

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

use crate::models::{Playlist, Song};
use crate::playback::PlaybackController;
use crate::state::{library::MusicLibrary, playlist_detail::PlaylistDetail};
use crate::ui::assets::thumbnail_url;
use crate::ui::components::{
    CELL_PADDING, COLUMN_GAP, HEADER_HEIGHT, ROW_TEXT_SIZE, ResizeDragPreview, TabBar, TabChanged,
    TabItem, TableColumn, artist_label, format_duration, like_icon_path, quality_badge_path,
    spinner, virtual_table,
};
use crate::ui::cover_color::Backdrop;
use crate::ui::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_ICON_ALPHA, PRESSED_OPACITY};

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
    playback: Entity<PlaybackController>,
    _playback_subscription: Subscription,
    _library_subscription: Subscription,
    /// 原始歌单顺序保存在 detail.songs；这里只保存显示顺序。
    display_order: Vec<usize>,
    /// 按原始歌曲索引缓存；排序只改变 display_order，不复制展示数据。
    song_display: Vec<SongDisplay>,
    columns: Rc<Vec<TableColumn>>,
    sort: Option<(SongSort, bool)>,
    album_share: f32,
    hovered_row: Option<u64>,
    table_header_hidden: Rc<Cell<bool>>,
    measured_size: Rc<Cell<Option<Size<Pixels>>>>,
    playing_indicator: Entity<PlayingIndicator>,
}

struct SongDisplay {
    cover: Option<ImageSource>,
    title: SharedString,
    subtitle: std::ops::Range<usize>,
    album: SharedString,
    duration: SharedString,
}

impl SongDisplay {
    fn new(song: &Song) -> Self {
        let mut title = song.name.clone();
        let start = title.len();
        if let Some(subtitle) = song.tns.first().or_else(|| song.alia.first()) {
            title.push_str(&format!("（{subtitle}）"));
        }
        let end = title.len();
        Self {
            cover: Some(crate::ui::assets::track_cover_url(song.al.pic_url.as_deref(), 72).into()),
            title: title.into(),
            subtitle: start..end,
            album: song
                .al
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("未知专辑")
                .to_owned()
                .into(),
            duration: format_duration(song.duration()).into(),
        }
    }
}

/// 使用同一段 StyledText，标题和副标题一起截断；颜色随当前主题计算。
fn title_with_subtitle(display: &SongDisplay, colors: ColorTokens) -> StyledText {
    if display.subtitle.is_empty() {
        return StyledText::new(display.title.clone());
    }

    // highlight 先 blend 再 fade_out：先替换 RGB，再恢复 alpha。
    // 因此播放中的红色标题不会污染副标题，也无需预合成背景色。
    let subtitle_color = colors.muted_foreground;

    StyledText::new(display.title.clone()).with_highlights([(
        display.subtitle.clone(),
        HighlightStyle {
            color: Some(subtitle_color.alpha(1.)),
            font_weight: Some(SECONDARY_FONT_WEIGHT),
            fade_out: Some(1. - subtitle_color.a),
            ..Default::default()
        },
    )])
}

/// 播放量：十万以下照写，十万起折算成万，万位满四位不再带小数。
fn play_count_label(count: u64) -> String {
    const WAN: u64 = 10_000;
    if count < 10 * WAN {
        return count.to_string();
    }
    let wan = count / WAN;
    // 十分位截断而不是四舍五入：812568 是 81.2万，不是 81.3万。
    let tenth = count % WAN / 1_000;
    if wan >= 1_000 || tenth == 0 {
        format!("{wan}万")
    } else {
        format!("{wan}.{tenth}万")
    }
}

/// 简介压成一行：换行、连续空白、首尾空白都折叠成一个空格。
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 标签胶囊：跟在作者后面横向排开，一行放不下时整体被裁掉，不挤走创建时间。
fn tag_row(tags: &[String], colors: ColorTokens) -> AnyElement {
    div()
        .ml(px(12.))
        .flex()
        .items_center()
        .gap(px(4.))
        .min_w(px(0.))
        .overflow_hidden()
        .whitespace_nowrap()
        .children(tags.iter().map(|tag| {
            div()
                .flex_none()
                .h(px(18.))
                .px(px(6.))
                .flex()
                .items_center()
                .rounded(px(9.))
                .bg(colors.muted)
                .text_size(px(11.))
                .text_color(colors.secondary_foreground)
                .child(tag.clone())
        }))
        .into_any_element()
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
    // `IconSize::Small`，灰度和三态交互在这里统一给。
    // 返回 `Stateful<Svg>`，需要交互的图标（收藏 / 更多）还能继续挂 `.on_click`。
    let action_icon = |id: &'static str, path: &'static str, label: &'static str| {
        svg()
            .path(path)
            .size(IconSize::Small.pixels())
            .flex_none()
            .text_color(colors.foreground.alpha(0.6))
            .hover(|style| style.text_color(colors.foreground))
            .id((id, index))
            .role(Role::Button)
            .aria_label(label)
            .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        // 间距抄 header 那排图标（图标之间 10px），左侧再加 10px，免得贴着标题文字。
        .gap(px(10.))
        .ml(px(10.))
        .child(action_icon(
            "playlist-song-download",
            "icons/download_outline.svg",
            "下载",
        ))
        // 从左往右第二个图标就是收藏入口。
        .child(
            action_icon("playlist-song-collect", "icons/collect.svg", "收藏")
                .on_click(|_, window, cx| crate::ui::components::open_collect_window(window, cx)),
        )
        .child(action_icon(
            "playlist-song-comment",
            "icons/comment.svg",
            "评论",
        ))
        // 最右侧的「更多」弹出原生菜单。
        .child(
            action_icon("playlist-song-more", "icons/xpoint.svg", "更多").on_click(
                |event, window, cx| {
                    crate::ui::components::show_song_menu(event.position(), window, cx)
                },
            ),
        )
        .into_any_element()
}

#[derive(Clone, Copy)]
struct IndicatorPlacement {
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
}

/// 与缓存的歌单 View 同级挂载，动画通知不经过歌单的视图路径。
struct PlayingIndicator {
    placement: Rc<Cell<Option<IndicatorPlacement>>>,
    started_at: Instant,
}

fn indicator_bars(bounds: Bounds<Pixels>, progress: f32) -> [Bounds<Pixels>; 3] {
    [0, 1, 2].map(|index| {
        let phase = index as f32 * 0.33;
        let height = px(2. + ((progress + phase) * std::f32::consts::TAU).sin().abs() * 13.);
        Bounds::new(
            point(
                bounds.left() + px(index as f32 * 5.),
                bounds.bottom() - height,
            ),
            size(px(2.), height),
        )
    })
}

impl Render for PlayingIndicator {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let placement = self.placement.clone();
        let started_at = self.started_at;
        let color = Theme::global(cx).tokens.colors.primary;
        canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                // 此时歌单已完成 prepaint，位置来自本帧或仍有效的缓存。
                let Some(placement) = placement.get() else {
                    return;
                };
                if !placement.bounds.intersects(&placement.mask.bounds) {
                    return;
                }
                let progress = if cx.reduce_motion() {
                    0.
                } else {
                    started_at.elapsed().as_secs_f32() / 2.
                };
                window.with_content_mask(Some(placement.mask), |window| {
                    for bounds in indicator_bars(placement.bounds, progress) {
                        window.paint_quad(fill(bounds, color));
                    }
                });
                if !cx.reduce_motion() {
                    window.request_animation_frame();
                }
            },
        )
        .absolute()
        .inset_0()
    }
}

impl PlaylistPage {
    pub fn new(
        library: Entity<MusicLibrary>,
        playback: Entity<PlaybackController>,
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
        let tabs_subscription = cx.subscribe(&tabs, |this, _, _: &TabChanged, cx| {
            this.measured_size.set(None);
            cx.notify();
        });
        let library_subscription = cx.observe(&library, |this, library, cx| {
            this.measured_size.set(None);
            // 喜欢的音乐歌单的内容由 liked_song_ids 决定：任何入口取消 / 点亮红心后，
            // 这里要把已加载的行也同步掉，否则列表不会“刷新”。
            // 但只有喜欢列表成功加载后这个集合才是可信的：加载中或请求失败时
            // 它可能是空的，拿它去过滤会把整个歌单误删。
            let (liked, cache, ready) = {
                let library = library.read(cx);
                (
                    library.liked_song_ids.clone(),
                    library.liked_song_cache.clone(),
                    !library.loading && library.likes_error.is_none(),
                )
            };
            if ready {
                this.detail.update(cx, |detail, cx| {
                    if detail.reconcile_liked(&liked, &cache) {
                        cx.notify();
                    }
                });
            }
            cx.notify();
        });
        let mut playback_state = playlist_playback_state(playback.read(cx));
        let playback_subscription = cx.observe(&playback, move |_, playback, cx| {
            let next = playlist_playback_state(playback.read(cx));
            if next != playback_state {
                playback_state = next;
                cx.notify();
            }
        });
        let detail_subscription = cx.observe(&detail, |this, detail, cx| {
            this.measured_size.set(None);
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
            this.song_display = detail.songs.iter().map(SongDisplay::new).collect();
            this.columns = table_columns(this.display_order.len(), this.album_share);
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
        let playing_indicator = cx.new(|_| PlayingIndicator {
            placement: Rc::new(Cell::new(None)),
            started_at: Instant::now(),
        });
        Self {
            detail,
            _detail_subscription: detail_subscription,
            playlist_id: None,
            tabs,
            _tabs_subscription: tabs_subscription,
            library,
            playback,
            _playback_subscription: playback_subscription,
            _library_subscription: library_subscription,
            display_order: Vec::new(),
            song_display: Vec::new(),
            columns: table_columns(0, 0.35),
            sort: None,
            album_share: 0.35,
            hovered_row: None,
            table_header_hidden: Rc::new(Cell::new(false)),
            measured_size: Rc::new(Cell::new(None)),
            playing_indicator,
        }
    }

    /// 缓存需要确定尺寸；首次布局或宽度改变时先正常测量，之后复用。
    pub fn content(view: Entity<Self>, width: Pixels, cx: &App) -> AnyElement {
        let measured = view.read(cx).measured_size.get();
        match measured {
            Some(size) if size.width == width => view
                .cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(size.height)
                        .flex_shrink(0.),
                )
                .into_any_element(),
            _ => view.into_any_element(),
        }
    }

    pub fn playing_overlay(&self) -> AnyElement {
        self.playing_indicator.clone().into_any_element()
    }

    pub fn cover_url(&self, cx: &App) -> Option<String> {
        self.detail
            .read(cx)
            .playlist
            .as_ref()?
            .cover_img_url
            .as_ref()
            .filter(|url| !url.is_empty())
            .map(|url| thumbnail_url(url, 340))
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

    fn action_buttons(&self, compact: bool, cx: &Context<Self>) -> Div {
        let colors = Theme::global(cx).tokens.colors;
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                play_all_button(colors, compact).on_click(cx.listener(|this, _, _, cx| {
                    let detail = this.detail.read(cx);
                    let Some(playlist_id) = detail.playlist.as_ref().map(|playlist| playlist.id)
                    else {
                        return;
                    };
                    let songs = this
                        .display_order
                        .iter()
                        .map(|&index| detail.songs[index].clone())
                        .collect();
                    this.playback.update(cx, |playback, cx| {
                        playback.play_list_from_start(playlist_id, songs, cx);
                    });
                })),
            )
            // 下载：宽度自适应，比 muted 更浅的底 + 比 muted 更浅的描边，
            // 文字/图标用比 muted_foreground 深一档的灰保证可读
            .child(if compact {
                icon_action_button(
                    "playlist-floating-download",
                    "icons/download.svg",
                    "下载",
                    colors,
                )
            } else {
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
                    .child("下载")
            })
            // 更多：固定 36×36 的纯图标按钮，靠 flex 居中
            .child(
                icon_action_button(
                    if compact {
                        "playlist-floating-more"
                    } else {
                        "playlist-more-button"
                    },
                    "icons/xpoint.svg",
                    "更多",
                    colors,
                )
                // 弹出系统原生菜单
                .on_click(|event, window, cx| {
                    NativeMenu::new()
                        .menu("分享…", Box::new(Share))
                        .menu("批量操作", Box::new(BatchOperation))
                        .menu("添加全部至播放列表", Box::new(AddAllToPlaylist))
                        .show(event.position(), window, cx);
                }),
            )
    }

    /// 在滚动容器外预绘制，透明度和位移共用进度；渐隐结束后才移除。
    pub fn floating_header(&self, backdrop: Backdrop, cx: &Context<Self>) -> AnyElement {
        let hidden = self.table_header_hidden.clone();
        let view = cx.entity();
        canvas(
            move |bounds, window, cx| {
                let progress = transition(
                    "playlist-floating-header-presence",
                    if hidden.get() { 1. } else { 0. },
                    Transition::new(Duration::from_millis(600)).ease(ease_out_quint()),
                    window,
                    cx,
                );
                if progress <= 0. {
                    return None;
                }
                let mut header = view.update(cx, |this, cx| {
                    let colors = Theme::global(cx).tokens.colors;
                    div()
                        .id("playlist-floating-header")
                        .w_full()
                        .h(px(110.))
                        .px(px(40.))
                        .py(px(14.))
                        .flex()
                        .flex_col()
                        .justify_between()
                        .bg(colors.background)
                        .relative()
                        .opacity(progress)
                        .occlude()
                        .child(
                            canvas(move |bounds, _, _| bounds, {
                                let backdrop = backdrop.clone();
                                move |bounds, _, window, _| backdrop.paint(bounds, window)
                            })
                            .absolute()
                            .inset_0(),
                        )
                        .child(this.playlist_title(cx).line_clamp(1))
                        .child(this.action_buttons(true, cx))
                        // 与侧栏分隔条同色，左右对齐页面内容的 40px 留白。
                        .child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left(px(40.))
                                .right(px(40.))
                                .h(px(1.))
                                .bg(colors.border),
                        )
                        .into_any_element()
                });
                header.layout_as_root(
                    size(
                        AvailableSpace::Definite(bounds.size.width),
                        AvailableSpace::Definite(px(110.)),
                    ),
                    window,
                    cx,
                );
                // 只移动预绘制坐标，布局尺寸和列表位置不变，点击区域随栏移动。
                header.prepaint_at(
                    bounds.origin + point(px(0.), px(12. * (1. - progress))),
                    window,
                    cx,
                );
                Some(header)
            },
            |_, header, window, cx| {
                if let Some(mut header) = header {
                    header.paint(window, cx);
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    fn playlist_title(&self, cx: &App) -> Div {
        div()
            .text_size(px(24.))
            .line_clamp(2)
            .font_weight(FontWeight::BOLD)
            .text_color(Theme::global(cx).tokens.colors.foreground)
            .child(
                self.detail
                    .read(cx)
                    .playlist
                    .as_ref()
                    .map(|playlist| playlist.name.clone())
                    .unwrap_or_else(|| "歌单".into()),
            )
    }

    fn playlist_metadata(&self, colors: ColorTokens, cx: &Context<Self>) -> Div {
        let detail = self.detail.read(cx);
        let playlist = detail.playlist.as_ref();
        let cover_url = self.cover_url(cx);
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
        let description = playlist.and_then(Playlist::description).map(one_line);
        let tags = playlist.map(Playlist::tags).unwrap_or_default();
        let cover = div()
            .size(px(170.))
            .flex_none()
            .relative()
            .when_some(cover_url, |cover, url| {
                cover
                    .child(
                        img(url)
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
                        .child(play_count_label(count)),
                )
            });
        let author = div()
            .flex()
            .min_w(px(0.))
            .items_center()
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
                            .ml(px(6.))
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors.secondary_foreground)
                            .child(playlist.creator.nickname.clone()),
                    )
            })
            .when(!tags.is_empty(), |author| {
                author.child(tag_row(tags, colors))
            })
            .when_some(created_date, |row, date| {
                row.child(
                    div()
                        .ml(px(12.))
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(colors.muted_foreground)
                        .font_weight(FontWeight::LIGHT)
                        .child(date),
                )
            });
        div().flex().gap_6().child(cover).child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.))
                        // 三行共用一个间距；其余留白交给 justify_between。
                        .gap(px(6.))
                        .child(self.playlist_title(cx))
                        // 简介单行截断，没有就整行不出现。
                        .when_some(description, |column, text| {
                            column.child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(SECONDARY_FONT_WEIGHT)
                                    .text_color(colors.muted_foreground)
                                    .child(text),
                            )
                        })
                        .child(author),
                )
                .child(self.action_buttons(false, cx)),
        )
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
        let detail = self.detail.read(cx);
        let playlist_id = detail.playlist.as_ref().map_or(0, |playlist| playlist.id);
        let songs = self
            .display_order
            .iter()
            .map(|&index| detail.songs[index].clone())
            .collect();
        self.playback.update(cx, |playback, cx| {
            playback.play_list(playlist_id, songs, song_id, cx);
        });
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
                                this.columns = table_columns(this.display_order.len(), share);
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
                            img(crate::ui::assets::track_cover_image(source))
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
                            // 标题和副标题共用 StyledText，截断时只有一个省略号。
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
                                    // img 保留徽章原色并按比例定宽；flex_none 让空间不足时截断歌手名。
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
                    )
                    // 隐形元素仍占宽度且可点击，因此只在 hover 时挂载操作图标。
                    .when(self.hovered_row == Some(song.id), |title| {
                        title.child(row_actions(song.id, colors))
                    }),
            )
            .into_any_element()
    }

    fn song_cells(&mut self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = Theme::global(cx).tokens.colors;
        let library = self.library.read(cx);
        let detail = self.detail.read(cx);
        let song = &detail.songs[self.display_order[index]];
        let display = &self.song_display[self.display_order[index]];
        let song_id = song.id;
        let like_song = song.clone();
        let playback = self.playback.read(cx);
        let is_current_song = playback
            .snapshot()
            .current_song
            .as_ref()
            .is_some_and(|current| current.id == song_id);
        let is_playing = playback.snapshot().is_playing;
        let play_requested = playback.is_play_requested();
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
            if is_current_song && play_requested && hovered {
                index_cell
                    .child(
                        div()
                            .id(("playlist-pause-song", song_id))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.playback.update(cx, |playback, cx| {
                                    playback.pause(cx);
                                });
                            }))
                            .child(row_hover_icon(
                                ("playlist-song-pause", song_id),
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
                            .id(("playlist-select-song", song_id))
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
            } else if is_current_song && is_playing {
                let placement = self.playing_indicator.read(cx).placement.clone();
                index_cell
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                placement.set(Some(IndicatorPlacement {
                                    bounds,
                                    mask: window.content_mask(),
                                }));
                            },
                            |_, _, _, _| {},
                        )
                        .w(px(12.))
                        .h(px(15.)),
                    )
                    .into_any_element()
            } else {
                index_cell
                    .text_color(colors.muted_foreground)
                    .child(format!("{:02}", index + 1))
                    .into_any_element()
            },
            self.song_title_cell(song, display, is_current_song, colors),
            div()
                .truncate()
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(if album_name.is_some() {
                    colors.secondary_foreground
                } else {
                    colors.foreground.alpha(0.45)
                })
                .child(display.album.clone())
                .into_any_element(),
            // 裸 SVG，没有圆形底色：已喜欢是实心红心，未喜欢是勾线灰心。
            svg()
                .path(like_icon_path(liked))
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
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.library
                        .update(cx, |library, cx| library.toggle_like(like_song.clone(), cx));
                }))
                .into_any_element(),
            div()
                // 时长是辅助信息：比 `muted_foreground`(60%) 再淡一档（45%），字重也细一档。
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(colors.foreground.alpha(0.45))
                .child(display.duration.clone())
                .into_any_element(),
        ]
    }
}

fn table_columns(row_count: usize, album_share: f32) -> Rc<Vec<TableColumn>> {
    Rc::new(vec![
        TableColumn::new("#", Some(index_column_width(row_count))).align_right(),
        TableColumn::new("标题", None).weight(1. - album_share),
        TableColumn::new("专辑", None).weight(album_share),
        TableColumn::new("喜欢", Some(LIKE_COLUMN_WIDTH)),
        TableColumn::new("时长", Some(DURATION_COLUMN_WIDTH)),
    ])
}

fn playlist_playback_state(playback: &PlaybackController) -> (Option<u64>, bool, bool) {
    let snapshot = playback.snapshot();
    (
        snapshot.current_song.as_ref().map(|song| song.id),
        snapshot.is_playing,
        playback.is_play_requested(),
    )
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

/// 与「更多」相同的方形图标按钮。
fn icon_action_button(
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

/// 「播放全部」：主题红实心按钮，白字白图标，无边框。
fn play_all_button(colors: ColorTokens, compact: bool) -> Button {
    Button::new(if compact {
        "playlist-floating-play"
    } else {
        "playlist-play-all-button"
    })
    .h(ACTION_BUTTON_HEIGHT)
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .gap(px(4.))
    // 宽度交给内容：图标 + 文字 + 左右内边距
    .px(ACTION_BUTTON_PADDING)
    .when(compact, |button| button.w(ACTION_BUTTON_HEIGHT).px(px(0.)))
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
    .aria_label("播放全部")
    .when(!compact, |button| button.child("播放全部"))
}

impl Render for PlaylistPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.playing_indicator.read(cx).placement.set(None);
        self.table_header_hidden.set(false);
        let colors = Theme::global(cx).tokens.colors;
        let active_tab = [
            PlaylistTab::Songs,
            PlaylistTab::Comments,
            PlaylistTab::Collectors,
        ][self.tabs.read(cx).selected_index()];
        let library = self.library.read(cx);
        let detail = self.detail.read(cx);
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
                .child(spinner(
                    "playlist-loading-animation",
                    20.,
                    colors.muted_foreground,
                ))
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
                    self.columns.clone(),
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
                    // 行上右键和点「更多」图标是同一个效果：都弹这份原生菜单，
                    // 位置也都取鼠标点的窗口坐标（「更多」取的是它自己的点击位置）。
                    |_, _, position, window, cx| {
                        crate::ui::components::show_song_menu(position, window, cx)
                    },
                    cx,
                )
                .into_any_element();
                // 分界线要绝对定位在列边界上，所以表格外面要有一层 position: relative。
                div()
                    .relative()
                    .child(table)
                    .on_children_prepainted({
                        let hidden = self.table_header_hidden.clone();
                        move |bounds, window, _| {
                            hidden.set(
                                bounds[0].top() + HEADER_HEIGHT
                                    <= window.content_mask().bounds.top(),
                            );
                        }
                    })
                    .child(self.title_divider(colors, header_layout, cx))
                    .into_any_element()
            }
            _ => div()
                .text_size(px(14.))
                .text_color(colors.muted_foreground)
                .child(active_tab.content_title())
                .into_any_element(),
        };

        let measured_size = self.measured_size.clone();
        div()
            .relative()
            .child(self.playlist_metadata(colors, cx))
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
            .child(
                canvas(
                    move |bounds, _, _| measured_size.set(Some(bounds.size)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Song, SongSort, next_sort, song_order};

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
}
