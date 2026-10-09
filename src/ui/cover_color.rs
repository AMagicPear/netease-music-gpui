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
    use super::{
        CoverGradient, blend_color, blend_colors, cover_color, dark_colors, dark_gradient,
    };
    use gpui::{Hsla, Rgba, hsla};

    #[test]
    fn gradient_sampling_hits_its_stops_and_clamps() {
        let stops = [
            Rgba {
                r: 0.1,
                g: 0.2,
                b: 0.3,
                a: 1.,
            },
            Rgba {
                r: 0.7,
                g: 0.8,
                b: 0.9,
                a: 1.,
            },
        ];
        let gradient = CoverGradient(stops);
        assert_eq!(gradient.sample(0.), Hsla::from(stops[0]));
        assert_eq!(gradient.sample(1.), Hsla::from(stops[1]));
        // 中点落在两站之间，而不是贴到某一端。
        assert!(gradient.sample(0.).l < gradient.sample(0.5).l);
        assert!(gradient.sample(0.5).l < gradient.sample(1.).l);
        // 越界夹到两端，不会采出渐变之外的颜色。
        assert_eq!(gradient.sample(-1.), gradient.sample(0.));
        assert_eq!(gradient.sample(2.), gradient.sample(1.));
        // 渐变自带的透明度要能采出来：歌单背景就是「主色 → 透明」，
        // 涂到浮层上时只能乘上自己的系数，不能把透明度盖掉。
        let fading = CoverGradient([stops[0], Rgba { a: 0., ..stops[1] }]);
        assert!((fading.sample(0.).a - 1.).abs() < 1e-6);
        assert!((fading.sample(0.5).a - 0.5).abs() < 1e-6);
        assert!(fading.sample(1.).a.abs() < 1e-6);
    }

    #[test]
    fn blend_colors_runs_between_the_two_palettes() {
        let light = dark_colors(Default::default(), hsla(0., 0., 0.12, 1.));
        let dark = dark_colors(Default::default(), hsla(0.3, 0.5, 0.05, 1.));
        assert_eq!(blend_colors(light, dark, 0.), light);
        let mid = blend_colors(light, dark, 0.5);
        assert_ne!(mid.background, light.background);
        assert_ne!(mid.background, dark.background);
    }

    #[test]
    fn neutral_to_blue_palette_never_flashes_green_in_either_direction() {
        let light = dark_colors(Default::default(), hsla(0., 0., 0.95, 1.));
        let blue = dark_colors(Default::default(), hsla(2. / 3., 0.85, 0.12, 1.));
        for (from, to) in [(light, blue), (blue, light)] {
            let start = Rgba::from(from.surface);
            let end = Rgba::from(to.surface);
            for step in 0..=100 {
                let colors = blend_colors(from, to, step as f32 / 100.);
                for color in [colors.background, colors.surface] {
                    let rgb = Rgba::from(color);
                    assert!(rgb.b + 1e-6 >= rgb.g, "蓝色过渡不应经过绿色：{rgb:?}");
                    for (value, from, to) in [
                        (rgb.r, start.r, end.r),
                        (rgb.g, start.g, end.g),
                        (rgb.b, start.b, end.b),
                    ] {
                        assert!(value >= from.min(to) - 1e-6 && value <= from.max(to) + 1e-6);
                    }
                }
            }
        }
    }

    #[test]
    fn color_blend_preserves_endpoints_alpha_and_rgb_midpoint() {
        let red = hsla(0., 1., 0.5, 1.);
        let blue = hsla(2. / 3., 1., 0.5, 0.2);
        assert_eq!(blend_color(red, blue, 0.), red);
        assert_eq!(blend_color(red, blue, 1.), blue);
        let mid = Rgba::from(blend_color(red, blue, 0.5));
        assert!((mid.r - 0.5).abs() < 1e-6);
        assert!(mid.g.abs() < 1e-6);
        assert!((mid.b - 0.5).abs() < 1e-6);
        assert!((mid.a - 0.6).abs() < 1e-6);
    }

    #[test]
    fn dark_gradient_keeps_hue_and_readable_lightness() {
        for l in [0., 0.04, 0.14, 0.19, 0.2, 0.5, 0.71, 0.95, 0.99, 1.] {
            let gradient = dark_gradient(Some(hsla(0.08, 0.6, l, 1.)));
            let top = Hsla::from(gradient.0[0]);
            let bottom = Hsla::from(gradient.0[1]);
            assert!(top.l > bottom.l && top.l <= 0.42 && bottom.l <= 0.22);
            assert_eq!(top.a, 1.);
            if top.s > 0. {
                assert!((top.h - (0.08f32 * 360.).round() / 360.).abs() < 0.001);
            }
        }
    }

    #[test]
    fn black_vinyl_background_matches_official_hsl_and_mask_oracles() {
        // 从原客户端 52/53/174 模块运行所得；单位为 0..255 的最终显示 RGB。
        for (input, expected) in [
            (0xf80000, [[133.875, 0., 0.], [57.375, 0., 0.]]),
            (
                0xb41e2d,
                [[111.19275, 18.85725, 28.0908], [45.78525, 7.76475, 11.5668]],
            ),
            (0x000000, [[34.425; 3], [15.3; 3]]),
            (0xffffff, [[78.4125; 3], [40.1625; 3]]),
        ] {
            let gradient = dark_gradient(Some(gpui::rgb(input).into()));
            for (color, expected) in gradient.0.into_iter().zip(expected) {
                for (actual, expected) in [color.r, color.g, color.b].into_iter().zip(expected) {
                    assert!((actual * 255. - expected).abs() < 0.001);
                }
            }
        }
    }

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
