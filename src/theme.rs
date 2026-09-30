use gpui::{Rgba, rgb, rgba};

#[derive(Clone, Copy)]
pub struct Theme {
    pub app_background: Rgba,
    pub player_bar_background: Rgba,
    pub black1: Rgba,
    pub black3: Rgba,
    pub black5: Rgba,
    pub black10: Rgba,
    pub secondary1_1: Rgba,
    pub secondary1_2: Rgba,
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
            black10: rgba(0x2832481a),
            secondary1_1: rgb(0xfc3b5b),
            secondary1_2: rgb(0xfc3d49),
            white1: rgb(0xffffff),
        }
    }
}
