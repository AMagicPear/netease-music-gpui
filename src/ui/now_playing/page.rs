//! 黑胶全屏页：外层滚动、背景、元信息与唱片协调。

mod comments;
mod lyrics;

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens, Theme};

use super::NowPlaying;
use crate::playback::PlaybackController;
use crate::state::library::MusicLibrary;
use crate::ui::assets::track_cover_url;
use crate::ui::components::{PLAYER_BAR_HEIGHT, Tonearm, Vinyl, format_duration, window_drag_area};
use crate::ui::cover_color::{
    Backdrop, CoverColor, CoverGradient, dark_colors, dark_gradient, gradient_layer,
};
use crate::ui::shell::{HEADER_HEIGHT, HEADER_TOP_INSET, hover_icon, mini_and_window_buttons};
use comments::{CommentsView, ListMode};
use lyrics::LyricsView;

const SUMMARY_HEIGHT: f32 = 72.;
const SONG_CONTENT_MAX_WIDTH: f32 = 1400.;
const SONG_CONTENT_PADDING: f32 = 100.;
const SONG_CONTENT_TOP_PADDING: f32 = 24.;
const VINYL_MAX_SIDE: f32 = 540.;
const CONTENT_MAX_ASPECT: f32 = 1.6;
const SUMMARY_WIDTH_RATIO: f32 = 0.8;
/// 判定外层「翻到评论区」的容差。滚动边界把 `scroll_max` 圆到两位小数、
/// 页高又可能是小数，差之毫厘不该让评论区在最后 1px 上反复开关。
const SCROLL_END_TOLERANCE: f32 = 1.;
/// 「回到唱片」把外层滑回歌词页的时长。和整页展开 / 收起同一档节拍。
const RETURN_DURATION: Duration = Duration::from_millis(600);
/// 补间的收敛容差，同时也是「偏移被谁动过」的判定门限。
const TWEEN_EPSILON: f32 = 0.5;
/// 吸附用的弹簧：临界阻尼（ζ = 1），ω₀ = 15 rad/s。
///
/// 临界阻尼就是不过冲——吸附是纠偏动作，来回摆一下会让人觉得页面在犹豫。
/// ω₀ 取 15：整页的落差（约 700px）0.48s 落定，几十像素的小修正 0.27s。
/// 弹簧的收敛时间只随落差的**对数**增长，所以小修正不会跟着一起变慢。
const SETTLE_SPRING: SpringConfig = SpringConfig::new(225., 30., 1.);
/// 弹簧的收敛容差（像素）。
const SETTLE_EPSILON: f32 = 0.5;
/// 停手判定：多久没有滚轮事件就算「手停了」。
///
/// 触控板抬指之后惯性还会一路派发事件，所以手势期间事件是连续的，一个短间隙
/// 就足够说明停手；鼠标滚轮没有抬指信号、两次咔哒之间本来就可能隔着几百毫秒，
/// 手指还按着的时候也只是在歇手，这两种情况都得用更长的窗口。
const SETTLE_QUIET_FINE: Duration = Duration::from_millis(110);
const SETTLE_QUIET_COARSE: Duration = Duration::from_millis(400);
/// 吸附落点的迟滞带。停在中段时按「最后往哪推」收紧或放宽：
/// 往评论区推着停下，四成就算数；往歌词页推着停下，得退到六成才回去。
/// 来向不明（只有零星事件）才退回中点。中间态是过路的，不该因为几像素之差
/// 就翻回去；但「推到哪儿算数」得听用户的意图，不能只看停在哪。
const SETTLE_COMMIT: (f32, f32) = (0.4, 0.6);
/// 判定「手往哪推」的最小单次位移（像素）。惯性的尾巴越来越小、方向却没变，
/// 噪声门限以下的事件不该改写来向。
const DIRECTION_NOISE: f32 = 0.5;
/// 页头「播放器模式」与悬浮「发布评论」跟着翻页进度淡出 / 淡入的区间（占一整屏的比例）。
const HEADER_FADE: (f32, f32) = (0.1, 0.5);
const PUBLISH_FADE: (f32, f32) = (0.55, 0.95);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SongTab {
    #[default]
    Lyrics,
    Encyclopedia,
    Similar,
}

/// 外层滚动的程序化运动。用户自己的滚轮不走这里。
///
/// 两种运动的性格不一样，所以是两个变体而不是一套参数：
/// - [`Motion::Command`]：点「回到唱片」发出的命令，固定节拍，和整页展开 / 收起同拍；
/// - [`Motion::Settle`]：延迟吸附，从当前偏移出发的临界阻尼弹簧。
///
/// 两者共用一条规矩：跑动中一旦发现偏移被别的来源改过（滚轮、窗口补位），
/// 立刻让位——手永远优先于动画。
#[derive(Default)]
struct ScrollMotion {
    active: Option<Motion>,
    /// 上一帧写进去的值，用来认「谁动了偏移」。
    written: Option<f32>,
}

enum Motion {
    /// 固定节拍的补间。
    Command {
        from: f32,
        target: f32,
        started: Instant,
    },
    /// 弹簧收敛。速度从 0 起算——手已经停了，落定本身不该带冲劲。
    Settle {
        target: f32,
        spring: SpringState,
        updated: Instant,
    },
}

