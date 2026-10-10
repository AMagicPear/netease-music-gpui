use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::*;
use gpui_kit::base::{ColorTokens, Interpolate, Transition, transition};
mod quantize;
use quantize::cover_color;

#[derive(Default)]
pub(super) struct CoverColor {
    cached: Option<(String, Option<Hsla>)>,
}

impl CoverColor {
    pub(super) fn current(&self) -> Option<Hsla> {
        self.cached.as_ref()?.1
    }
    /// 与 img 共用 GPUI 缓存；新图加载中保留上一张封面的颜色。
    pub(super) fn load(&mut self, url: &str, window: &mut Window, cx: &mut App) -> Option<Hsla> {
        if self.cached.as_ref().map(|cached| cached.0.as_str()) != Some(url) {
            if let Some(image) = crate::ui::assets::load_track_cover(url, window, cx) {
                let color = image.ok().and_then(|image| {
                    let size = image.size(0);
                    cover_color(
                        image.as_bytes(0)?,
                        size.width.0.try_into().ok()?,
                        size.height.0.try_into().ok()?,
                    )
                });
                self.cached = Some((url.to_owned(), color));
            }
        }
        self.current()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct CoverGradient(pub [Rgba; 2]);

impl CoverGradient {
    pub(super) fn background(self) -> Background {
        linear_gradient(
            180.,
            linear_color_stop(self.0[0], 0.),
            linear_color_stop(self.0[1], 1.),
        )
    }

    /// 按位置取色：`0.` 是渐变顶端，`1.` 是底端。
    /// 供悬浮层按自己在屏幕上的位置采出恰好对得上的底色。
    pub(super) fn sample(self, position: f32) -> Hsla {
        let position = position.clamp(0., 1.);
        let (from, to) = (self.0[0], self.0[1]);
        // 与 `interpolate` 一致地用 RGB 插值，避免色相绕行。
        Hsla::from(Rgba {
            r: from.r + (to.r - from.r) * position,
            g: from.g + (to.g - from.g) * position,
            b: from.b + (to.b - from.b) * position,
            a: from.a + (to.a - from.a) * position,
        })
    }
}

impl Interpolate for CoverGradient {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        // RGB 插值避免跨过色相 0/1 时绕行整圈。
        Self(std::array::from_fn(|index| {
            let from = self.0[index];
            let to = target.0[index];
            Rgba {
                r: from.r + (to.r - from.r) * progress,
                g: from.g + (to.g - from.g) * progress,
                b: from.b + (to.b - from.b) * progress,
                a: from.a + (to.a - from.a) * progress,
            }
        }))
    }
}

/// 页面背景那一层渐变，连同它在屏幕上的位置。
///
/// 渐变铺满整页且不随内容滚动，所以贴在内容上的浮层要把它当背景时，
/// 必须按自己当前在屏幕上的位置现采，否则一滚就和身后错位。
/// 共享是必需的（多处浮层读同一份），回填是必需的（由绘制阶段写），所以是 `Rc<Cell<_>>`。
#[derive(Clone, Default)]
pub(super) struct Backdrop(Rc<Cell<Option<Frame>>>);

/// 一帧的背景快照。
#[derive(Clone, Copy)]
struct Frame {
    /// 渐变铺开的范围。
    bounds: Bounds<Pixels>,
    gradient: CoverGradient,
}

impl Backdrop {
    fn set(&self, bounds: Bounds<Pixels>, gradient: CoverGradient) {
        self.0.set(Some(Frame { bounds, gradient }));
    }

    /// 当前帧的渐变；还没画过背景时是 `None`。
    pub(super) fn gradient(&self) -> Option<CoverGradient> {
        self.0.get().map(|frame| frame.gradient)
    }

    /// 原样铺满 `bounds`：悬浮栏要和身后连成一片。
    pub(super) fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        self.paint_ramp(bounds, (1., 1.), window);
    }

    /// 铺满 `bounds` 并从透明渐入：浮层底部淡出到背景。
    pub(super) fn paint_fade(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        self.paint_ramp(bounds, (0., 1.), window);
    }

    /// 铺满 `bounds` 并渐出到透明：浮层顶部盖住滚出去的内容。
    /// `strength` 是顶端的不透明度，由调用方按滚出的距离给——顶部留白很薄时，
    /// 没滚动就该是 0，否则会把还没开始滚的第一行一起盖掉。
    pub(super) fn paint_top_fade(
        &self,
        bounds: Bounds<Pixels>,
        strength: f32,
        window: &mut Window,
    ) {
        self.paint_ramp(bounds, (strength, 0.), window);
    }

    /// `alpha` 是上下两端的透明度系数，乘到渐变自身的不透明度上。
    /// 渐变本身可能就带透明度（歌单背景是「主色→透明」），所以必须乘不能盖。
    fn paint_ramp(&self, bounds: Bounds<Pixels>, alpha: (f32, f32), window: &mut Window) {
        let Some(frame) = self.0.get() else {
            return;
        };
        let height = f32::from(frame.bounds.size.height).max(1.);
        let at = |y: Pixels| {
            frame
                .gradient
                .sample((f32::from(y) - f32::from(frame.bounds.top())) / height)
        };
        window.paint_quad(fill(
            bounds,
            linear_gradient(
                180.,
                linear_color_stop(at(bounds.top()).opacity(alpha.0), 0.),
                linear_color_stop(at(bounds.bottom()).opacity(alpha.1), 1.),
            ),
        ));
    }
}

