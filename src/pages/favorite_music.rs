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
///
/// 字体内嵌在二进制里、字号也固定，所以这些数事先就能知道：查 `assets/font/dolphin.ttf`
/// 的 `hmtx` 表，数字占 520 或 600 units（em = 1000 units）。取最宽的 0.6em 就是所有
/// 数字的上界 —— 管它渲染出来是 `1` 还是 `7`，按最宽的算都不会挤到换行。
const DOLPHIN_WIDEST_DIGIT_EM: f32 = 0.6;

/// 序号列宽：字体和字号都是我们定的，一位数字占多宽是个已知常数，乘位数即可。
///
/// 不能交给布局自己撑开：`virtual_table` 为了虚拟化，表头和每一行都是**各自独立**的
/// 布局根，由内容撑开的话每行都会算出不同宽度，列就错位了。所以只能算一次再复用。
fn index_column_width(row_count: usize) -> Pixels {
    // 末行渲染的是 `format!("{:02}", row_count)`，它就是最长的那个序号。
    let digits = format!("{:02}", row_count).chars().count() as f32;
    // 字号跟行内保持一致（ROW_TEXT_SIZE），以后改字号不用回来动这里。
    let width = digits * DOLPHIN_WIDEST_DIGIT_EM * f32::from(ROW_TEXT_SIZE);
    // 向上取整：小数宽度按设备像素取整后可能差一丁点，而这点差距就够让数字折行。
    px(width.ceil())
}

/// 「专辑」列固定宽度，且由分界线的拖动决定；「标题」列则是弹性的（`None`），
/// 窗口变宽时多余的空间归它 —— 和原版一致。这个组合也是分界线能拖的前提，
/// 见 `divider_right_offset`。
const ALBUM_WIDTH_DEFAULT: Pixels = px(230.);
const ALBUM_WIDTH_MIN: Pixels = px(80.);
/// 上限是防止专辑列把弹性的标题列挤没，那样整行会溢出。
const ALBUM_WIDTH_MAX: Pixels = px(400.);

/// 歌曲标题和它右边副标题的字号。固定值，不跟着行内的 `ROW_TEXT_SIZE` 联动；
/// 两者必须是同一个值，否则副标题看起来会被"降级"。
const TITLE_TEXT_SIZE: Pixels = px(14.);

/// 音质徽章的显示高度，宽度交给 `img()` 按各自比例算（见 `Quality` 的注释）。
///
/// 7 个图标都是**原生 13 高**：它们取自网易云同一套徽章的小号版本（大号是 16 高），
/// 所以按 13 显示就是 1:1 —— 描边和字都不会被重采样，这是选这套图标的理由，别随手改。
/// 想整体放大只管改这里，但要知道那就不是原尺寸了。
const QUALITY_BADGE_HEIGHT: Pixels = px(13.);

/// 歌曲的音质标识，显示在歌手前面。
///
/// 这些徽章图标自带配色（金色描边），而且宽高各不相同，所以必须用 `img()` 渲染：
/// `svg()` 会把它当成单色遮罩、用 `text_color` 重绘，原始配色会丢。
#[derive(Clone, Copy)]
enum Quality {
    Hq,
    Sq,
    HiRes,
    /// 全景声
    Spatial,
    /// 沉浸声
    Immersive,
    /// 超清母带
    Master,
    /// 高清臻音
    Hd,
}

impl Quality {
    /// 全部档位，示例数据用它按行轮换，好把每个徽章都看一眼。
    const ALL: [Quality; 7] = [
        Quality::Hq,
        Quality::Sq,
        Quality::HiRes,
        Quality::Spatial,
        Quality::Immersive,
        Quality::Master,
        Quality::Hd,
    ];

    fn badge(self) -> &'static str {
        match self {
            Self::Hq => "icons/音质选项/HQ.svg",
            Self::Sq => "icons/音质选项/sq.svg",
            Self::HiRes => "icons/音质选项/Hi-Res.svg",
            Self::Spatial => "icons/音质选项/全景声.svg",
            Self::Immersive => "icons/音质选项/沉浸声.svg",
            Self::Master => "icons/音质选项/超清母带.svg",
            Self::Hd => "icons/音质选项/高清臻音.svg",
        }
    }
}

/// 次要文字（副标题、艺术家、专辑、时长）的字重：比正文细一档。
///
/// `FontWeight` 就是 f32 的包装，想取 Light/Regular 之间的中间值可以写
/// `FontWeight::from(350.)`（能不能落到真正的中间字面取决于系统字体有没有那一档）。
const SECONDARY_FONT_WEIGHT: FontWeight = FontWeight::NORMAL;