impl ScrollMotion {
    fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// 命令式补间：从当前位置滑到 `target`。已经在飞向同一个目标就什么都不做。
    fn command(&mut self, handle: &ScrollHandle, target: Pixels, now: Instant) {
        let from = f32::from(handle.offset().y);
        let to = f32::from(target);
        if let Some(Motion::Command { target, .. }) = &self.active
            && *target == to
        {
            return;
        }
        if (to - from).abs() <= TWEEN_EPSILON {
            *self = Self::default();
            return;
        }
        self.active = Some(Motion::Command {
            from,
            target: to,
            started: now,
        });
        self.written = Some(from);
    }

    /// 弹簧收敛：从当前位置停到 `target`。已经在目标上就什么都不做。
    fn settle(&mut self, handle: &ScrollHandle, target: Pixels, now: Instant) {
        let from = f32::from(handle.offset().y);
        let to = f32::from(target);
        if (to - from).abs() <= SETTLE_EPSILON {
            *self = Self::default();
            return;
        }
        self.active = Some(Motion::Settle {
            target: to,
            spring: SpringState {
                position: from,
                velocity: 0.,
            },
            updated: now,
        });
        self.written = Some(from);
    }

    /// 推进运动；还在动就顺带要下一帧。
    ///
    /// 这里只能续上「已经在飞」的运动：起跑的那一帧必须由起跑处自己要帧，见
    /// [`Self::settle`] 调用点。`Motion::Command` 的起跑点在点击回调里（绘制之外），
    /// 那次 notify 已经把窗口弄脏了，第一帧自然会有；`Motion::Settle` 的起跑点在
    /// `render` 里，绘制之内 notify 不作数，只能 `request_animation_frame`。
    fn step(&mut self, handle: &ScrollHandle, window: &mut Window, now: Instant) -> bool {
        let running = self.advance(handle, now);
        if running {
            window.request_animation_frame();
        }
        running
    }

    fn advance(&mut self, handle: &ScrollHandle, now: Instant) -> bool {
        let Some(mut active) = self.active.take() else {
            return false;
        };
        // 上一帧写下的偏移被改过：有人接管了滚动，让位。
        if let Some(written) = self.written
            && (f32::from(handle.offset().y) - written).abs() > TWEEN_EPSILON
        {
            *self = Self::default();
            return false;
        }
        let (y, done) = match &mut active {
            Motion::Command {
                from,
                target,
                started,
            } => {
                let progress = (now.saturating_duration_since(*started).as_secs_f32()
                    / RETURN_DURATION.as_secs_f32())
                .min(1.);
                (
                    *from + (*target - *from) * ease_out_quint()(progress),
                    progress >= 1.,
                )
            }
            Motion::Settle {
                target,
                spring,
                updated,
            } => {
                *spring = SETTLE_SPRING.step(
                    *spring,
                    *target,
                    now.saturating_duration_since(*updated).as_secs_f32(),
                );
                *updated = now;
                (
                    spring.position,
                    SETTLE_SPRING.is_settled(*spring, *target, SETTLE_EPSILON),
                )
            }
        };
        handle.set_offset(point(px(0.), px(y)));
        self.written = Some(y);
        if done {
            // 到站整块清空：目标留着不清，下一次点「回到唱片」会被「目标没变」挡掉。
            *self = Self::default();
            return false;
        }
        self.active = Some(active);
        true
    }
}

/// 最近一次滚轮事件的样本。
#[derive(Clone, Copy)]
struct WheelSample {
    at: Instant,
    /// 精确位移（触控板）还是按行计的（鼠标滚轮）。
    fine: bool,
    /// 手指抬起来了没有。只有触控板会给出这个信号。
    lifted: bool,
    /// 方向：正数是往评论区推，负数是往歌词页推。
    direction: f32,
}

/// 「手停了没有」。
///
/// 滚轮事件是这一页唯一拿得到的「手」的信号。`ScrollWheelEvent::touch_phase`
/// 只有触控板会填（鼠标滚轮恒为 `Moved`），手指离开后惯性也还是 `Moved`，
/// 所以相位当不了「松手」用，只能配合事件间隙一起判。
#[derive(Default)]
struct WheelWatch {
    last: Option<WheelSample>,
}

impl WheelWatch {
    fn note(&mut self, now: Instant, event: &ScrollWheelEvent, line_height: Pixels) {
        let delta = f32::from(event.delta.pixel_delta(line_height).y);
        // 触控板一次新手势会带 `Started`：来向从这里重新算起，免得上一轮的方向
        // 被一串噪声事件继承下来。
        let carried = if event.touch_phase == TouchPhase::Started {
            0.
        } else {
            self.last.map_or(0., |sample| sample.direction)
        };
        let direction = if delta.abs() >= DIRECTION_NOISE {
            delta.signum()
        } else {
            carried
        };
        let lifted = match event.touch_phase {
            TouchPhase::Started => false,
            TouchPhase::Ended | TouchPhase::Cancelled => true,
            // 惯性段和鼠标滚轮都落在这里，沿用上一次的判断。
            TouchPhase::Moved => self.last.is_some_and(|sample| sample.lifted),
        };
        self.last = Some(WheelSample {
            at: now,
            fine: event.delta.precise(),
            lifted,
            direction,
        });
    }

    /// 手停了没有；停了就把这轮样本取走，下一次手势重新计时。
    fn released(&mut self, now: Instant) -> Option<f32> {
        let sample = self.last?;
        // 只有「触控板抬了指」才认短窗口：手指还按着时只是在歇手，别抢。
        let quiet = if sample.fine && sample.lifted {
            SETTLE_QUIET_FINE
        } else {
            SETTLE_QUIET_COARSE
        };
        if now.saturating_duration_since(sample.at) < quiet {
            return None;
        }
        self.last = None;
        Some(sample.direction)
    }

