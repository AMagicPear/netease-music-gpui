use std::{sync::Arc, time::Duration};

use gpui::{prelude::*, *};
use gpui_kit::base::input::{Input, InputEditorStyle, InputState};
use gpui_kit::base::{Button, ColorTokens, Theme};
use gpui_kit::component::checkbox::Checkbox;
use ncm_api_rs::create_client;
use qrcode::{QrCode, render::svg};

use super::components::{TabBar, TabChanged, TabItem};
use crate::{
    api::MusicApi,
    auth::{self, PhoneSecret, QrStatus},
    state::account::AccountState,
};

/// 三份协议的链接，条款文案在网易云官网维护。
const TERMS: [(&str, &str); 3] = [
    (
        "《服务条款》",
        "https://st.music.163.com/official-terms/service",
    ),
    (
        "《隐私政策》",
        "https://st.music.163.com/official-terms/privacy",
    ),
    (
        "《儿童隐私政策》",
        "https://st.music.163.com/official-terms/children",
    ),
];
const CAPTCHA_COOLDOWN_SECS: u32 = 60;

pub(super) fn open(
    parent: &Window,
    account: Entity<AccountState>,
    existing: Option<AnyWindowHandle>,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    if let Some(handle) = existing
        && handle
            .update(cx, |_, window, cx| {
                cx.activate(true);
                window.activate_window();
            })
            .is_ok()
    {
        return Some(handle);
    }
    let bounds = Bounds::centered_at(
        parent.window_bounds().get_bounds().center(),
        size(px(760.), px(480.)),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: parent.display(cx).map(|display| display.id()),
        titlebar: Some(TitlebarOptions {
            appears_transparent: true,
            ..Default::default()
        }),
        app_owns_titlebar_drag: true,
        // 允许拖拽缩放：双栏内容弹性自适应，仅设置下限防止拖坏布局。
        is_resizable: true,
        is_minimizable: true,
        window_min_size: Some(size(px(680.), px(440.))),
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
    // —— 手机号登录 ——
    /// 0 = 验证码登录，1 = 密码登录。
    tabs: Entity<TabBar>,
    _tabs_subscription: Subscription,
    phone_input: Entity<InputState>,
    secret_input: Entity<InputState>,
    agreed: bool,
    /// 验证码按钮倒计时；>0 时禁用并显示剩余秒数。
    cooldown: u32,
    captcha_task: Option<Task<()>>,
    login_task: Option<Task<()>>,
    /// 手机号登录专用的提示 / 错误，与扫码侧的 message 分开。
    phone_message: String,
    logging_in: bool,
}

impl LoginWindow {
    fn new(account: Entity<AccountState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 隐藏红绿灯里的绿灯（最大化按钮）：登录窗允许拖拽缩放，但不应该被一拉满。
        // gpui 的 WindowOptions 没有这个开关，只能对原生 NSWindow 设 style mask。
        #[cfg(target_os = "macos")]
        super::components::hide_traffic_lights(window, false);
        let subscription = cx.observe_in(&account, window, |this, account, window, cx| {
            // 登录成功即关窗：这个窗口只承担“登录”职责，成功后没有存在的必要。
            if account.read(cx).profile.user_id != 0 {
                window.remove_window();
                return;
            }
            window.refresh();
            cx.notify();
            let _ = this;
        });
        let tabs =
            cx.new(|_| TabBar::new(vec![TabItem::new("验证码登录"), TabItem::new("密码登录")]));
        let phone_input = Self::input("请输入手机号", window, cx);
        let secret_input = Self::input("请输入短信验证码", window, cx);
        let tabs_subscription = cx.subscribe_in(&tabs, window, {
            let secret_input = secret_input.clone();
            move |_, tabs, _: &TabChanged, window, cx| {
                // 切换登录方式时同步凭据输入框的占位文案。
                let using_captcha = tabs.read(cx).selected_index() == 0;
                secret_input.update(cx, |state, cx| {
                    state.set_placeholder(
                        if using_captcha {
                            "请输入短信验证码"
                        } else {
                            "请输入密码"
                        },
                        window,
                        cx,
                    );
                });
                cx.notify();
            }
        });
        let mut this = Self {
            account,
            qr: None,
            message: "点击刷新二维码开始登录".into(),
            task: None,
            _subscription: subscription,
            tabs,
            _tabs_subscription: tabs_subscription,
            phone_input,
            secret_input,
            agreed: false,
            cooldown: 0,
            captcha_task: None,
            login_task: None,
            phone_message: String::new(),
            logging_in: false,
        };
        if this.account.read(cx).profile.user_id != 0 {
            // 防御：窗口在已登录状态下被打开（不应发生，shell 会拦）。
            // 直接排一个关窗任务，避免渲染出无意义的空窗口。
            cx.spawn_in(window, async move |_, cx| {
                let _ = cx.update(|window, _| window.remove_window());
            })
            .detach();
            return this;
        }
        this.start(window, cx);
        this
    }

    fn input(placeholder: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(placeholder);
            state.set_editor_style(InputEditorStyle {
                selection: Theme::global(cx).tokens.colors.selection,
                ..Default::default()
            });
            state
        })
    }

    fn using_captcha(&self, cx: &App) -> bool {
        self.tabs.read(cx).selected_index() == 0
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
        // 二维码过期后自动重新获取，无需手动刷新；网络故障时
        // 显示错误并在 3s 后重试，直到登录成功或窗口关闭。
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let step: Result<bool, String> = async {
                    let request = cx
                        .update(|_, cx| {
                            MusicApi::global(cx).runtime.spawn({
                                let client = client.clone();
                                async move { auth::qr_key(&client).await }
                            })
                        })
                        .map_err(|_| "登录窗口已关闭")?;
                    let key = request
                        .await
                        .map_err(|_| "二维码请求任务失败".to_string())??;
                    let url = format!("https://music.163.com/login?codekey={key}");
                    let code = QrCode::new(url.as_bytes()).map_err(|_| "生成二维码失败")?;
                    // light_color 设为全透明：SVG 默认会铺一层不透明白色衬底。
                    let svg = code
                        .render::<svg::Color>()
                        .dark_color(svg::Color("#283248"))
                        .light_color(svg::Color("rgba(0,0,0,0)"))
                        .min_dimensions(240, 240)
                        .build();
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
                        let status = request
                            .await
                            .map_err(|_| "扫码状态查询失败，正在重试".to_string())??;
                        match status {
                            QrStatus::Authorized(cookie) => {
                                Self::finish_login(&this, cookie, cx).await?;
                                return Ok(false);
                            }
                            // 外层返回 true 让循环重新取 key 换新二维码。
                            QrStatus::Expired => {
                                this.update_in(cx, |view, window, cx| {
                                    view.message = "二维码已过期，正在刷新…".into();
                                    window.refresh();
                                    cx.notify();
                                })
                                .map_err(|_| "登录窗口已关闭")?;
                                return Ok(true);
                            }
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
                match step {
                    Ok(false) => return,
                    Ok(true) => {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                    }
                    Err(message) => {
                        let _ = this.update_in(cx, |view, window, cx| {
                            view.message = message;
                            window.refresh();
                            cx.notify();
                        });
                        cx.background_executor().timer(Duration::from_secs(3)).await;
                    }
                }
            }
        }));
        cx.notify();
    }

    /// 凭据到手后的公共收尾：拉取用户资料并写入账号状态。
    async fn finish_login(
        this: &WeakEntity<Self>,
        cookie: String,
        cx: &mut AsyncWindowContext,
    ) -> Result<(), String> {
        let request = cx
            .update(|_, cx| {
                MusicApi::global(cx)
                    .runtime
                    .spawn(MusicApi::user_profile(create_client(Some(cookie.clone()))))
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
        Ok(())
    }

    /// 发送短信验证码，成功后进入 60s 倒计时。
    fn send_captcha(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cooldown > 0 {
            return;
        }
        let phone = self.phone_input.read(cx).value().trim().to_owned();
        if !valid_phone(&phone) {
            self.phone_message = "请输入正确的手机号".into();
            cx.notify();
            return;
        }
        self.cooldown = CAPTCHA_COOLDOWN_SECS;
        self.phone_message = "验证码发送中…".into();
        cx.notify();
        let client = create_client(None);
        let request = MusicApi::global(cx).runtime.spawn({
            let client = client.clone();
            let phone = phone.clone();
            async move { auth::send_captcha(&client, &phone).await }
        });
        self.captcha_task = Some(cx.spawn_in(window, async move |this, cx| {
            let sent: Result<(), String> = request
                .await
                .map_err(|_| "验证码请求任务失败".to_string())
                .and_then(std::convert::identity);
            // 倒计时无论成败都照走：失败也给足冷却，避免连点刷接口。
            let _ = this.update_in(cx, |view, window, cx| {
                view.phone_message = match sent {
                    Ok(()) => "验证码已发送，请查收短信".into(),
                    Err(ref message) => message.clone(),
                };
                window.refresh();
                cx.notify();
            });
            for _ in 0..CAPTCHA_COOLDOWN_SECS {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update_in(cx, |view, window, cx| {
                    view.cooldown = view.cooldown.saturating_sub(1);
                    window.refresh();
                    cx.notify();
                    view.cooldown > 0
                });
                if alive.map_or(true, |alive| !alive) {
                    break;
                }
            }
        }));
    }

    /// 手机号 + 验证码 / 密码登录。
    fn phone_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.logging_in {
            return;
        }
        let using_captcha = self.using_captcha(cx);
        let phone = self.phone_input.read(cx).value().trim().to_owned();
        let secret = self.secret_input.read(cx).value().trim().to_owned();
        if !valid_phone(&phone) {
            self.phone_message = "请输入正确的手机号".into();
            cx.notify();
            return;
        }
        if secret.is_empty() {
            self.phone_message = if using_captcha {
                "请输入短信验证码"
            } else {
                "请输入密码"
            }
            .into();
            cx.notify();
            return;
        }
        let secret = if using_captcha {
            PhoneSecret::Captcha(secret)
        } else {
            PhoneSecret::Password(secret)
        };
        self.logging_in = true;
        self.phone_message = "正在登录…".into();
        cx.notify();
        let client = create_client(None);
        let request = MusicApi::global(cx)
            .runtime
            .spawn(async move { auth::phone_login(&client, &phone, secret).await });
        self.login_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result: Result<(), String> = async {
                let cookie = request.await.map_err(|_| "登录请求任务失败")??;
                Self::finish_login(&this, cookie, cx).await
            }
            .await;
            if let Err(message) = result {
                let _ = this.update_in(cx, |view, window, cx| {
                    view.logging_in = false;
                    view.phone_message = message;
                    window.refresh();
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }

    fn qr_pane(&self, colors: ColorTokens, busy: bool) -> Div {
        div()
            .w_1_2()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .child(
                // 二维码 SVG 自带透明背景，直接画在窗口底色上，不再垫白底。
                div()
                    .size(px(200.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when_some(self.qr.clone(), |area, qr| {
                        area.child(img(qr).size(px(200.)))
                    })
                    .when(self.qr.is_none(), |area| {
                        area.child(div().text_sm().text_color(colors.muted_foreground).child(
                            if busy {
                                "正在处理…"
                            } else {
                                "正在生成二维码…"
                            },
                        ))
                    }),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.muted_foreground)
                    .child(if busy {
                        "正在处理登录状态…".to_owned()
                    } else {
                        self.message.clone()
                    }),
            )
    }

    /// 右栏：手机号验证码 / 密码登录。
    fn phone_pane(&self, colors: ColorTokens, cx: &mut Context<Self>) -> Div {
        let using_captcha = self.using_captcha(cx);
        // Checkbox 的回调只有 App 上下文，用 Entity 句柄回写状态。
        let view = cx.entity();
        let logging_in = self.logging_in;
        div()
            .w_1_2()
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(18.))
            .px(px(36.))
            .py(px(24.))
            .child(self.tabs.clone())
            .child(field("login-phone", &self.phone_input, &colors))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(field("login-secret", &self.secret_input, &colors).flex_1())
                    .when(using_captcha, |row| {
                        row.child(
                            Button::new("send-captcha")
                                .disabled(self.cooldown > 0)
                                .text_size(px(12.))
                                .child(if self.cooldown > 0 {
                                    format!("{}s后重发", self.cooldown).into_any_element()
                                } else {
                                    "获取验证码".into_any_element()
                                })
                                .flex_none()
                                .on_click(
                                    cx.listener(|view, _, window, cx| {
                                        view.send_captcha(window, cx)
                                    }),
                                ),
                        )
                    }),
            )
            .child(
                // 协议勾选行：勾选后登录按钮才可用，书名号是可点的链接。
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(colors.muted_foreground)
                    .child(Checkbox::new("agree-terms").checked(self.agreed).on_click(
                        move |checked, _, cx| {
                            view.update(cx, |view, cx| {
                                view.agreed = *checked;
                                cx.notify();
                            });
                        },
                    ))
                    .child("同意")
                    .child(term_link("terms-service", TERMS[0].0, TERMS[0].1, &colors))
                    .child(term_link("terms-privacy", TERMS[1].0, TERMS[1].1, &colors))
                    .child(term_link("terms-children", TERMS[2].0, TERMS[2].1, &colors)),
            )
            .child(
                Button::new("phone-login")
                    .disabled(!self.agreed || logging_in)
                    .w_full()
                    .h(px(36.))
                    .rounded(px(8.))
                    .bg(colors.primary)
                    .text_color(colors.primary_foreground)
                    .hover(|style| style.bg(colors.primary.opacity(0.9)))
                    .active(|style| style.bg(colors.primary.opacity(0.8)))
                    .child(if logging_in {
                        "正在登录…"
                    } else {
                        "登录"
                    })
                    .on_click(cx.listener(|view, _, window, cx| view.phone_login(window, cx))),
            )
            .when_some(
                (!self.phone_message.is_empty()).then(|| self.phone_message.clone()),
                |pane, message| {
                    pane.child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.muted_foreground)
                            .child(message),
                    )
                },
            )
    }
}