/// 「喜欢」「时长」的固定宽度：两列都在分界线右边，反推分界线位置要用到它们。
const LIKE_COLUMN_WIDTH: Pixels = px(42.);
const DURATION_COLUMN_WIDTH: Pixels = px(66.);

/// 分界线的元素 id，同时用作它作为 hover group 的名字。
/// 两处必须完全一致 `group_hover` 才会响应，所以抽成常量而不是写两遍字面量。
const DIVIDER_ID: &str = "favorite-title-divider";

/// 拖动分界线时随 drag 传递的载荷 `(起拖时的鼠标 x, 起拖时的专辑列宽)`。
///
/// 用位移换算而不是鼠标的绝对坐标：表格不在窗口的 x=0 处，它的左边距（侧边栏 + 页面
/// 内边距）页面拿不到；而且位移是幂等的，一帧里来几个鼠标事件重复算也不会漂。
type DividerDragAnchor = Rc<Cell<(Pixels, Pixels)>>;

use super::ContentPage;
use crate::components::{
    COLUMN_GAP, HEADER_HEIGHT, ROW_TEXT_SIZE, ResizeDragPreview, TabBar, TabChanged, TabItem,
    TableColumn, virtual_table,
};
use crate::state::user::UserProfile;
use crate::theme::{DOLPHIN_FAMILY, IconSize, PRESSED_OPACITY};

// 「更多」菜单里的三个命令。
//
// `NativeMenu` 的每一项都挂一个 GPUI `Action`，选中后由 `Window::dispatch_action`
// 派发 —— 和系统菜单栏、快捷键走的是同一套机制，所以将来只要在某个视图上
// `on_action(...)` 就能接住，不必改菜单代码。现在还没有任何监听者，
// 选中即派发到空处，等于什么都不做。
actions!(favorite_music, [Share, BatchOperation, AddAllToPlaylist]);

#[derive(Clone, Copy)]
enum FavoriteMusicTab {
    Songs,
    Comments,
    Collectors,
}

impl FavoriteMusicTab {
    fn content_title(self) -> &'static str {
        match self {
            Self::Songs => "歌曲列表",
            Self::Comments => "评论",
            Self::Collectors => "收藏者",
        }
    }
}

/// 我喜欢的音乐
pub struct FavoriteMusicPage {
    user_profile: Entity<UserProfile>,
    _user_profile_subscription: Subscription,
    tabs: Entity<TabBar>,
    _tabs_subscription: Subscription,
    songs: Vec<DemoSong>,
    /// 序号列宽：构造时按当前数据量（最长序号有几位）算一次。
    /// 它不随每帧重算，所以以后接入真实歌单、行数变了，要在这里重算。
    index_width: Pixels,
    /// 专辑列宽：由「标题 / 专辑」之间那条分界线拖出来，所以是要改的状态。
    album_width: Pixels,
}

/// 临时示例数据，用于验证长表滚动；接入远程歌单后替换。
struct DemoSong {
    title: SharedString,
    /// 副标题（比如所属 EP）：渲染在标题右边、括号里，同一字号、灰色。
    subtitle: Option<SharedString>,
    /// 艺术家可能不止一个，渲染时用「 / 」连起来。
    artists: Vec<SharedString>,
    /// 音质徽章，显示在歌手前面。
    quality: Quality,
    album: SharedString,
    duration: SharedString,
    liked: bool,
    cover: SharedString,
}

fn demo_songs() -> Vec<DemoSong> {
    // (标题, 副标题, 艺术家, 专辑, 时长)
    let examples: [(&str, Option<&str>, &[&str], &str, &str); 6] = [
        (
            "Lose My Mind (feat. Doja Cat)",
            None,
            &["Don Toliver", "Doja Cat"],
            "Hardstone Psycho",
            "03:29",
        ),
        (
            "DAMIDAMI",
            Some("《绝区零》卢西娅EP"),
            &["Sihan", "三Z-STUDIO", "HOYO-MiX"],
            "绝区零-DAMIDAMI",
            "03:11",
        ),
        (
            "Let You Down",
            None,
            &["Stonebank", "Danyka Nadeau"],
            "Let You Down",
            "04:13",
        ),
        ("Crown", None, &["BUNT."], "Crown", "04:00"),
        ("晴天", None, &["周杰伦"], "叶惠美", "04:29"),
        ("海阔天空", None, &["Beyond"], "乐与怒", "05:24"),
    ];
    (0..1193)
        .map(|index| {
            let (title, subtitle, artists, album, duration) = examples[index % examples.len()];
            DemoSong {
                title: title.into(),
                subtitle: subtitle.map(SharedString::from),
                artists: artists
                    .iter()
                    .map(|artist| SharedString::from(*artist))
                    .collect(),
                // 音质按行轮换，一屏里就能把每个徽章都看到。
                quality: Quality::ALL[index % Quality::ALL.len()],
                album: album.into(),
                duration: duration.into(),
                // 混着来：既能看到点亮时的实心红心，也能看到未点亮的勾线灰心。
                liked: index % 4 != 0,
                cover: "images/demo-album.svg".into(),
            }
        })
        .collect()
}