    /// 还在等窗口过去——这时要下一帧才判得了。
    fn waiting(&self) -> bool {
        self.last.is_some()
    }
}

pub(in crate::ui) struct NowPlayingPage {
    playback: Entity<PlaybackController>,
    library: Entity<MusicLibrary>,
    _library_subscription: Subscription,
    _playback_subscription: Subscription,
    /// 展开/收起的单一状态源。本页不再自持 `opened`，开合同步也靠它广播。
    now_playing: Entity<NowPlaying>,
    _now_playing_subscription: Subscription,
    lyrics: Entity<LyricsView>,
    comments: Entity<CommentsView>,
    vinyl: Entity<Vinyl>,
    summary_vinyl: Entity<Vinyl>,
    scroll: ScrollHandle,
    /// 外层的程序化运动：「回到唱片」的补间和延迟吸附共用它，只动外层。
    scroll_motion: ScrollMotion,
    /// 滚轮事件的采样。延迟吸附靠它判「手停了没有」和「最后往哪推」。
    wheel_watch: WheelWatch,
    cover_color: CoverColor,
    backdrop: Backdrop,
    song_id: Option<u64>,
    tonearm: Tonearm,
    tab: SongTab,
    page_height: Option<f32>,
}

struct FrameData {
    page_height: f32,
    height: f32,
    content_width: f32,
    side: f32,
    summary_top: f32,
    /// 评论区顶到视口顶的进度：0 = 歌词页，1 = 评论区整屏到位。
    comment_progress: f32,
    song_at_rest: bool,
    /// 评论列表这一帧听谁的，见 [`ListMode`]。
    list_mode: ListMode,
    reveal: f32,
    colors: ColorTokens,
    gradient: CoverGradient,
    title: String,
    artists: String,
    album: String,
    source: Option<String>,
    duration: String,
}

impl NowPlayingPage {
    pub(in crate::ui) fn new(
        playback: Entity<PlaybackController>,
        library: Entity<MusicLibrary>,
        now_playing: Entity<NowPlaying>,
        cx: &mut Context<Self>,
    ) -> Self {
        // 共享渐变底与旋转时钟都由 NowPlaying 持有：本页绘制/转动时写、播放栏与进度条读，
        // 句柄共享同一份，迷你碟切到大碟时相位才连续。
        let backdrop = now_playing.read(cx).backdrop();
        let clock = now_playing.read(cx).rotation_clock();
        let vinyl = cx.new(|cx| Vinyl::new(playback.clone(), clock.clone(), "images/disc.png", cx));
        let summary_vinyl =
            cx.new(|cx| Vinyl::new(playback.clone(), clock, "images/miniVinyl.png", cx));
        let scroll = ScrollHandle::default();
        let lyrics = cx.new(|cx| LyricsView::new(playback.clone(), backdrop.clone(), cx));
        // 评论区自己吃不下滚轮位移时让事件冒泡，交给外层平台滚动。
        let comments = cx.new(|_| CommentsView::new(backdrop.clone()));
        let mut source = source_name(&library, &playback, cx);
        let library_playback = playback.clone();
        let library_subscription = cx.observe(&library, move |this, library, cx| {
            let next = source_name(&library, &library_playback, cx);
            if next != source {
                source = next;
                if this.is_expanded(cx) {
                    cx.notify();
                }
            }
        });
        let mut playback_state = {
            let state = playback.read(cx).snapshot();
            (
                state.revision,
                state.is_playing,
                playback.read(cx).playlist_id(),
            )
        };
        let playback_subscription = cx.observe(&playback, move |this, playback, cx| {
            let state = playback.read(cx).snapshot();
            let next = (
                state.revision,
                state.is_playing,
                playback.read(cx).playlist_id(),
            );
            let song_id = state.current_song.as_ref().map(|song| song.id);
            let changed = this.song_id != song_id;
            if this.is_expanded(cx) {
                this.sync_song(cx);
            } else if changed && this.song_id.is_some() {
                // 隐藏后切歌只取消旧请求；下一次 open 再请求实际歌曲。
                this.song_id = None;
                this.lyrics.update(cx, |view, cx| view.set_song(None, cx));
                this.comments.update(cx, |view, cx| view.set_song(None, cx));
            }
            if this.is_expanded(cx) && (changed || next != playback_state) {
                cx.notify();
            }
            playback_state = next;
        });
        // 展开/收起的状态迁移由 NowPlaying 广播；本页在这里跑对应的副作用，
        // 播放栏、进度条、音量则各自 observe 它来重绘。
        //
        // 这个 `expanded` 守卫是必需的，不能省：NowPlaying 还会用 `publish_backdrop`
        // 每帧广播背景渐变，那些通知同样会打到这里。少了守卫，`on_expand` 会在整个
        // 600ms 渐变期间被反复触发，`reset_page()` 每帧复位一次，页面看起来就是
        // "自己弹回顶部"。这里只认状态位的边沿，渐变通知一律放过。
        let mut expanded = now_playing.read(cx).is_expanded();
        let now_playing_subscription = cx.observe(&now_playing, move |this, now_playing, cx| {
            let next = now_playing.read(cx).is_expanded();
            if next == expanded {
                return;
            }
            expanded = next;
            if next {
                this.on_expand(cx);
            } else {
                this.on_collapse(cx);
            }
        });
        Self {
            playback,
            library,
            _library_subscription: library_subscription,
            _playback_subscription: playback_subscription,
            now_playing,
            _now_playing_subscription: now_playing_subscription,
            lyrics,
            comments,
            vinyl,
            summary_vinyl,
            scroll,
            scroll_motion: ScrollMotion::default(),
            wheel_watch: WheelWatch::default(),
            cover_color: CoverColor::default(),
            backdrop,
            song_id: None,
            tonearm: Tonearm::default(),
            tab: SongTab::default(),
            page_height: None,
        }
    }

