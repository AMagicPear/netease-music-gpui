//! 全项目共用的加载图标：一个绕自身中心匀速转圈的 `icons/loading.svg`。
//! 素材是一段圆弧，转起来就自然读作「加载中」，各处不必再写「加载中」这三个字。

use std::time::Duration;

use gpui::*;

/// 转一圈的时长。
const SPIN_PERIOD: Duration = Duration::from_millis(900);

/// 旋转的加载图标。`id` 用来标识这段循环动画，同一屏内出现多个时必须各不相同。
pub fn spinner(id: &'static str, size: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path("icons/loading.svg")
        .size(px(size))
        .flex_none()
        .text_color(color)
        .with_animation(id, Animation::new(SPIN_PERIOD).repeat(), |icon, delta| {
            icon.with_transformation(Transformation::rotate(percentage(delta)))
        })
}