/// 把标题和（可选）副标题拼成**一整行**文字，副标题那段换成灰色 + 细字重。
///
/// 为什么用 `StyledText` + highlights 而不是两个元素并排：并排的两个文本元素各截各的，
/// 收缩时会出现两个省略号。highlights 是在排版时叠加到**继承来的**文字样式上的
/// （字号、家族都跟随父级），整串仍是一行文字，所以只会有一个省略号 ——
/// 效果等价于 HTML 里标题后面套一个 `<span style="color: gray">`。
fn title_with_subtitle(song: &DemoSong, colors: ColorTokens) -> StyledText {
    let Some(subtitle) = &song.subtitle else {
        return StyledText::new(song.title.clone());
    };

    let subtitle = format!("（{subtitle}）");
    let title_len = song.title.len();
    let mut line = song.title.to_string();
    line.push_str(&subtitle);

    // run 级颜色要先合成成不透明色，原因在 `TextStyle::highlight`：
    // 它是 `继承色.blend(这里的颜色)`，而 `blend` 是"把 other 叠在 self 上"。
    // 主题里的灰都是半透明的 ink，半透明灰叠在不透明的标题色上，出来的还是接近标题色
    // —— 看着就像"副标题根本没变灰"。压在页面底色上算出等价的不透明灰，
    // 再传进来就会命中 `blend` 的 `other.a >= 1.0` 分支，直接原样使用。
    let subtitle_color = colors.background.blend(colors.muted_foreground);

    StyledText::new(line).with_highlights([(
        title_len..title_len + subtitle.len(),
        HighlightStyle {
            color: Some(subtitle_color),
            font_weight: Some(SECONDARY_FONT_WEIGHT),
            ..Default::default()
        },
    )])
}

impl FavoriteMusicPage {
    pub fn new(user_profile: Entity<UserProfile>, cx: &mut Context<Self>) -> Self {
        let user_profile_subscription = cx.observe(&user_profile, |_, _, cx| cx.notify());
        let tabs = cx.new(|_| {
            TabBar::new(vec![
                TabItem::new("歌曲").count("1193"),
                TabItem::new("评论"),
                TabItem::new("收藏者").count("5"),
            ])
        });
        let tabs_subscription = cx.subscribe(&tabs, |_, _, _: &TabChanged, cx| cx.notify());
        // 序号列宽依赖数据量（最长序号的位数），所以先建数据再算宽度。
        let songs = demo_songs();
        let index_width = index_column_width(songs.len());
        Self {
            user_profile,
            _user_profile_subscription: user_profile_subscription,
            tabs,
            _tabs_subscription: tabs_subscription,
            songs,
            index_width,
            album_width: ALBUM_WIDTH_DEFAULT,
        }
    }

    /// 列定义。只有「专辑」的宽度会被分界线改，所以每次渲染现算一份，
    /// 省得改宽度时还要记得同步刷新一个 `columns` 字段。一共 5 项，成本可以忽略。
    fn columns(&self) -> Rc<Vec<TableColumn>> {
        Rc::new(vec![
            TableColumn::new("#", Some(self.index_width)).align_right(),
            // 标题列弹性：多出来的宽度归它，和原版一样。
            TableColumn::new("标题", None),
            TableColumn::new("专辑", Some(self.album_width)),
            TableColumn::new("喜欢", Some(LIKE_COLUMN_WIDTH)),
            TableColumn::new("时长", Some(DURATION_COLUMN_WIDTH)),
        ])
    }

    /// 分界线到行右边缘的距离：`专辑宽 + 间距 + 喜欢宽 + 间距 + 时长宽`，
    /// 正好是标题与专辑之间那个间距的右边缘。
    ///
    /// 右边这三列都是固定宽度，所以从**右边**量是确定的；左边是弹性的标题列，宽度由布局
    /// 算出来、页面拿不到。这就是分界线用 `.right()` 而不是 `.left()` 定位的原因，也是
    /// 「标题弹性 + 专辑固定」这个组合能拖的前提 —— 两侧都弹性的话位置就无从算起了。
    fn divider_right_offset(&self) -> Pixels {
        self.album_width + COLUMN_GAP + LIKE_COLUMN_WIDTH + COLUMN_GAP + DURATION_COLUMN_WIDTH
    }