    fn sync_song(&mut self, cx: &mut Context<Self>) {
        let id = self
            .playback
            .read(cx)
            .snapshot()
            .current_song
            .as_ref()
            .map(|song| song.id);
        if self.song_id == id {
            return;
        }
        self.song_id = id;
        self.lyrics.update(cx, |view, cx| view.set_song(id, cx));
        self.comments.update(cx, |view, cx| view.set_song(id, cx));
        self.reset_page();
    }

    /// 回到歌词页：换歌、重新打开这一页都从这里起步。
    fn reset_page(&mut self) {
        self.scroll_motion = ScrollMotion::default();
        self.wheel_watch = WheelWatch::default();
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    /// 「回到唱片」：外层滑回歌词页。
    ///
    /// 只有外层偏移在动——评论列表保持自己的位置，整页连着评论区一起升上去，
    /// 中途不会有内容跳变。运动期间滚轮归谁见 [`ListMode::Held`]。
    fn return_to_song(&mut self, cx: &mut Context<Self>) {
        self.wheel_watch = WheelWatch::default();
        if cx.reduce_motion() {
            self.scroll.set_offset(point(px(0.), px(0.)));
        } else {
            let now = cx.background_executor().now();
            self.scroll_motion.command(&self.scroll, px(0.), now);
        }
        cx.notify();
    }

    /// 延迟吸附：手停下来之后，如果这一屏停在两个静止点之间，就把它送到该去的那一边。
    ///
    /// 「延迟」是这套交互的要点。两屏各占一整屏、中间态不是合法的落脚点，所以
    /// 停在中间终究要被送走；但**不**在用户还在推的时候改他的位置——那正是官方
    /// 客户端「定点强制滚动」手感的来源。这里让自由滚动一直连到手指离开，等滚轮
    /// 事件彻底静下来再收敛，用户看到的是「我停稳了，页面自己归位」。
    fn settle_if_idle(
        &mut self,
        expanded: bool,
        page_height: f32,
        now: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !expanded || self.scroll_motion.is_active() {
            return;
        }
        let scrolled = -f32::from(self.scroll.offset().y);
        // 两个静止点本来就是合法的落脚点，只有中间地带才需要归位。
        if song_page_at_rest(scrolled) || reached_comments(scrolled, page_height) {
            return;
        }
        let Some(direction) = self.wheel_watch.released(now) else {
            if self.wheel_watch.waiting() {
                // 还在等窗口过去：要下一帧才接着判。
                window.request_animation_frame();
            }
            return;
        };
        let target = settle_target(scrolled, page_height, direction);
        if cx.reduce_motion() {
            self.scroll.set_offset(point(px(0.), target));
        } else {
            self.scroll_motion.settle(&self.scroll, target, now);
        }
        // 起跑的这一帧得由这里自己要到下一帧，而且**不能**写成 `cx.notify()`。
        //
        // 本页是 cached 视图（shell 里 `.cached(..)`），cached 视图的 `render` 跑在
        // prepaint 而不是 layout 阶段；GPUI 此时 `WindowInvalidator` 的 draw_phase 是
        // `Prepaint`，`cx.notify()` 走不到「窗口脏了、下一帧重画」那一支——它只把本页
        // 记进 `dirty_views`，而那份记录在本次绘制结束时就被清掉了。于是没有人再画一帧，
        // `render` 没有下一次，弹簧永远停在起点：弹簧的初始位置就是手停下的那个偏移，
        // 看上去正好是「吸附卡在半路」。
        //
        // `window.request_animation_frame()` 才是这里能用的那句：它把回调排到本帧
        // 渲染之后（跑在绘制之外），回调里再 notify，窗口就真的脏了、真的会有下一帧。
        // 同理，暂停播放时看不出问题——播放 tick 每 100ms 从绘制之外通知一次，顺手
        // 把帧喂上了，弹簧被推着走完；一暂停就再没人喂帧，卡在原地。
        window.request_animation_frame();
    }

    /// 这一页是否展开。只读单一状态源，页面自己不再存副本。
    fn is_expanded(&self, cx: &App) -> bool {
        self.now_playing.read(cx).is_expanded()
    }

    pub(in crate::ui) fn vinyl_overlays(&self) -> [Entity<Vinyl>; 2] {
        [self.vinyl.clone(), self.summary_vinyl.clone()]
    }

    /// 展开时的副作用：回到歌词页起点并同步当前歌曲。
    fn on_expand(&mut self, cx: &mut Context<Self>) {
        self.reset_page();
        self.sync_song(cx);
        cx.notify();
    }

    /// 收起时的副作用：停掉歌词跟随、复位评论区，和展开时对称。
    fn on_collapse(&mut self, cx: &mut Context<Self>) {
        self.scroll_motion = ScrollMotion::default();
        self.wheel_watch = WheelWatch::default();
        self.lyrics
            .update(cx, |view, cx| view.set_active(false, cx));
        self.comments
            .update(cx, |view, cx| view.configure(ListMode::Parked, cx));
        cx.notify();
    }

    fn frame_data(&mut self, window: &mut Window, cx: &mut Context<Self>) -> FrameData {
        let expanded = self.now_playing.read(cx).is_expanded();
        let cover_url = self
            .playback
            .read(cx)
            .snapshot()
            .current_song
            .as_ref()
            .map(|song| track_cover_url(song.al.pic_url.as_deref(), 480));
        let color = if expanded {
            cover_url
                .as_deref()
                .and_then(|url| self.cover_color.load(url, window, cx))
        } else {
            self.cover_color.current()
        };
        let gradient = dark_gradient(color);
        let colors = dark_colors(Theme::global(cx).tokens.colors, gradient.0[1].into());
        let song = self.playback.read(cx).snapshot().current_song.clone();
        let page_height = (f32::from(window.viewport_size().height) - PLAYER_BAR_HEIGHT).max(1.);
        let height = (page_height - HEADER_HEIGHT).max(1.);
        // 展开/收起的节拍来自 NowPlaying：id、时长、曲线都只有它一处定义。
        let reveal = NowPlaying::reveal(expanded, window, cx);
        // 摘要条此刻在屏幕上的位置，唱片 / 摘要唱片的绘制用它做连续性判断。
        let summary_top = page_height + HEADER_TOP_INSET + f32::from(self.scroll.offset().y);
        // 「在不在评论区」不再是一个布尔位，而是这一屏翻过去的连续进度。页头文案和
        // 悬浮按钮都跟着它淡出 / 淡入：在中点硬切会在翻页途中造出一次肉眼可见的
        // 状态跳变，而那一刻场景里没有任何别的信号说明「换了状态」。原生滚动每一步
        // 都会通知本页重绘，这个值跟着滚动连续更新。
        let scrolled = -f32::from(self.scroll.offset().y);
        let comment_progress = (scrolled / page_height).clamp(0., 1.);
        // 两屏各有一个静止点，滚轮只归「已经停住」的那一屏，路上（两个静止点之间）
        // 一律归外层。判定都从偏移算：0 是歌词页，页高是评论区。
        //
        // 这不是洁癖——两屏的嵌套滚动都抢在指针前面注册监听，只要露出一点、
        // 指针又在它上面，就会把位移截走，外层这一屏从此停在中途，再也翻不完。
        let song_at_rest = song_page_at_rest(scrolled);
        // 评论列表这一帧听谁的。程序化运动插在最前面：外层正在飞的那几百毫秒里，
        // 「到没到位」每帧都在翻，不能让它决定列表的去留——否则滚动条会闪一下，
        // 列表也会自己跳回顶部。
        let list_mode = if self.scroll_motion.is_active() {
            ListMode::Held
        } else if expanded && reached_comments(scrolled, page_height) {
            ListMode::Owned
        } else {
            ListMode::Parked
        };
        let content_height = (height - SONG_CONTENT_TOP_PADDING).max(1.);
        let content_width = (f32::from(window.viewport_size().width) - SONG_CONTENT_PADDING * 2.)
            .min(SONG_CONTENT_MAX_WIDTH)
            .min(content_height * CONTENT_MAX_ASPECT)
            .max(1.);
        let side = (content_width * 0.4).min(VINYL_MAX_SIDE).floor().max(1.);
        let title = song
            .as_ref()
            .map(|song| song.name.clone())
            .unwrap_or_else(|| "暂无正在播放的歌曲".into());
        let artists = song
            .as_ref()
            .map(|song| {
                song.ar
                    .iter()
                    .filter_map(|artist| artist.name.as_deref())
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default();
        let album = song
            .as_ref()
            .and_then(|song| song.al.name.as_deref())
            .unwrap_or("未知专辑")
            .to_owned();
        let source = source_name(&self.library, &self.playback, cx);
        let duration = song
            .as_ref()
            .map(|song| format_duration(song.duration()))
            .unwrap_or_default();
        FrameData {
            page_height,
            height,
            content_width,
            side,
            summary_top,
            comment_progress,
            song_at_rest,
            list_mode,
            reveal,
            colors,
            gradient,
            title,
            artists,
            album,
            source,
            duration,
        }
    }

    fn render_summary(
        &self,
        frame: &FrameData,
        summary_vinyl: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new("album-song-summary")
            .aria_label("回到唱片")
            .debug_selector(|| "album-song-summary".into())
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                // 「回到唱片」：外层滑回歌词页，评论列表原地不动。
                this.return_to_song(cx);
            }))
            .max_w(px(frame.content_width * SUMMARY_WIDTH_RATIO))
            .h(px(58.))
            .pl(px(5.5))
            .pr(px(16.))
            .flex()
            .items_center()
            .gap_3()
            .rounded_full()
            .border_1()
            .border_color(white().alpha(0.1))
            .bg(white().alpha(0.04))
            .block_mouse_except_scroll()
            .child(summary_vinyl)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .line_height(relative(1.5))
                    .child(
                        div()
                            .debug_selector(|| "album-summary-title".into())
                            .min_w_0()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white())
                            .truncate()
                            .child(frame.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(frame.colors.muted_foreground)
                            .child("—"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "album-summary-artists".into())
                            .max_w(px(240.))
                            .text_size(px(13.))
                            .text_color(frame.colors.muted_foreground)
                            .truncate()
                            .child(frame.artists.clone()),
                    ),
            )
            .child(
                svg()
                    .path("icons/unfold.svg")
                    .size(px(16.))
                    .flex_none()
                    .text_color(white().alpha(0.6))
                    .with_transformation(Transformation::rotate(radians(std::f32::consts::PI))),
            )
            .into_any_element()
    }

    fn render_tab(&self, frame: &FrameData) -> AnyElement {
        match self.tab {
            SongTab::Lyrics => div()
                .flex_1()
                .min_h_0()
                .relative()
                // cached 是独立布局根，尺寸同时写在缓存 refinement 与子视图根。
                .child(
                    self.lyrics
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                )
                .into_any_element(),
            SongTab::Encyclopedia => div()
                .id("song-encyclopedia-content")
                .debug_selector(|| "song-encyclopedia-content".into())
                .flex_1()
                .min_h_0()
                .pt(px(32.))
                .flex()
                .flex_col()
                .gap_4()
                .text_size(px(14.))
                .text_color(frame.colors.muted_foreground)
                .children(
                    [
                        ("歌曲", frame.title.clone()),
                        ("歌手", frame.artists.clone()),
                        ("专辑", frame.album.clone()),
                        ("时长", frame.duration.clone()),
                    ]
                    .into_iter()
                    .map(|(label, value)| div().child(format!("{label}：{value}"))),
                )
                .into_any_element(),
            SongTab::Similar => div()
                .id("song-similar-content")
                .debug_selector(|| "song-similar-content".into())
                .flex_1()
                .min_h_0()
                .pt(px(80.))
                .text_color(frame.colors.muted_foreground)
                .child("相似推荐暂不可用")
                .into_any_element(),
        }
    }

    fn render_tab_bar(&self, frame: &FrameData, cx: &mut Context<Self>) -> AnyElement {
        let colors = frame.colors;
        div()
            .id("album-song-tabs")
            .debug_selector(|| "album-song-tabs".into())
            .mt(px(24.))
            .self_start()
            .flex_none()
            .h(px(32.))
            .p(px(3.))
            .flex()
            .items_center()
            .rounded_full()
            .bg(white().alpha(0.06))
            .text_size(px(14.))
            .children(
                [
                    (SongTab::Lyrics, "song-lyrics", "歌词"),
                    (SongTab::Encyclopedia, "song-encyclopedia", "百科"),
                    (SongTab::Similar, "song-similar", "相似推荐"),
                ]
                .into_iter()
                .map(|(tab, id, label)| {
                    let selected = self.tab == tab;
                    Button::new(id)
                        .debug_selector(move || id.into())
                        .selected(selected)
                        .aria_label(label)
                        .h(px(26.))
                        .px(px(9.))
                        .rounded_full()
                        .border_1()
                        .border_color(transparent_black())
                        .cursor_pointer()
                        .text_color(if selected {
                            white()
                        } else {
                            colors.muted_foreground
                        })
                        .when(selected, |button| button.bg(white().alpha(0.12)))
                        .focus_visible(|style| style.border_color(white().alpha(0.5)))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tab = tab;
                            cx.notify();
                        }))
                }),
            )
            .into_any_element()
    }

    fn render_song_info(
        &self,
        frame: &FrameData,
        tab_content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = frame.colors;
        div()
            .id("album-lyrics-region")
            .debug_selector(|| "album-lyrics-region".into())
            .w(relative(0.5))
            .h_full()
            .min_w_0()
            .pl(px(24.))
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(26.))
                    .line_height(px(34.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white())
                    .line_clamp(2)
                    .child(frame.title.clone()),
            )
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .gap_4()
                    .text_size(px(14.))
                    .text_color(colors.muted_foreground)
                    .child(
                        div()
                            .debug_selector(|| "album-metadata-album".into())
                            .min_w_0()
                            .truncate()
                            .child(format!("专辑：{}", frame.album)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "album-metadata-artists".into())
                            .min_w_0()
                            .truncate()
                            .child(format!("歌手：{}", frame.artists)),
                    )
                    .when_some(frame.source.clone(), |metadata, source| {
                        metadata.child(
                            div()
                                .debug_selector(|| "album-metadata-source".into())
                                .min_w_0()
                                .truncate()
                                .child(format!("来源：{source}")),
                        )
                    }),
            )
            .child(self.render_tab_bar(frame, cx))
            .child(tab_content)
            .into_any_element()
    }

    /// 收起整页的按钮。
    ///
    /// 放在页头左端，和右端那组图标同一行——页头只有一块，所以它不需要再单占一行。
    fn collapse_button(&self, frame: &FrameData, cx: &mut Context<Self>) -> Button {
        hover_icon(
            "close-album-lyrics",
            "收起专辑歌词页",
            "icons/unfold.svg",
            frame.colors,
        )
        .ml_0()
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            this.now_playing
                .update(cx, |now_playing, cx| now_playing.collapse(cx));
        }))
    }

    /// 页头。整块自带顶部留白（`pt`），所以只有一块：左端收起，右端播放器模式 + 图标组。
    fn page_header(
        &self,
        frame: &FrameData,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        window_drag_area("album-lyrics-header")
            .absolute()
            .top_0()
            .left_0()
            .h(px(HEADER_HEIGHT))
            .pt(px(HEADER_TOP_INSET))
            .w_full()
            .bg(transparent_black())
            .px(px(40.))
            .flex()
            .items_center()
            .child(self.collapse_button(frame, cx))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    // 文案常驻，只改透明度。整块右对齐，抽掉它会让右边那组按钮横向
                    // 跳一下——淡出把这一跳也一并省了。
                    .child(
                        div()
                            .text_color(frame.colors.muted_foreground)
                            .text_size(px(13.))
                            .opacity(1. - ramp(frame.comment_progress, HEADER_FADE))
                            .child("播放器模式"),
                    )
                    .children(mini_and_window_buttons(
                        "album-header-mini-button",
                        window,
                        frame.colors,
                    )),
            )
            .into_any_element()
    }
}

