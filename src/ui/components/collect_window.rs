//! 单实例收藏弹窗；音乐库负责请求，窗口订阅结果并在成功后关闭。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::input::{Input, InputEditorStyle, InputEvent, InputState};
use gpui_kit::base::{ColorTokens, Root, Scrollbar, ScrollbarMode, Theme};

use super::hide_traffic_lights;
use crate::desktop;
use crate::models::Song;
use crate::state::library::{MusicLibrary, SongsCollected};
use crate::ui::assets::thumbnail_url;
use crate::ui::theme::PRESSED_ICON_ALPHA;

const WINDOW_SIZE: Size<Pixels> = size(px(441.), px(528.));
const TITLE_TOP_GAP: Pixels = px(27.);
const TITLE_BOTTOM_GAP: Pixels = px(18.);
const ROW_PADDING_X: Pixels = px(26.);
const ROW_PADDING_Y: Pixels = px(10.);
const COVER_SIZE: Pixels = px(46.);
const COVER_RADIUS: Pixels = px(8.);
const COVER_GAP: Pixels = px(18.);

/// 使用调用方窗口定位，避免在点击回调中再次 update 同一个窗口。
pub fn open_collect_window(parent: &Window, songs: Vec<Song>, cx: &mut App) {
    if songs.is_empty() {
        return;
    }
    if let Some(handle) = desktop::collect_window(cx) {
        let replacement = songs.clone();
        let focused = handle
            .update(cx, |view, window, cx| {
                cx.activate(true);
                window.activate_window();
                // gpui-kit 在窗口内容外包了一层 Root。
                let Ok(root) = view.downcast::<Root>() else {
                    return;
                };
                let content = root.read(cx).view().clone();
                if let Ok(view) = content.downcast::<CollectWindow>() {
                    view.update(cx, |this, cx| this.set_songs(replacement, cx));
                }
            })
            .is_ok();
        if focused {
            return;
        }
        desktop::set_collect_window(None, cx);
    }

    let parent_bounds = parent.window_bounds().get_bounds();
    let bounds = Bounds::centered_at(parent_bounds.center(), WINDOW_SIZE);
    // 多显示器下必须同时传入父窗口的显示器，避免落到主屏。
    let display_id = parent.display(cx).map(|display| display.id());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        titlebar: Some(TitlebarOptions {
            appears_transparent: true,
            ..Default::default()
        }),
        app_owns_titlebar_drag: true,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let Some(library) = desktop::library(cx) else {
        log::warn!("音乐库尚未就绪，无法打开收藏窗口");
        return;
    };
    match gpui_kit::open_window(options, cx, move |window, cx| {
        #[cfg(target_os = "macos")]
        hide_traffic_lights(window, true);
        window.on_window_should_close(cx, |_, cx| {
            desktop::set_collect_window(None, cx);
            true
        });
        cx.new(|cx| CollectWindow::new(library, songs, window, cx))
    }) {
        Ok((window, _)) => desktop::set_collect_window(Some(window), cx),
        Err(error) => log::warn!("打开收藏窗口失败：{error}"),
    }
}

struct CollectWindow {
    library: Entity<MusicLibrary>,
    songs: Vec<Song>,
    scroll: ScrollHandle,
    name_input: Entity<InputState>,
    creating: bool,
    submitting: bool,
    _library_subscription: Subscription,
    _collect_subscription: Subscription,
    _input_subscription: Subscription,
}

