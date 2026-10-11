use std::{sync::Arc, time::Duration};

use gpui::{prelude::*, *};
use gpui_kit::base::{Button, Theme};
use ncm_api_rs::create_client;
use qrcode::{QrCode, render::svg};

use crate::{
    api::MusicApi,
    auth::{self, QrStatus},
    state::account::AccountState,
};

pub(super) fn open(
    parent: &Window,
    account: Entity<AccountState>,
    existing: Option<AnyWindowHandle>,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, cx| {
                cx.activate(true);
                window.activate_window();
            })
            .is_ok()
        {
            return Some(handle);
        }
    }
    let bounds = Bounds::centered_at(
        parent.window_bounds().get_bounds().center(),
        size(px(360.), px(440.)),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: parent.display(cx).map(|display| display.id()),
        titlebar: Some(TitlebarOptions {
            appears_transparent: true,
            ..Default::default()
        }),
        app_owns_titlebar_drag: true,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| LoginWindow::new(account, window, cx))
    }) {
        Ok((handle, _)) => Some(handle),
        Err(error) => {
            eprintln!("打开登录窗口失败：{error}");
            None
        }
    }
}

pub(super) struct LoginWindow {
    account: Entity<AccountState>,
    qr: Option<Arc<Image>>,
    message: String,
    // 丢弃任务即可停止轮询，刷新和关闭窗口不会留下后台循环。
    task: Option<Task<()>>,
    _subscription: Subscription,
}

impl LoginWindow {
    fn new(account: Entity<AccountState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe_in(&account, window, |_, _, window, cx| {
            window.refresh();
            cx.notify();
        });
        let mut this = Self {
            account,
            qr: None,
            message: "点击刷新二维码开始登录".into(),
            task: None,
            _subscription: subscription,
        };
        if this.account.read(cx).profile.user_id == 0 && !this.account.read(cx).loading {
            this.start(window, cx);
        }
        this
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.account.read(cx).loading {
            return;
        }
        self.account.update(cx, |account, cx| {
            account.error = None;
            cx.notify();
        });
        self.task = None;
        self.qr = None;
        self.message = "正在获取二维码…".into();
        let client = create_client(None);
        let request = MusicApi::global(cx).runtime.spawn({
            let client = client.clone();
            async move { auth::qr_key(&client).await }
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result: Result<(), String> = async {
                let key = request.await.map_err(|_| "二维码请求任务失败")??;
                let url = format!("https://music.163.com/login?codekey={key}");
                let code = QrCode::new(url.as_bytes()).map_err(|_| "生成二维码失败")?;
                let svg = code.render::<svg::Color>().min_dimensions(240, 240).build();
                this.update_in(cx, |view, window, cx| {
                    view.qr = Some(Arc::new(Image::from_bytes(
                        ImageFormat::Svg,
                        svg.into_bytes(),
                    )));
                    view.message = "请用网易云音乐 App 扫码".into();
                    window.refresh();
                    cx.notify();
                })
                .map_err(|_| "登录窗口已关闭")?;
                loop {
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    let request = cx
                        .update(|_, cx| {
                            MusicApi::global(cx).runtime.spawn({
                                let client = client.clone();
                                let key = key.clone();
                                async move { auth::qr_check(&client, &key).await }
                            })
                        })
                        .map_err(|_| "登录窗口已关闭")?;
                    let status = request.await.map_err(|_| "扫码查询任务失败")??;
                    match status {
                        QrStatus::Authorized(cookie) => {
                            let request = cx
                                .update(|_, cx| {
                                    MusicApi::global(cx).runtime.spawn(MusicApi::user_profile(
                                        create_client(Some(cookie.clone())),
                                    ))
                                })
                                .map_err(|_| "登录窗口已关闭")?;
                            let profile = request.await.map_err(|_| "账号验证任务失败")??;
                            this.update_in(cx, |view, window, cx| {
                                view.qr = None;
                                view.message = "正在保存登录状态…".into();
                                view.account
                                    .update(cx, |account, cx| account.sign_in(cookie, profile, cx));
                                window.refresh();
                                cx.notify();
                            })
                            .map_err(|_| "登录窗口已关闭")?;
                            return Ok(());
                        }
                        QrStatus::Expired => return Err("二维码已过期，请刷新".into()),
                        QrStatus::Waiting | QrStatus::Scanned => {
                            this.update_in(cx, |view, window, cx| {
                                view.message = if matches!(status, QrStatus::Scanned) {
                                    "已扫码，请在手机上确认登录"
                                } else {
                                    "请用网易云音乐 App 扫码"
                                }
                                .into();
                                window.refresh();
                                cx.notify();
                            })
                            .map_err(|_| "登录窗口已关闭")?;
                        }
                    }
                }
            }
            .await;
            if let Err(message) = result {
                let _ = this.update_in(cx, |view, window, cx| {
                    view.qr = None;
                    view.message = message;
                    window.refresh();
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }
}

impl Render for LoginWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let account = self.account.read(cx);
        let logged_in = account.profile.user_id != 0;
        let busy = account.loading;
        let nickname = account.profile.nickname.clone();
        let error = account.error.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(
                super::components::window_drag_area("login-titlebar")
                    .relative()
                    .flex_none()
                    .h(px(64.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if logged_in {
                        "我的账号"
                    } else {
                        "扫码登录"
                    })
                    .when(!cfg!(target_os = "macos"), |bar| {
                        bar.child(
                            Button::new("login-close")
                                .absolute()
                                .right(px(12.))
                                .aria_label("关闭登录窗口")
                                .child(svg().path("icons/window/close.svg").size(px(16.)))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(|_, window, _| window.remove_window()),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .px_6()
                    .when(logged_in, |body| {
                        body.child(nickname).child("已登录网易云音乐")
                    })
                    .when(!logged_in, |body| {
                        body.child(
                            div()
                                .size(px(240.))
                                .bg(rgb(0xffffff))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when_some(self.qr.clone(), |area, qr| {
                                    area.child(img(qr).size(px(240.)))
                                })
                                .when(self.qr.is_none(), |area| {
                                    area.child(div().text_color(rgb(0x555555)).child(if busy {
                                        "正在处理…"
                                    } else {
                                        "二维码待刷新"
                                    }))
                                }),
                        )
                        .child(if busy {
                            "正在处理登录状态…".to_owned()
                        } else {
                            error.clone().unwrap_or_else(|| self.message.clone())
                        })
                    })
                    .when_some(error.filter(|_| logged_in), |body, error| {
                        body.child(div().text_sm().child(error))
                    })
                    .when(!logged_in, |body| {
                        body.child(
                            Button::new("login-refresh")
                                .disabled(busy)
                                .child("刷新二维码")
                                .on_click(
                                    cx.listener(|view, _, window, cx| view.start(window, cx)),
                                ),
                        )
                    })
                    .when(logged_in, |body| {
                        body.child(
                            Button::new("account-logout")
                                .disabled(busy)
                                .child(if busy {
                                    "正在退出…"
                                } else {
                                    "退出登录"
                                })
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.task = None;
                                    view.qr = None;
                                    view.message = "已退出，请刷新二维码登录".into();
                                    view.account.update(cx, |account, cx| account.logout(cx));
                                })),
                        )
                    }),
            )
    }
}
