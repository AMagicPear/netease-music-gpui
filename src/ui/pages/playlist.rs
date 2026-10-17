use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::input::{Input, InputEditorStyle, InputEvent, InputState};
use gpui_kit::base::{Button, ColorTokens, Theme, Transition, transition};
use gpui_kit::component::Sizable;
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::native_menu::NativeMenu;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// 标题、专辑按剩余宽度的比例分配；拖动后保留该比例。
const ALBUM_SHARE_MIN: f32 = 0.1;
const ALBUM_SHARE_MAX: f32 = 0.8;

/// 分界线的元素 id，同时用作它作为 hover group 的名字。
/// 两处必须完全一致 `group_hover` 才会响应，所以抽成常量而不是写两遍字面量。
const DIVIDER_ID: &str = "playlist-title-divider";

/// 搜索框收起时的宽度。刚好放下「放大镜 + 搜索」：8 留白、16 图标、6 间距、
/// 26 文字、8 留白，另留 8px 余量。占位文字走系统字体，汉字步进宽度等于字号，
/// 即 13px × 2 字 = 26px。
const SEARCH_COLLAPSED_WIDTH: Pixels = px(72.);
/// 搜索框展开后的宽度，和官方「歌单内搜索」一致。
const SEARCH_EXPANDED_WIDTH: Pixels = px(200.);
const SEARCH_BOX_HEIGHT: Pixels = px(30.);
/// 收起 / 展开的时长。宽度是位移量，给足时间胶囊才像「长」出来而不是跳一下。
const SEARCH_EXPAND_DURATION: Duration = Duration::from_millis(220);
/// 边框变色比宽度快一档：先把边界交代清楚，宽度再跟上，读起来是两个动作。
const SEARCH_BORDER_DURATION: Duration = Duration::from_millis(150);

/// 起拖时的鼠标 x、专辑占比、两列可用宽度。
type DividerDragAnchor = Rc<Cell<(Pixels, f32, Pixels)>>;

use crate::models::{Playlist, Song};
use crate::playback::PlaybackController;
use crate::state::{library::MusicLibrary, playlist_detail::PlaylistDetail};
use crate::ui::assets::thumbnail_url;
use crate::ui::components::{
    ACTION_BUTTON_HEIGHT, ACTION_BUTTON_PADDING, CELL_PADDING, COLUMN_GAP, DURATION_COLUMN_WIDTH,
    HEADER_HEIGHT, LIKE_COLUMN_WIDTH, ResizeDragPreview, SECONDARY_FONT_WEIGHT, SongDisplay,
    TabBar, TabChanged, TabItem, TableColumn, artist_label, centered_empty, hovered_row_after,
    index_column_width, keyword_ranges, like_icon_path, play_all_button, play_count_label,
    quality_badge_path, row_hover_icon, search_clear_button, spinner, text_with_matches,
    title_with_subtitle, virtual_table,
};
use crate::ui::cover_color::{Backdrop, blend_color};
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
    /// 导航意图：当前是否是「我喜欢的音乐」页。
    /// 未登录时详情加载不了、拿不到 specialType，标题靠这个字段保持正确。
    is_favorite_view: bool,
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
    /// 歌单内搜索。只过滤已加载的歌曲，不发请求。
    search_input: Entity<InputState>,
    _search_subscription: Subscription,
    /// 当前生效的关键词（trim 过，大小写折叠由 `keyword_ranges` 做）。
    /// 和 `display_order` 同一次 `rebuild_display_order` 写入，两者永远一致；
    /// 存一份是因为每个可见行都要拿它算高亮，现读输入框会逐行分配字符串。
    search_keyword: SharedString,
    columns: Rc<Vec<TableColumn>>,
    sort: Option<(SongSort, bool)>,
    album_share: f32,
    hovered_row: Option<u64>,
    table_header_hidden: Rc<Cell<bool>>,
    measured_size: Rc<Cell<Option<Size<Pixels>>>>,
    playing_indicator: Entity<PlayingIndicator>,
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