    /// 「标题 / 专辑」之间的分界线：拖动它，专辑列变窄/变宽（我们存着的固定宽度），
    /// 弹性的标题列反向伸缩，视觉上这条边界就跟着鼠标走。位置见 `divider_right_offset`。
    ///
    /// 它必须是表格的**兄弟节点**并绝对定位：每一行都是独立的布局根，塞进某一行的单元格里
    /// 只会影响那一行；而单元格都带 `overflow_hidden`，塞进去的东西一超出格子就被裁掉。
    ///
    /// 高度只盖住表头（行区域不拦鼠标），平时完全透明、靠光标变化提示可拖；悬停时露出一小段
    /// 竖条，形状抄 `TabBar` 选中项下面那根小横条，只是转了 90° 并换成灰色。
    fn title_divider(&self, colors: ColorTokens, cx: &Context<Self>) -> AnyElement {
        let anchor: DividerDragAnchor = Rc::new(Cell::new((px(0.), self.album_width)));
        div()
            .id(DIVIDER_ID)
            .group(DIVIDER_ID)
            .absolute()
            .top_0()
            .h(HEADER_HEIGHT)
            .right(self.divider_right_offset())
            .w(COLUMN_GAP)
            .flex()
            .items_center()
            .justify_center()
            .cursor_col_resize()
            .child(
                // 3 宽 × 16 高的小竖条，flex 居中后正好落在两条列边界中间。
                div()
                    .w(px(3.))
                    .h(px(16.))
                    .rounded(px(1.5))
                    .group_hover(DIVIDER_ID, |style| style.bg(colors.border)),
            )
            .on_drag(anchor.clone(), move |_, _, window, cx| {
                // 起拖这一刻记下鼠标位置和当时的专辑列宽，之后按位移换算新宽度。
                let (_, start_width) = anchor.get();
                anchor.set((window.mouse_position().x, start_width));
                cx.new(|_| ResizeDragPreview)
            })
            .on_drag_move(cx.listener(
                move |this, event: &DragMoveEvent<DividerDragAnchor>, _, cx| {
                    let (start_x, start_width) = event.drag(cx).get();
                    let moved = event.event.position.x - start_x;
                    // 往右拖 = 分界线右移 = 专辑列变窄（它的右边缘被后面的固定列钉住了），
                    // 少掉的那部分正好加给弹性的标题列。
                    this.album_width =
                        (start_width - moved).clamp(ALBUM_WIDTH_MIN, ALBUM_WIDTH_MAX);
                    cx.notify();
                },
            ))
            .into_any_element()
    }

    fn song_cells(&mut self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = Theme::global(cx).tokens.colors;
        let song = &self.songs[index];
        let liked = song.liked;
        vec![
            div()
                // 铺满单元格后再靠右对齐；nowrap 兜底，序号永远不该折成两行。
                .w_full()
                .text_right()
                .whitespace_nowrap()
                .text_color(colors.muted_foreground)
                .font_family(DOLPHIN_FAMILY)
                .child(format!("{:02}", index + 1))
                .into_any_element(),
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
                        .bg([
                            rgb(0x537b83),
                            rgb(0x94705b),
                            rgb(0x60658b),
                            rgb(0x8b687e),
                            rgb(0x657853),
                            rgb(0x566678),
                        ][index % 6])
                        // 只在可见行中创建 img；以后 cover 可直接换成远程 URL。
                        .child(
                            img(song.cover.clone())
                                .size_full()
                                .object_fit(ObjectFit::Cover),
                        ),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        // 标题行：标题和副标题是同一行文字（见 title_with_subtitle），
                        // 所以窄了只会出现一个省略号。
                        .child(
                            div()
                                .text_size(TITLE_TEXT_SIZE)
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
                                .child(
                                    img(song.quality.badge())
                                        .h(QUALITY_BADGE_HEIGHT)
                                        .flex_none(),
                                )
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .truncate()
                                        .font_weight(SECONDARY_FONT_WEIGHT)
                                        .text_color(colors.muted_foreground)
                                        .child(song.artists.join(" / ")),
                                ),
                        ),
                )
                .into_any_element(),
            div()
                .truncate()
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(colors.secondary_foreground)
                .child(song.album.clone())
                .into_any_element(),
            // 裸 SVG，没有圆形底色：已喜欢是实心红心，未喜欢是勾线灰心。
            svg()
                .path(if liked {
                    "icons/like.svg"
                } else {
                    "icons/like_outline.svg"
                })
                // 和 header 右上角那排图标同一个尺寸。
                .size(IconSize::Small.pixels())
                .cursor_pointer()
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
                .id(("favorite-song-like", index))
                .role(Role::Button)
                .aria_label(if liked { "取消喜欢" } else { "喜欢" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.songs[index].liked = !this.songs[index].liked;
                    cx.notify();
                }))
                .into_any_element(),
            div()
                // 时长是辅助信息：比 `muted_foreground`(60%) 再淡一档（45%），字重也细一档。
                .font_weight(SECONDARY_FONT_WEIGHT)
                .text_color(colors.foreground.alpha(0.45))
                .child(song.duration.clone())
                .into_any_element(),
        ]
    }
}