fn field(id: &'static str, input: &Entity<InputState>, colors: &ColorTokens) -> Stateful<Div> {
    div()
        .id(id)
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .h(px(40.))
        .w_full()
        .flex()
        .items_center()
        .px_2()
        .border_1()
        .border_color(colors.border)
        .rounded(px(8.))
        .text_size(px(13.))
        .child(Input::new(input))
}

fn valid_phone(phone: &str) -> bool {
    phone.len() == 11 && phone.bytes().all(|byte| byte.is_ascii_digit())
}

fn term_link(
    id: &'static str,
    label: &'static str,
    url: &'static str,
    colors: &ColorTokens,
) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .text_color(colors.muted_foreground)
        .hover(|style| style.text_color(colors.foreground))
        .on_click(move |_, _, cx| cx.open_url(url))
        .child(label)
}

impl Render for LoginWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let busy = self.account.read(cx).loading;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(
                // 不显示大标题，只保留一条能拖动窗口的窄条（macOS 红绿灯也挂在这）。
                super::components::window_drag_area("login-titlebar")
                    .relative()
                    .flex_none()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!cfg!(target_os = "macos"), |bar| {
                        bar.child(
                            Button::new("login-close")
                                .absolute()
                                .right(px(12.))
                                .aria_label("关闭登录窗口")
                                .child(
                                    svg()
                                        .path("icons/window/close.svg")
                                        .size(px(20.))
                                        .text_color(colors.foreground.alpha(0.6))
                                        .hover(|style| style.text_color(colors.foreground)),
                                )
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(|_, window, _| window.remove_window()),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .child(self.qr_pane(colors, busy))
                    .child(div().w(px(1.)).flex_none().my(px(24.)).bg(colors.border))
                    .child(self.phone_pane(colors, cx)),
            )
    }
}