impl Render for NowPlayingPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = cx.background_executor().now();
        let expanded = self.now_playing.read(cx).is_expanded();
        // 程序化运动（「回到唱片」的补间 / 吸附的弹簧）先走一步：它写的就是这一帧要画的偏移。
        self.scroll_motion.step(&self.scroll, window, now);
        let page_height = (f32::from(window.viewport_size().height) - PLAYER_BAR_HEIGHT).max(1.);
        let previous_height = self.page_height.replace(page_height);
        if let Some(previous_height) = previous_height
            && previous_height != page_height
            // 运动在飞时不动偏移：目标就是歌词页静止点，窗口变了它照样对。
            && !self.scroll_motion.is_active()
            && reached_comments(-f32::from(self.scroll.offset().y), previous_height)
        {
            // 窗口变高时原生滚动不会自动补位：原来停在评论页静止点，
            // 就直接挪到新的评论页静止点。
            self.scroll.set_offset(point(px(0.), px(-page_height)));
        }
        // 手停了就把停在中间的这一屏送走。判定要发生在读偏移之前——吸附一旦
        // 起跑，这一帧的 list_mode 就得是 `Held`。
        self.settle_if_idle(expanded, page_height, now, window, cx);
        let frame = self.frame_data(window, cx);
        // Backdrop 在 prepaint 写入实际插值色；其下一帧通知让缓存 fade 重新绘制。
        let backdrop_gradient = self.backdrop.gradient();
        // 把共享底的最新插值广播给播放栏/进度条：它们据此重新取色，配色才能跟着
        // 封面色渐变一帧帧走（渐变由 gradient_layer 在 prepaint 写入，这里晚一帧读到）。
        self.now_playing
            .update(cx, |now_playing, cx| now_playing.publish_backdrop(cx));
        self.lyrics.update(cx, |view, cx| {
            view.configure_backdrop(backdrop_gradient, cx)
        });
        self.lyrics.update(cx, |view, cx| {
            // 只有停在歌词页时歌词才吃滚轮（并恢复跟随播放）；翻页路上松手，
            // 否则指针恰好压在歌词上就把这一屏卡在半路了。
            view.set_active(
                expanded && self.tab == SongTab::Lyrics && frame.song_at_rest,
                cx,
            )
        });
        self.comments.update(cx, |view, cx| {
            // 评论列表这一帧听谁的，见 `ListMode`。
            view.configure(frame.list_mode, cx)
        });
        self.vinyl.read(cx).clear();
        self.summary_vinyl.read(cx).clear();
        if !expanded && frame.reveal <= 0. {
            self.tonearm = Tonearm::default();
            return div().into_any_element();
        }
        let arm_angle = {
            let playback = self.playback.read(cx).snapshot();
            self.tonearm
                .angle(playback.is_playing, playback.revision, window, cx)
        };
        let vinyl = (frame.reveal > 0. && frame.summary_top > 0.).then(|| {
            self.vinyl
                .read(cx)
                .placeholder(frame.side, 1., Some(arm_angle))
                .debug_selector(|| "album-vinyl-image".into())
        });
        let summary_vinyl = if frame.summary_top < frame.page_height {
            self.summary_vinyl.read(cx).placeholder(45., 1., None)
        } else {
            div().size(px(45.)).flex_none()
        }
        .debug_selector(|| "album-summary-vinyl".into());
        let tab_content = self.render_tab(&frame);
        let song_info = self.render_song_info(&frame, tab_content, cx);
        let comments = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .max_w(px(1120.))
            .mx_auto()
            .child(
                self.comments
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            );
        let summary = self.render_summary(&frame, summary_vinyl, cx);
        let song_page = build_song_page(&frame, song_info, vinyl);
        // 悬浮按钮的淡入进度。它和页头文案用同一条连续量，只是区间不同：
        // 页头文案属于歌词页、先走，按钮属于评论区、后到。
        let publish = ramp(frame.comment_progress, PUBLISH_FADE);

        // 滑动层放在缓存根的子节点，百分比 top 才以全页为参照。
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(relative(1. - frame.reveal))
                    .size_full()
                    .flex()
                    .flex_col()
                    .bg(frame.colors.background)
                    .occlude()
                    .child(
                        gradient_layer(
                            "album-background-color",
                            frame.gradient,
                            self.backdrop.clone(),
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("now-playing-scroll")
                                    .debug_selector(|| "now-playing-scroll".into())
                                    .size_full()
                                    .pt(px(HEADER_HEIGHT))
                                    // 外层是普通平台滚动：两屏按文档流上下排布，滚到第二屏就是评论区。
                                    // 嵌套的歌词 / 评论列表自己决定什么时候拦位移，到边自然交回这里。
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    // 只采样滚轮事件、不拦位移：位移照旧由平台滚动和
                                    // 评论列表分。延迟吸附靠这份采样判手停了没有、
                                    // 以及最后是往哪推的。
                                    .on_scroll_wheel(cx.listener(
                                        |this, event: &ScrollWheelEvent, window, cx| {
                                            let now = cx.background_executor().now();
                                            this.wheel_watch.note(now, event, window.line_height());
                                        },
                                    ))
                                    .child(song_page)
                                    .child(
                                        div()
                                            .id("album-comments-page")
                                            .debug_selector(|| "album-comments-page".into())
                                            .w_full()
                                            .h(px(frame.page_height))
                                            .pt(px(HEADER_TOP_INSET))
                                            .flex()
                                            .flex_col()
                                            .overflow_hidden()
                                            .child(
                                                div()
                                                    .w_full()
                                                    .h(px(SUMMARY_HEIGHT))
                                                    .flex_none()
                                                    .px(px(40.))
                                                    .flex()
                                                    .justify_center()
                                                    .child(summary),
                                            )
                                            .child(comments),
                                    ),
                            )
                            // 悬浮按钮跟着评论区一起升上来。只在淡入真正开始之后才挂上去，
                            // 免得白付一次背景模糊；模糊半径跟着进度长，也就跟着一起淡了
                            // ——`paint_backdrop_blur` 不吃元素透明度，半径得自己缩。
                            .when(publish > 0., |content| {
                                content.child(
                                    div()
                                        .absolute()
                                        .bottom(px(16. + 10. * (1. - publish)))
                                        .left(relative(0.5))
                                        .ml(px(-76.))
                                        .opacity(publish)
                                        .child(
                                            div()
                                                .relative()
                                                .w(px(152.))
                                                .h(px(40.))
                                                .child(
                                                    canvas(
                                                        |_, _, _| (),
                                                        move |bounds, _, window, _| {
                                                            window.paint_backdrop_blur(
                                                                bounds,
                                                                px(12. * publish),
                                                                px(20.).into(),
                                                            );
                                                        },
                                                    )
                                                    .absolute()
                                                    .inset_0(),
                                                )
                                                .child(
                                                    Button::new("publish-song-comment")
                                                        .disabled(true)
                                                        .w(px(152.))
                                                        .h(px(40.))
                                                        .rounded_full()
                                                        .bg(linear_gradient(
                                                            180.,
                                                            linear_color_stop(
                                                                white().alpha(0.28),
                                                                0.,
                                                            ),
                                                            linear_color_stop(
                                                                white().alpha(0.14),
                                                                1.,
                                                            ),
                                                        ))
                                                        .border_1()
                                                        .border_color(white().alpha(0.10))
                                                        .text_color(white())
                                                        .child("发布评论"),
                                                ),
                                        ),
                                )
                            }),
                    )
                    .child(self.page_header(&frame, window, cx)),
            )
            .into_any_element()
    }
}

