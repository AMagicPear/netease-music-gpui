use std::borrow::Cow;

use gpui::{App, Hsla, Pixels, px, rgb};
use gpui_kit::base::Theme;

/// Dolphin 字体在 .ttf **内部**声明的 family 名称。
///
/// 文本系统按这个名字查找字体，与文件名无关；文件名是 `dolphin.ttf`，
/// 但内部的 family 是 `Dolphin`，而且匹配是大小写敏感的，写错就静默回退到系统字体。
pub const DOLPHIN_FAMILY: &str = "Dolphin";

/// 编译期把字体字节打进二进制，运行时零拷贝地交给文本系统。
const DOLPHIN_MEDIUM: &[u8] = include_bytes!("../assets/font/dolphin.ttf");
const DOLPHIN_BOLD: &[u8] = include_bytes!("../assets/font/dolphin_bold.ttf");

#[derive(Clone, Copy)]
pub enum IconSize {
    #[allow(dead_code)]
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

/// 按钮按下（active）时的整体透明度。
///
/// 这是交互反馈而不是配色：它走元素级的 `opacity`（作用于按钮本身及其所有子元素），
/// 所以不放进 `ColorTokens`、不随主题变化，全站按钮统一一个值。
/// hover 一律只改颜色、不改透明度，两者分工明确。
pub const PRESSED_OPACITY: f32 = 0.8;

/// 注册打包字体。
///
/// 必须早于任何文字测量与 `open_window`：`add_fonts` 之后首次布局才会用上新字体，
/// 之后再注册则还要 `cx.refresh_windows()` 才能重新排版已显示的窗口。
pub fn load_fonts(cx: &mut App) {
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(DOLPHIN_MEDIUM),
            Cow::Borrowed(DOLPHIN_BOLD),
        ])
        .expect("failed to load dolphin fonts");
}

pub fn init(cx: &mut App) {
    let colors = &mut Theme::global_mut(cx).tokens.colors;
    let ink = Hsla::from(rgb(0x283248));

    colors.background = rgb(0xf7f9fc).into();
    colors.surface = rgb(0xfafafa).into();
    colors.foreground = ink;
    colors.secondary_foreground = ink.alpha(0.8);
    colors.muted_foreground = ink.alpha(0.6);
    colors.border = ink.alpha(0.1);
    colors.muted = ink.alpha(0.06);
    colors.accent = ink.alpha(0.06);
    colors.primary = rgb(0xfc3d49).into();
    colors.primary_foreground = rgb(0xffffff).into();
    colors.selection = colors.primary.alpha(0.3);
}