/// 行 hover 时贴在标题右侧的那排操作图标：下载 / 收藏 / 评论 / 更多。
fn row_actions(index: u64, song: &Song, colors: ColorTokens) -> AnyElement {
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
            action_icon("playlist-song-more", "icons/xpoint.svg", "更多").on_click({
                let song = song.clone();
                move |event, window, cx| {
                    crate::ui::components::show_song_menu(
                        event.position(),
                        song.clone(),
                        window,
                        cx,
                    )
                }
            }),
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
        window: &mut Window,
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
        // 用带窗口的 `observe_in` 而不是 `observe`：换歌单要清空搜索框，
        // 而 `InputState::set_value` 需要窗口 —— 这是唯一能同时拿到「详情变了」
        // 和「窗口」的地方，不用把 window 一路穿过 `open` / `navigate`。
        let detail_subscription = cx.observe_in(&detail, window, |this, detail, window, cx| {
            this.measured_size.set(None);
            // 「换没换歌单」要先问出来：清空搜索框要 `&mut App`（`Entity::update`），
            // 那个借用和下面 detail 的读借用不能共存，所以这段读锁单独开一个作用域，
            // 出了作用域就还回去了。
            let (changed, id) = {
                let detail = detail.read(cx);
                (this.playlist_id != detail.id, detail.id)
            };
            if changed {
                this.playlist_id = id;
                this.sort = None;
                this.hovered_row = None;
                // 换歌单就把关键词一起清掉：留着上一个歌单的词，新页面会显示一份
                // 「搜过但没搜到」的结果，看起来像这个歌单没有歌。
                //
                // 这里**故意**用 `set_value` 而不是清除按钮那边的 `replace_all`：
                // 导航不是一次可撤销的编辑，Cmd+Z 不该把上一个歌单的关键词拽回来，
                // 而 `set_value` 正好清空编辑历史。代价是它不发 `InputEvent::Change`，
                // 所以下面那次 `rebuild_display_order` 是这里唯一的重算入口，不能省。
                this.search_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            let detail = detail.read(cx);
            this.song_display = detail.songs.iter().map(SongDisplay::new).collect();
            // 列宽跟着显示行数走（序号列按位数定宽），过滤后也要重算。
            this.rebuild_display_order(cx);
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
        // 搜索框的状态交给 `InputState` 自己管：它内部已经建好 `FocusHandle`
        // 并登记为 tab stop，聚焦、选中、输入法都归它，这里不重复造。
        let search_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("搜索");
            state.set_editor_style(InputEditorStyle {
                selection: Theme::global(cx).tokens.colors.selection,
                ..Default::default()
            });
            state
        });
        // 只用到 `Change`：输入即过滤，纯本地计算，不发请求。
        // `Focus` / `Blur` 不需要在这里处理 —— 展开与否在 `render` 里直接读
        // `focus_handle().is_focused(window)`，那才是唯一真值，少存一份状态就不会失同步。
        let search_subscription = cx.subscribe(&search_input, |this, _, _: &InputEvent, cx| {
            // 过滤会让「列表」和「没搜到」两种空态互换，页面高度会变；
            // 缓存的 measured_size 得丢掉，下一帧才会按真实高度重新量。
            this.measured_size.set(None);
            this.rebuild_display_order(cx);
            cx.notify();
        });
        Self {
            detail,
            _detail_subscription: detail_subscription,
            playlist_id: None,
            is_favorite_view: false,
            tabs,
            _tabs_subscription: tabs_subscription,
            library,
            playback,
            _playback_subscription: playback_subscription,
            _library_subscription: library_subscription,
            display_order: Vec::new(),
            song_display: Vec::new(),
            search_input,
            _search_subscription: search_subscription,
            search_keyword: SharedString::default(),
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

    /// 打开一个歌单。`summary` 是调用方手上已有的歌单摘要（搜索结果里那份），
    /// 没有时才回音乐库里找 —— 侧栏和收藏歌单的摘要本来就在库里。
    /// 给了摘要就先渲染页头，详情接口返回后再补齐歌曲列表。
    pub fn open(&mut self, id: Option<u64>, summary: Option<Playlist>, cx: &mut Context<Self>) {
        let summary = summary.or_else(|| {
            self.library
                .read(cx)
                .playlists
                .iter()
                .find(|playlist| Some(playlist.id) == id)
                .cloned()
        });
        self.detail
            .update(cx, |detail, cx| detail.open(id, summary, cx));
    }

    /// 记录导航意图（是否「我喜欢的音乐」页）；未登录时标题靠它。
    pub fn set_favorite_view(&mut self, favorite: bool) {
        self.is_favorite_view = favorite;
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
                // 「是否我喜欢的音乐」由 open_playlist 在导航时写入 is_favorite_view，
                // 未登录拿不到接口数据也照样成立，标题只有一个真值来源。
                if self.is_favorite_view {
                    "我喜欢的音乐".into()
                } else {
                    self.detail
                        .read(cx)
                        .playlist
                        .as_ref()
                        .map(|playlist| playlist.name.clone())
                        .unwrap_or_else(|| "歌单".into())
                },
            )
    }

    fn playlist_metadata(&self, colors: ColorTokens, cx: &Context<Self>) -> Div {
        let detail = self.detail.read(cx);
        let playlist = detail.playlist.as_ref();
        let cover_url = self.cover_url(cx);
        let play_count = playlist.map(|playlist| playlist.play_count);
        let created_date = playlist
            .and_then(|playlist| playlist.create_time)
            .and_then(|create_time| {
                time::OffsetDateTime::from_unix_timestamp(create_time / 1000).ok()
            })
            .map(|date| {
                format!(
                    "{}-{:02}-{:02}创建",
                    date.year(),
                    u8::from(date.month()),
                    date.day()
                )
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

    /// 显示顺序 = 命中关键词的行，按当前排序排好。
    ///
    /// 过滤和排序都只改 `display_order`（`detail.songs` 的下标），歌曲数据本身不动，
    /// 列宽跟着显示行数重算，序号列因此在过滤后自动收窄。
    ///
    /// 顺带把 trim 后的关键词存进 `search_keyword` 供高亮用。保持原始大小写：
    /// 「没有找到『X』」要照用户输入的样子显示，大小写折叠统一由 `keyword_ranges` 负责。
    fn rebuild_display_order(&mut self, cx: &App) {
        let sort = self.sort;
        let keyword: SharedString = self.search_input.read(cx).value().trim().into();
        let songs = &self.detail.read(cx).songs;
        let matched = (0..songs.len())
            .filter(|&index| keyword.is_empty() || song_matches(&songs[index], &keyword))
            .collect();
        self.display_order = match sort {
            Some((column, descending)) => sort_indices(matched, songs, column, descending),
            None => matched,
        };
        self.search_keyword = keyword;
        self.columns = table_columns(self.display_order.len(), self.album_share);
    }

    fn toggle_sort(&mut self, column: SongSort, cx: &mut Context<Self>) {
        self.sort = next_sort(self.sort, column);
        // 排序只在「已经筛出来的行」里重排，搜索结果不会因为换排序而忽多忽少。
        self.rebuild_display_order(cx);
        cx.notify();
    }

    /// 歌单内搜索框：白底胶囊，左边放大镜、右边输入区。
    ///
    /// `progress` 是 0（收起）到 1（展开）的动画进度，只用来插值宽度。
    /// 占位文字两种状态都显示 —— 收起时也看得见「搜索」，胶囊才像个能被认出来的
    /// 搜索框而不是一颗没来由的圆点。
    ///
    /// `has_value` 由 `render` 算好传进来：宽度和清空按钮要基于同一个判断。
    fn search_box(
        &self,
        progress: f32,
        has_value: bool,
        border: Hsla,
        colors: ColorTokens,
        cx: &Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        div()
            .id("playlist-search-box")
            .occlude()
            .h(SEARCH_BOX_HEIGHT)
            .w(
                SEARCH_COLLAPSED_WIDTH
                    + (SEARCH_EXPANDED_WIDTH - SEARCH_COLLAPSED_WIDTH) * progress,
            )
            // 固定宽度、只由动画改：flex_none 免得行内 flex 把它当可伸缩项重新分配。
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .bg(white())
            .border_1()
            .border_color(border)
            .rounded_full()
            // 收起时宽度仍够放下占位文字，所以这里不淡出；留着它是为了输入了
            // 文字又失焦时（收起态仍显示有内容），长文本在圆角处裁得干净。
            .overflow_hidden()
            // 边框比宽度先变白：胶囊底色本来就是白的，边框退成背景色，
            // 视觉上就是这排控件被「托起来」了。
            .hover(|style| style.border_color(white()))
            // 点胶囊外面失焦。这一条得自己写：GPUI 从不在按下时清焦点，
            // `Window::blur` 只有 a11y 动作才会调，所以不写的话焦点会一直挂着，
            // 点去别的搜索框才「顺便」失焦。`on_mouse_down_out` 走 capture 阶段
            // 且只做几何判定，点在胶囊外（含被 occlude 挡住的地方）就回调。
            .on_mouse_down_out({
                let focus = self.search_input.focus_handle(cx);
                move |_, window, cx| {
                    // 只在自己真有焦点时才放手。这个监听器对任何外部点击都会触发，
                    // 无差别 blur 会顺手清掉播放条、登录窗等别处的焦点。
                    if focus.is_focused(window) {
                        window.blur(cx);
                    }
                }
            })
            // 整颗胶囊都是点击热区：`Input` 自己只占中间那截，点图标或两侧留白
            // 不会命中它。这里把焦点转交给真正的 InputState，胶囊才会展开。
            .on_click(move |_, window, cx| {
                view.update(cx, |this, cx| {
                    window.focus(&this.search_input.focus_handle(cx), cx);
                });
            })
            .child(
                svg()
                    .path("icons/search.svg")
                    .size(px(16.))
                    .flex_none()
                    .text_color(colors.muted_foreground)
                    .hover(|style| style.text_color(colors.foreground)),
            )
            .child(
                // 输入区宽度跟着胶囊走。占位文字在收起态就要露出来（收起宽度
                // 本来就够放它），所以不做淡入淡出，只让宽度动画负责「长出来」。
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .text_size(px(13.))
                    .text_color(colors.foreground)
                    .child(Input::new(&self.search_input)),
            )
            // 清空按钮贴在胶囊内侧右端。有内容时框必然是展开的（宽度和这个按钮
            // 共用 `has_value` 判断），所以这里不会挤到收起的 72px 上。
            .when(has_value, |box_| {
                box_.child(search_clear_button(
                    self.search_input.clone(),
                    "playlist-search-clear",
                    colors,
                ))
            })
            .into_any_element()
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

    fn set_hovered_row(&mut self, index: u64, hovered: bool, cx: &mut Context<Self>) {
        // 转移规则在 virtual_table 里，搜索页共用同一份，两边的 hover 表现才不会分叉。
        let next = hovered_row_after(self.hovered_row, index, hovered);
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
                                    .child(title_with_subtitle(
                                        display,
                                        &self.search_keyword,
                                        colors,
                                    )),
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
                                            .child(artist_label(
                                                song,
                                                &self.search_keyword,
                                                colors,
                                            )),
                                    ),
                            ),
                    )
                    // 隐形元素仍占宽度且可点击，因此只在 hover 时挂载操作图标。
                    .when(self.hovered_row == Some(song.id), |title| {
                        title.child(row_actions(song.id, song, colors))
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
                .child(text_with_matches(
                    display.album.clone(),
                    &self.search_keyword,
                ))
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

/// 关键词命中：标题、任一歌手、专辑任一包含即算命中。
///
/// 判定直接复用 `keyword_ranges`，所以「过滤」和「高亮」是同一套匹配，
/// 不会出现某行搜出来了、高亮却找不到位置的情况。搜的是子串不是前缀，
/// 所以「周」能命中歌名中间带「周」字的那首。
///
/// `tns`（音译）和 `alia`（别名）也算：它们显示成标题的副标题，
/// 用户看得到就一定会搜它 —— 用「daoxiang」搜「稻香」应该找得到。
fn song_matches(song: &Song, keyword: &str) -> bool {
    let hit = |text: &str| !keyword_ranges(text, keyword).is_empty();
    hit(&song.name)
        || song.tns.iter().any(|t| hit(t))
        || song.alia.iter().any(|a| hit(a))
        || song
            .ar
            .iter()
            .any(|artist| artist.name.as_deref().is_some_and(hit))
        || song.al.name.as_deref().is_some_and(hit)
}

/// 把给定的一批下标按列排序。
///
/// 过滤与排序拆成两步：过滤先决定「哪些行出现」，排序只在这批下标内部重排，
/// 所以 `detail.songs` 的原始顺序始终不动，搜索结果也不会因为换排序而变多变少。
fn sort_indices(
    mut order: Vec<usize>,
    songs: &[Song],
    column: SongSort,
    descending: bool,
) -> Vec<usize> {
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

impl Render for PlaylistPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.playing_indicator.read(cx).placement.set(None);
        self.table_header_hidden.set(false);
        let colors = Theme::global(cx).tokens.colors;
        // 展开与否只由「有没有焦点」和「有没有字」决定：光标停在框里就该让用户看得见
        // 自己能往哪打字，输入过字就该留着结果让人对照，鼠标 hover 不算 —— 它只是探过来看一眼。
        //
        // 「有字」看原始内容（哪怕只有一个空格），过滤用的关键词才是 trim 结果：
        // 宽度和清空按钮共用 `has_value`，若看 trim 后的，只输入一个空格时
        // 按钮会挤进收起的 72px 里。
        let has_value = !self.search_input.read(cx).value().is_empty();
        let keyword = self.search_keyword.clone();
        let expanded = self.search_input.focus_handle(cx).is_focused(window) || has_value;
        // 一条进度通道同时驱动宽度和文字淡入，两者不会错拍；
        // 库自己处理 reduced motion 和掉帧重绘，这里只管给目标值。
        let progress = transition(
            "playlist-search",
            if expanded { 1. } else { 0. },
            Transition::new(SEARCH_EXPAND_DURATION).ease(ease_out_quint()),
            window,
            cx,
        );
        // 边框走独立通道、时长更短：先把边界交代清楚，宽度再跟上，读起来是两个动作。
        let border = blend_color(
            colors.border,
            white(),
            transition(
                "playlist-search-border",
                if expanded { 1. } else { 0. },
                Transition::new(SEARCH_BORDER_DURATION).ease(ease_out_quint()),
                window,
                cx,
            ),
        );
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
            // 未登录（库级错误）时与官方一致：不弹登录提示，直接给空状态插画。
            _ if error.is_some() && detail.id.is_none() && library.user_id == 0 => {
                centered_empty("images/empty/cm_empty_wuzuopin.png", "空空如也", colors)
                    .into_any_element()
            }
            _ if error.is_some() => div()
                .text_color(colors.muted_foreground)
                .child(error.unwrap())
                .when(detail.id.is_some() || library.user_id != 0, |message| {
                    message.child(Button::new("playlist-retry").ml_2().child("重试").on_click(
                        cx.listener(|this, _, _, cx| {
                            if this.detail.read(cx).id.is_some() {
                                this.open(this.playlist_id, None, cx);
                            } else {
                                this.library.update(cx, |library, cx| library.retry(cx));
                            }
                        }),
                    ))
                })
                .into_any_element(),
            _ if detail.id.is_none() => {
                centered_empty("images/empty/cm_empty_wuzuopin.png", "空空如也", colors)
                    .into_any_element()
            }
            // 「歌单本身是空的」和「搜了但没搜到」要说两句话：
            // 混成同一句「空空如也」会让人以为歌单被清空了，进而去找重试按钮。
            PlaylistTab::Songs if self.display_order.is_empty() => {
                if keyword.is_empty() {
                    centered_empty("images/empty/cm_empty_wuzuopin.png", "空空如也", colors)
                        .into_any_element()
                } else {
                    div()
                        .w_full()
                        .py(px(36.))
                        .flex()
                        .justify_center()
                        .text_size(px(13.))
                        .text_color(colors.muted_foreground)
                        .child(format!("没有找到与「{keyword}」相关的歌曲"))
                        .into_any_element()
                }
            }
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
                    |this, key, position, window, cx| {
                        let Some(song) = this
                            .detail
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
            // 控件区域：标签在左、搜索在右，justify_between 让胶囊贴住右边缘，
            // 展开时向左「长」出去，标签的位置不受影响。
            .child(
                div()
                    .mt(px(28.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(self.tabs.clone())
                    .child(self.search_box(progress, has_value, border, colors, cx)),
            )
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
    use super::{Song, SongSort, next_sort, song_matches, sort_indices};

    /// 全部下标，等价于「没有关键词」时的显示顺序。
    fn song_order(songs: &[Song], column: SongSort, descending: bool) -> Vec<usize> {
        sort_indices((0..songs.len()).collect(), songs, column, descending)
    }

    /// `Artist` / `Album` 没有从 `crate::models` 导出，测试就用最小 JSON 造条目，
    /// 顺带也覆盖了真实响应的字段形状（和 playlist_detail 的测试同一套做法）。
    fn song(name: &str, artist: &str, album: Option<&str>) -> Song {
        serde_json::from_value(serde_json::json!({
            "id": 1,
            "name": name,
            "ar": [{"id": 1, "name": artist}],
            "al": {"id": 1, "name": album},
            "dt": 0,
        }))
        .unwrap()
    }

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
    fn keyword_matches_title_artist_and_album_case_insensitively() {
        let track = song("Sunny Day", "Taylor Swift", Some("Fearless"));
        assert!(song_matches(&track, "sunny"));
        assert!(song_matches(&track, "taylor"));
        assert!(song_matches(&track, "fearless"));
        // 专辑缺失不应 panic，只是不命中。
        assert!(!song_matches(&song("a", "b", None), "fearless"));
        assert!(!song_matches(&track, "周"));
    }

    #[test]
    fn sorting_a_filtered_subset_only_reorders_matched_rows() {
        let songs = vec![
            song("b", "z", None),
            song("a", "y", None),
            song("c", "x", None),
        ];
        let matched: Vec<usize> = (0..songs.len()).filter(|&index| index != 1).collect();
        // 排序只在筛出来的下标里重排，结果就是全量排序去掉被滤掉的那行 ——
        // 被滤掉的歌不会因为换排序又冒回来。
        assert_eq!(song_order(&songs, SongSort::Title, false), [1, 0, 2]);
        assert_eq!(
            sort_indices(matched.clone(), &songs, SongSort::Title, false),
            [0, 2]
        );
        assert_eq!(sort_indices(matched, &songs, SongSort::Title, true), [2, 0]);
    }

    /// `tns`（音译）和 `alia`（别名）会显示成标题的副标题，用户看得见就一定会搜它。
    /// 「稻香」的副标题是 `（daoxiang）`，所以 `daoxiang` 必须能搜到它。
    #[test]
    fn keyword_also_matches_the_rendered_subtitle() {
        let mut track = song("稻香", "周杰伦", Some("叶惠美"));
        track.tns = vec!["dao xiang".into()];
        track.alia = vec!["稻香 Theme".into()];

        // 标题 / 歌手 / 专辑照旧。
        assert!(song_matches(&track, "稻香"));
        assert!(song_matches(&track, "周杰伦"));
        assert!(song_matches(&track, "叶惠美"));
        // 副标题里的命中也算，大小写不敏感。
        assert!(song_matches(&track, "xiang"));
        assert!(song_matches(&track, "DAO"));
        assert!(song_matches(&track, "theme"));
        assert!(!song_matches(&track, "不存在"));
    }
}
