//! 两种浮层（音质弹窗、播放列表面板）共用的表面外观。
//!
//! 音质弹窗的开合、点外面关闭、焦点和进出场动画都由播放栏自己管（见
//! `player_bar::PlayerBar::quality_popover`），只有锚点定位和延迟绘制借了
//! `base::Popup`。这里剩下本项目的底色/圆角/投影，以及播放列表面板复用的常量。

use gpui::{BoxShadow, Pixels, px};
use gpui_kit::base::ColorTokens;

/// 浮层统一圆角。播放列表面板复用它（只取左侧两角），两种浮层的圆角量始终一致。
pub(super) const SURFACE_RADIUS: Pixels = px(10.);

/// 浮层统一投影。集中在这里，改一次音质弹窗和播放列表面板一起生效。
pub(super) fn surface_shadow(colors: ColorTokens) -> BoxShadow {
    BoxShadow::new(px(0.), px(2.), colors.foreground.alpha(0.1)).blur_radius(px(6.))
}
