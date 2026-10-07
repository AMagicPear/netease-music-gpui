use gpui::{App, Hsla, Pixels, px, rgb};
use gpui_kit::component::Theme;

/// Dolphin 字体在 .ttf **内部**声明的 family 名称。
///
/// 文本系统按这个名字查找字体，与文件名无关；文件名是 `dolphin.ttf`，
/// 但内部的 family 是 `Dolphin`，而且匹配是大小写敏感的，写错就静默回退到系统字体。
pub const DOLPHIN_FAMILY: &str = "Dolphin";

#[derive(Clone, Copy)]
pub enum IconSize {
    Small,
    Middle,
    Large,
}

impl IconSize {
    pub const fn pixels(self) -> Pixels {
        match self {
            Self::Small => px(20.),
            Self::Middle => px(22.),
            Self::Large => px(24.),
        }
    }
}

/// 有底色按钮按下（active）时的整体 `opacity`。
///
/// 这是交互反馈而不是配色，所以不放进 `ColorTokens`、不随主题变化，全站按钮统一一个值。
/// 它作用于元素本身及其所有子元素，因此只适合有底色的按钮，别用在无底色的图标上
/// （图标改用 [`PRESSED_ICON_ALPHA`]）。hover 一律只改颜色、不改透明度，两者分工明确。
pub const PRESSED_OPACITY: f32 = 0.8;

/// 图标按下（active）时，`foreground` 的 alpha。
///
/// 图标三态刻意排成「按下最浅 < 普通 < hover 最深」：hover 是视觉上最实的状态，
/// 按下则立刻变淡，作为一种即时、可逆的"按下去"反馈。
/// 这里给 alpha 而不是整体 `opacity`——图标没有底色，叠 `opacity` 会在
/// 「按住后把鼠标移出图标」时因失去 hover、退回更浅的底色而双重变淡。
pub const PRESSED_ICON_ALPHA: f32 = 0.4;

/// 从与图标相同的资源目录读取并注册字体。
///
/// 必须早于任何文字测量与 `open_window`：`add_fonts` 之后首次布局才会用上新字体，
/// 之后再注册则还要 `cx.refresh_windows()` 才能重新排版已显示的窗口。
pub fn load_fonts(cx: &mut App) -> gpui::Result<()> {
    let fonts = ["font/dolphin.ttf", "font/dolphin_bold.ttf"]
        .into_iter()
        .map(|path| {
            cx.asset_source().load(path)?.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("font missing: {path}"),
                )
                .into()
            })
        })
        .collect::<gpui::Result<Vec<_>>>()?;
    cx.text_system().add_fonts(fonts)
}

pub fn init(cx: &mut App) {
    let ink = Hsla::from(rgb(0x283248));
    // Component 是主题来源；update 会同步颜色 token 和 Base 的主题投影。
    Theme::update(cx, |theme| {
        let colors = &mut theme.colors;
        colors.background = rgb(0xf7f9fc).into();
        // Base 的 surface 对应 Component 的 popover。
        colors.popover = rgb(0xfafafa).into();
        colors.foreground = ink;
        colors.secondary_foreground = ink.alpha(0.8);
        colors.muted_foreground = ink.alpha(0.6);
        colors.border = ink.alpha(0.1);
        colors.muted = ink.alpha(0.06);
        colors.accent = ink.alpha(0.06);
        colors.primary = rgb(0xfc3d49).into();
        colors.primary_foreground = rgb(0xffffff).into();
        colors.selection = colors.primary.alpha(0.3);
        colors.progress_bar = colors.primary;
    });
    debug_assert_eq!(
        gpui_kit::base::Theme::global(cx).tokens.colors,
        Theme::global(cx).color_tokens(),
        "Component and Base themes must stay synchronized",
    );
}