impl CollectWindow {
    fn new(
        library: Entity<MusicLibrary>,
        songs: Vec<Song>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("歌单名");
            state.set_editor_style(InputEditorStyle {
                selection: Theme::global(cx).tokens.colors.selection,
                ..Default::default()
            });
            state
        });
        let subscription = cx.observe(&library, |_, _, cx| cx.notify());
        let collect_subscription = cx.subscribe_in(
            &library,
            window,
            |this, _, event: &SongsCollected, window, cx| this.collected(event, window, cx),
        );
        let input_subscription =
            cx.subscribe_in(&name_input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.create_and_collect(cx);
                }
            });
        Self {
            library,
            songs,
            scroll: ScrollHandle::default(),
            name_input,
            creating: false,
            submitting: false,
            _library_subscription: subscription,
            _collect_subscription: collect_subscription,
            _input_subscription: input_subscription,
        }
    }

    fn set_songs(&mut self, songs: Vec<Song>, cx: &mut Context<Self>) {
        // 请求提交后保留原目标，避免结果对应到另一批歌曲。
        if self.submitting || songs.is_empty() {
            return;
        }
        self.songs = songs;
        self.creating = false;
        cx.notify();
    }

    fn collect(&mut self, playlist_id: u64, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        self.submitting = true;
        let songs = self.songs.clone();
        self.library.update(cx, |library, cx| {
            library.collect_songs(playlist_id, songs, cx)
        });
        cx.notify();
    }

    fn start_creating(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        self.creating = true;
        self.name_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn cancel_creating(&mut self, cx: &mut Context<Self>) {
        self.creating = false;
        cx.notify();
    }

    fn create_and_collect(&mut self, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let name = self.name_input.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return;
        }
        self.submitting = true;
        let songs = self.songs.clone();
        self.library.update(cx, |library, cx| {
            library.create_playlist_and_collect(name, songs, cx)
        });
        cx.notify();
    }

    // shortcut: 结果未关联窗口，关闭重开时可能收到旧结果；支持请求隔离时需增加关联标识。
    fn collected(&mut self, event: &SongsCollected, window: &mut Window, cx: &mut Context<Self>) {
        self.submitting = false;
        match &event.result {
            Ok(()) => {
                self.creating = false;
                self.close(window, cx);
            }
            Err(message) => log::warn!("收藏失败：{message}"),
        }
        cx.notify();
    }

    // 延后关窗，避免在音乐库事件广播中打断订阅链。
    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |_, cx| {
            let _ = cx.update(|window, cx| {
                desktop::set_collect_window(None, cx);
                window.remove_window();
            });
        })
        .detach();
    }

    fn playlist_rows(&self, colors: ColorTokens, cx: &Context<Self>) -> Vec<AnyElement> {
        let library = self.library.read(cx);
        let mut rows = Vec::with_capacity(library.playlists().len() + 1);
        if self.creating {
            rows.push(self.create_row(colors, cx));
        } else {
            rows.push(playlist_row(
                ("collect-window-new", 0),
                create_playlist_cover(colors),
                "创建新歌单".to_owned(),
                None,
                colors,
                cx.listener(|this, _, window, cx| this.start_creating(window, cx)),
            ));
        }
        let playlists = library.favorite_playlist().into_iter().chain(
            library
                .playlists()
                .iter()
                .filter(|playlist| playlist.special_type != 5),
        );
        rows.extend(playlists.map(|playlist| {
            let id = playlist.id;
            playlist_row(
                ("collect-window-playlist", id),
                playlist_cover(playlist.cover_img_url.as_deref(), colors),
                playlist.name.clone(),
                Some(format!("{}首音乐", playlist.track_count)),
                colors,
                cx.listener(move |this, _, _, cx| this.collect(id, cx)),
            )
        }));
        rows
    }

    fn create_row(&self, colors: ColorTokens, cx: &Context<Self>) -> AnyElement {
        div()
            .id("collect-window-create")
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .py(ROW_PADDING_Y)
            .px(ROW_PADDING_X)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h(COVER_SIZE)
                    .flex()
                    .items_center()
                    .px(px(12.))
                    .rounded(COVER_RADIUS)
                    .border_1()
                    .border_color(colors.border)
                    .text_size(px(14.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(Input::new(&self.name_input)),
            )
            .child(text_button(
                "collect-window-create-cancel",
                "取消",
                colors,
                cx.listener(|this, _, _, cx| this.cancel_creating(cx)),
            ))
            .child(text_button(
                "collect-window-create-confirm",
                "创建",
                colors,
                cx.listener(|this, _, _, cx| this.create_and_collect(cx)),
            ))
            .into_any_element()
    }
}

