use gpui::{App, Hsla, Pixels, px, rgb};
use gpui_kit::base::Theme;
use std::time::Duration;

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

/// 动画时长
pub const ANIMATION_DURATION: Duration = Duration::from_millis(130);

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