/// 「播放全部」：主题红实心按钮，白字白图标，无边框。
///
/// 主操作用实心色块来吸引视线，所以它刻意不描边 —— 描边在纯色填充上
/// 只会削弱色块边缘的锐利感。
fn play_all_button(colors: ColorTokens) -> Button {
    Button::new("favorite-play-all-button")
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

impl Render for FavoriteMusicPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let active_tab = [
            FavoriteMusicTab::Songs,
            FavoriteMusicTab::Comments,
            FavoriteMusicTab::Collectors,
        ][self.tabs.read(cx).selected_index()];
        let cover_path = "/Users/amagicpear/Pictures/Perry Origin Character/ChatGPT Image 2026年9月29日 15_39_30.png";
        let user_profile = self.user_profile.read(cx);
        let content = match active_tab {
            FavoriteMusicTab::Songs => {
                let table = virtual_table(
                    cx.entity(),
                    "favorite-songs",
                    self.columns(),
                    self.songs.len(),
                    |this, index, _, cx| this.song_cells(index, cx),
                    cx,
                )
                .into_any_element();
                // 分界线要绝对定位在列边界上，所以表格外面要有一层 position: relative。
                div()
                    .relative()
                    .child(table)
                    .child(self.title_divider(colors, cx))
                    .into_any_element()
            }
            _ => div()
                .text_size(px(14.))
                .text_color(colors.muted_foreground)
                .child(active_tab.content_title())
                .into_any_element(),
        };

        div()
            // 封面标题区域
            .child(
                div()
                    .flex()
                    .justify_start()
                    .gap_6()
                    .child(
                        div()
                            .w(px(170.))
                            .h(px(170.))
                            .flex_none()
                            .relative()
                            .overflow_hidden()
                            // 歌单封面
                            .child(
                                img(cover_path)
                                    .size_full()
                                    .object_fit(ObjectFit::Cover)
                                    .rounded(px(8.)),
                            )
                            // 播放量
                            .child(
                                div()
                                    .absolute()
                                    .top_1()
                                    .right_2()
                                    .flex()
                                    .items_center()
                                    .child(
                                        svg()
                                            .path("icons/headphone.svg")
                                            .size(px(15.))
                                            .text_color(colors.primary_foreground),
                                    )
                                    .text_color(colors.primary_foreground)
                                    .text_size(px(15.))
                                    .font_family(DOLPHIN_FAMILY)
                                    .child(7735.to_string()),
                            )
                            // 正中间的爱心图标，仅「我喜欢的音乐」有
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        svg()
                                            .path("icons/like.svg")
                                            .size(px(96.))
                                            .text_color(colors.primary_foreground)
                                            .opacity(0.95),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .child(
                                div()
                                    .child(
                                        div()
                                            .text_size(px(24.))
                                            .line_height(rems(3.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(colors.foreground)
                                            .child(text!(ContentPage::FavoriteMusic.title())),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                Avatar::new()
                                                    .with_size(px(26.))
                                                    .flex_none()
                                                    .src(user_profile.avatar_path.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(colors.secondary_foreground)
                                                    .child(user_profile.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .ml_2()
                                                    .text_size(px(12.))
                                                    .text_color(colors.muted_foreground)
                                                    .font_weight(FontWeight::LIGHT)
                                                    .child("2017-05-31创建"),
                                            ),
                                    ),
                            )
                            // 操作按钮组：播放全部 / 下载 / 更多
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(play_all_button(colors))
                                    // 下载：宽度自适应，比 muted 更浅的底 + 比 muted 更浅的描边，
                                    // 文字/图标用比 muted_foreground 深一档的灰保证可读
                                    .child(
                                        Button::new("favorite-download-button")
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
                                        Button::new("favorite-more-button")
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
            .child(content)
    }
}