impl Render for CollectWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let rows = self.playlist_rows(colors, cx);
        let title = if self.songs.len() > 1 {
            format!("收藏 {} 首到歌单", self.songs.len())
        } else {
            "收藏到歌单".to_owned()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.foreground)
            .overflow_hidden()
            .child(
                super::window_drag_area("collect-window-titlebar")
                    .relative()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pt(TITLE_TOP_GAP)
                    .pb(TITLE_BOTTOM_GAP)
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(!cfg!(target_os = "macos"), |bar| {
                        bar.child(close_button(colors))
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("collect-window-playlists")
                            .flex_1()
                            .min_h(px(0.))
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            // 横向手势不改写成竖向：一次滑动只认一个方向。
                            .restrict_scroll_to_axis()
                            .track_scroll(&self.scroll)
                            .children(rows),
                    )
                    .child(Scrollbar::vertical(&self.scroll).mode(ScrollbarMode::Always)),
            )
    }
}

fn text_button(
    id: &'static str,
    label: &'static str,
    colors: ColorTokens,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(colors.muted_foreground)
        .hover(|style| style.text_color(colors.foreground))
        .on_click(on_click)
        .child(label)
}

fn playlist_row(
    id: (&'static str, u64),
    cover: AnyElement,
    title: String,
    subtitle: Option<String>,
    colors: ColorTokens,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(COVER_GAP)
        .py(ROW_PADDING_Y)
        .px(ROW_PADDING_X)
        .cursor_pointer()
        .role(Role::Button)
        .aria_label(title.clone())
        .hover(|style| style.bg(colors.muted))
        .on_click(on_click)
        .child(cover)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().truncate().text_size(px(16.)).child(title))
                .when_some(subtitle, |column, subtitle| {
                    column.child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(colors.muted_foreground)
                            .child(subtitle),
                    )
                }),
        )
        .into_any_element()
}

fn create_playlist_cover(colors: ColorTokens) -> AnyElement {
    div()
        .size(COVER_SIZE)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(COVER_RADIUS)
        .bg(colors.muted)
        .child(
            svg()
                .path("icons/加号.svg")
                .size(px(20.))
                .flex_none()
                .text_color(colors.muted_foreground),
        )
        .into_any_element()
}

fn playlist_cover(url: Option<&str>, colors: ColorTokens) -> AnyElement {
    let source = url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|url| thumbnail_url(url, 92));
    div()
        .size(COVER_SIZE)
        .flex_none()
        .rounded(COVER_RADIUS)
        .overflow_hidden()
        .bg(colors.muted)
        .when_some(source, |cover, source| {
            cover.child(
                // `overflow_hidden` 只按矩形裁剪，圆角要直接设在图片上。
                img(source)
                    .size_full()
                    .rounded(COVER_RADIUS)
                    .object_fit(ObjectFit::Cover),
            )
        })
        .into_any_element()
}

fn close_button(colors: ColorTokens) -> impl IntoElement {
    svg()
        .path("icons/window/close.svg")
        .size(px(20.))
        .absolute()
        .top(px(24.))
        .right(px(20.))
        .flex_none()
        .cursor_pointer()
        .text_color(colors.foreground.alpha(0.6))
        .hover(|style| style.text_color(colors.foreground))
        .id("collect-window-close")
        .role(Role::Button)
        .aria_label("关闭")
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .active(|style| style.text_color(colors.foreground.alpha(PRESSED_ICON_ALPHA)))
        .on_click(|_, window, cx| {
            desktop::set_collect_window(None, cx);
            window.remove_window();
        })
}