/// 这一屏还停在歌词页静止点没有。
fn song_page_at_rest(scrolled: f32) -> bool {
    scrolled <= SCROLL_END_TOLERANCE
}

/// 把进度映射到 `(起点, 终点)` 区间上的 0→1。
fn ramp(progress: f32, (from, to): (f32, f32)) -> f32 {
    ((progress - from) / (to - from)).clamp(0., 1.)
}

/// 外层这一屏翻完了没有。
///
/// 两屏的页高都等于窗口可视高度，内容总高正好是两倍页高，所以外层的滚动尽头
/// 就是「评论区顶对齐到视口顶」，也就是评论区整屏到位的那一刻。整页只有这里
/// 把滚轮交给评论列表，[`ListMode::Owned`] 和窗口变高时的补位都读它。
fn reached_comments(scrolled: f32, page_height: f32) -> bool {
    scrolled >= page_height - SCROLL_END_TOLERANCE
}

/// 吸附的落点：按「停在哪」和「最后往哪推」一起定。
///
/// 只按中点切会在一个很常见的手势上出错——慢慢往评论区推、推到四成停下，
/// 明明是奔着评论区去的，却被判回歌词页。所以停在中段时把判据按来向收紧 /
/// 放宽：推着往下停，四成就算数；推着往上停，得退到六成才回去。来向不明
/// （只有零星事件）才退回中点。
fn settle_target(scrolled: f32, page_height: f32, direction: f32) -> Pixels {
    let (push_down, push_up) = SETTLE_COMMIT;
    let commit = if direction > 0. {
        push_down
    } else if direction < 0. {
        push_up
    } else {
        0.5
    };
    if scrolled / page_height >= commit {
        px(-page_height)
    } else {
        px(0.)
    }
}

fn source_name(
    library: &Entity<MusicLibrary>,
    playback: &Entity<PlaybackController>,
    cx: &App,
) -> Option<String> {
    let id = playback.read(cx).playlist_id()?;
    library
        .read(cx)
        .playlists
        .iter()
        .find(|playlist| playlist.id == id)
        .map(|playlist| playlist.name.clone())
}

fn build_song_page(frame: &FrameData, song_info: AnyElement, vinyl: Option<Div>) -> AnyElement {
    div()
        .h(px(frame.height))
        .w_full()
        .px(px(SONG_CONTENT_PADDING))
        .pt(px(SONG_CONTENT_TOP_PADDING))
        .flex()
        .justify_center()
        .child(
            div()
                .debug_selector(|| "album-song-content".into())
                .w(px(frame.content_width))
                .h_full()
                .flex()
                .child(artwork(vinyl))
                .child(song_info),
        )
        .into_any_element()
}

fn artwork(disc: Option<Div>) -> impl IntoElement {
    div()
        .id("album-artwork-region")
        .debug_selector(|| "album-artwork-region".into())
        .w(relative(0.5))
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .justify_start()
        .children(disc)
}