pub(super) fn gradient_layer(
    id: &'static str,
    target: CoverGradient,
    backdrop: Backdrop,
) -> Canvas<CoverGradient> {
    canvas(
        move |bounds, window, cx| {
            let gradient = transition(
                id,
                target,
                Transition::new(Duration::from_millis(600)),
                window,
                cx,
            );
            let changed = backdrop.gradient() != Some(gradient);
            backdrop.set(bounds, gradient);
            if changed {
                // 共享底色在 prepaint 才写入；再通知一次，让缓存控件也采到最终颜色。
                window.request_animation_frame();
            }
            gradient
        },
        |bounds, gradient, window, _| window.paint_quad(fill(bounds, gradient.background())),
    )
}

/// 网易黑胶页的亮度分段映射；保留主色的色相，把上下端点都压入暗色范围。
pub(super) fn dark_gradient(color: Option<Hsla>) -> CoverGradient {
    let color = color.unwrap_or(hsla(2. / 3., 0.12, 0.35, 1.));
    // 官方先把输入 RGB 转成整数 H/S/L，再做分段映射。
    let h = (color.h * 360.).round() / 360.;
    let l = (color.l * 100.).round();
    let mut s = (color.s * 100.).round();
    let (mut top, bottom) = if l < 20. {
        if l == 0. {
            s = 0.;
        } else if l < 5. {
            s /= (100. - l).abs() / l * 0.5;
        } else if l < 15. {
            s /= (l - 50.).powi(2) / 800. - 0.01 * l;
        } else {
            s /= (l - 50.).powi(2) / 800.;
        }
        let top = 0.21 * l + 18.;
        (if top > 20. { top - 1. } else { top }, 0.11 * l + 8.)
    } else if l < 71. {
        let top = 0.1 * l + 30.;
        (if top > 35. { top - 1. } else { top }, 0.1 * l + 10.)
    } else {
        if l < 96. {
            s /= (l - 50.).powi(2) / 400.;
        } else if l < 100. {
            s /= l / (100. - l) * 0.5;
        } else {
            s = 0.;
        }
        (0.21 * l + 20., 0.11 * l + 10.)
    };
    top = top.round() / 100.;
    let s = (s.round() / 100.).clamp(0., 1.);
    // 普通黑胶页另有 25% 黑色蒙层；合并进端点，让所有裁取背景与控件共用最终色。
    CoverGradient([top, bottom.round() / 100.].map(|l| {
        let color = Rgba::from(hsla(h, s, l, 1.));
        Rgba {
            r: color.r * 0.75,
            g: color.g * 0.75,
            b: color.b * 0.75,
            a: 1.,
        }
    }))
}

pub(super) fn dark_colors(mut colors: ColorTokens, background: Hsla) -> ColorTokens {
    colors.background = background;
    colors.surface = background;
    colors.foreground = white();
    colors.secondary_foreground = white().alpha(0.75);
    colors.muted_foreground = white().alpha(0.5);
    colors.border = white().alpha(0.08);
    colors.muted = white().alpha(0.08);
    colors.accent = white().alpha(0.12);
    colors.primary = white().alpha(0.12);
    colors.primary_foreground = white().alpha(0.85);
    colors.secondary = white().alpha(0.08);
    colors
}

/// 按 RGB 通道过渡；灰色的 HSL 色相没有意义，直接插值会绕过绿色等无关颜色。
pub(super) fn blend_color(from: Hsla, to: Hsla, t: f32) -> Hsla {
    if t <= 0. {
        return from;
    }
    if t >= 1. {
        return to;
    }
    CoverGradient([from.into(), to.into()]).sample(t)
}

/// 在两套色板之间按进度对 RGB 通道插值。展开/收起黑胶页时，播放栏用同一个进度
/// 从普通主题色过渡到暗色主题色，而不是整条栏瞬间跳变。
pub(super) fn blend_colors(from: ColorTokens, to: ColorTokens, t: f32) -> ColorTokens {
    macro_rules! blend {
        ($($field:ident),+ $(,)?) => {
            ColorTokens { $($field: blend_color(from.$field, to.$field, t),)+ }
        };
    }
    blend!(
        background,
        foreground,
        surface,
        surface_foreground,
        primary,
        primary_foreground,
        secondary,
        secondary_foreground,
        muted,
        muted_foreground,
        accent,
        accent_foreground,
        destructive,
        destructive_foreground,
        border,
        input,
        ring,
        selection,
    )
}

#[cfg(test)]
mod tests {
    use super::cover_color;

    #[test]
    fn cover_tint_reads_bgra_and_ignores_transparent_pixels() {
        let pixels = [
            0, 0, 255, 255, 0, 0, 255, 255, 255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 0,
            255, 0, 0,
        ];
        assert_eq!(cover_color(&pixels, 6, 1).map(|c| c.h), Some(0.));
        assert_eq!(
            cover_color(&[255, 0, 0, 255], 1, 1).map(|c| c.h),
            Some(2. / 3.)
        );
        assert_eq!(cover_color(&[128, 128, 128, 255], 1, 1).unwrap().s, 0.);
        assert_eq!(cover_color(&[0, 0, 255, 0], 1, 1), None);
        assert_eq!(cover_color(&[], 0, 0), None);
        assert_eq!(cover_color(&[], 1, 1), None);
    }
}
