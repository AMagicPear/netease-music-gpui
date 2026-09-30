use gpui::{Pixels, Rgba, px, rgb, rgba};
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

#[derive(Clone, Copy)]
pub struct Theme {
    pub app_background: Rgba,
    pub player_bar_background: Rgba,
    pub black1: Rgba,
    pub black3: Rgba,
    pub black5: Rgba,
    pub black10: Rgba,
    pub sidebar_subtle: Rgba,
    pub primary: Rgba,
    pub white1: Rgba,
}

impl Theme {
    pub fn netease() -> Self {
        Self {
            app_background: rgb(0xf7f9fc),
            player_bar_background: rgb(0xfafafa),
            black1: rgb(0x283248),
            black3: rgba(0x283248cc),
            black5: rgba(0x28324899),
            black10: Rgba {
                r: 40. / 255.,
                g: 50. / 255.,
                b: 72. / 255.,
                a: 0.1,
            },
            sidebar_subtle: Rgba {
                r: 40. / 255.,
                g: 50. / 255.,
                b: 72. / 255.,
                a: 0.06,
            },
            primary: rgb(0xfc3d49),
            white1: rgb(0xffffff),
        }
    }
}
