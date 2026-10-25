//! 带小箭头的悬浮气泡：音量气泡和播放栏的「已添加至播放列表」提示共用同一套外观。
//!
//! 形状是「圆角矩形 + 底边长出一个三角」。三角尖端落在宽度的哪个位置由调用方决定：
//! 音量气泡居中对着图标，播放栏的提示偏左对着播放列表按钮。
//! 底色、圆角、投影三处完全一致，改样式只改这里。

use gpui::{
    Bounds, BoxShadow, Corners, Path, PathBuilder, Pixels, Result, Size, Window, point, px,
};
use gpui_kit::base::ColorTokens;

/// 气泡圆角。
pub(super) const BALLOON_RADIUS: Pixels = px(10.);
/// 底部小三角：宽度沿矩形底边展开，高度单独占气泡底部的一段。
pub(super) const ARROW_WIDTH: Pixels = px(12.);
pub(super) const ARROW_HEIGHT: Pixels = px(6.);

/// 气泡轮廓：圆角矩形，底边中途绕出一个向下的三角。
///
/// 三角直接长在底边上，整条路径一次填充，
/// 不像两个形状拼接那样在小数 DPI 下露出接缝。
///
/// `arrow_tip` 是三角尖端相对气泡左边沿的偏移：居中就是宽度的一半。
pub(super) fn balloon_path(bounds: Bounds<Pixels>, arrow_tip: Pixels) -> Result<Path<Pixels>> {
    let mut path = PathBuilder::fill();
    let left = bounds.origin.x;
    let top = bounds.origin.y;
    let right = bounds.origin.x + bounds.size.width;
    // 矩形底边：三角从这里往下长出去。
    let bottom = top + bounds.size.height - ARROW_HEIGHT;
    let tip_y = top + bounds.size.height;
    let r = BALLOON_RADIUS;
    // 三角要落在底边的直线段上：压到圆角上，底边就会自己折回去。
    let min_tip = left + r + ARROW_WIDTH / 2.;
    let max_tip = right - r - ARROW_WIDTH / 2.;
    let tip_x = if min_tip <= max_tip {
        (left + arrow_tip).clamp(min_tip, max_tip)
    } else {
        // 气泡太窄放不下三角：退回居中。
        left + bounds.size.width / 2.
    };
    let arrow_left = tip_x - ARROW_WIDTH / 2.;
    let arrow_right = tip_x + ARROW_WIDTH / 2.;

    path.move_to(point(left + r, top));
    path.line_to(point(right - r, top));
    path.arc_to(point(r, r), px(0.), false, true, point(right, top + r));
    path.line_to(point(right, bottom - r));
    path.arc_to(point(r, r), px(0.), false, true, point(right - r, bottom));
    // 底边从右往左走，走到三角时绕出去再回来。
    path.line_to(point(arrow_right, bottom));
    path.line_to(point(tip_x, tip_y));
    path.line_to(point(arrow_left, bottom));
    path.line_to(point(left + r, bottom));
    path.arc_to(point(r, r), px(0.), false, true, point(left, bottom - r));
    path.line_to(point(left, top + r));
    path.arc_to(point(r, r), px(0.), false, true, point(left + r, top));
    path.close();
    path.build()
}

/// 把气泡画到窗口上：先投影，再填整条路径。
///
/// 投影只支持矩形（`paint_drop_shadows` 只吃 `corner_radii`），
/// 三角没有自己的投影；当前投影很轻，看不出断裂。
pub(super) fn paint_balloon(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    colors: ColorTokens,
    arrow_tip: Pixels,
) {
    let rect = Bounds::new(
        bounds.origin,
        Size {
            width: bounds.size.width,
            height: bounds.size.height - ARROW_HEIGHT,
        },
    );
    window.paint_drop_shadows(
        rect,
        Corners::all(BALLOON_RADIUS),
        &[BoxShadow::new(px(0.), px(2.), colors.foreground.alpha(0.1)).blur_radius(px(6.))],
    );
    let Ok(path) = balloon_path(bounds, arrow_tip) else {
        return;
    };
    window.paint_path(path, colors.surface);
}
