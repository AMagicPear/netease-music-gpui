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
            let source = Resource::Uri(url.to_owned().into());
            if let Some(image) = window.use_asset::<ImgResourceLoader>(&source, cx) {
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

/// 保存当前帧的颜色和窗口坐标，悬浮栏从相同的渐变位置裁取背景。
pub(super) type Backdrop = Rc<Cell<Option<(Bounds<Pixels>, CoverGradient)>>>;

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
            backdrop.set(Some((bounds, gradient)));
            gradient
        },
        |bounds, gradient, window, _| window.paint_quad(fill(bounds, gradient.background())),
    )
}

pub(super) fn paint_backdrop(bounds: Bounds<Pixels>, backdrop: &Backdrop, window: &mut Window) {
    if let Some((gradient_bounds, gradient)) = backdrop.get() {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(gradient_bounds, gradient.background()));
        });
    }
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

#[cfg(test)]
mod tests {
    use super::{cover_color, dark_gradient};
    use gpui::{Hsla, hsla};

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn floating_background_uses_same_gradient_coordinates_and_stops(cx: &mut gpui::TestAppContext) {
        use super::{Backdrop, CoverGradient, gradient_layer, paint_backdrop};
        use gpui::prelude::*;
        use gpui::*;
        struct Host {
            backdrop: Backdrop,
        }
        impl Render for Host {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let backdrop = self.backdrop.clone();
                div()
                    .size_full()
                    .relative()
                    .bg(white())
                    .child(
                        gradient_layer(
                            "test-gradient",
                            CoverGradient([
                                hsla(0.6, 1., 0.5, 0.1).into(),
                                hsla(0.6, 1., 0.5, 0.).into(),
                            ]),
                            self.backdrop.clone(),
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(px(480.)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(72.))
                            .left_0()
                            .w_full()
                            .h(px(110.))
                            .bg(white())
                            .child(
                                canvas(
                                    |_, _, _| {},
                                    move |bounds, _, window, _| {
                                        paint_backdrop(bounds, &backdrop, window)
                                    },
                                )
                                .absolute()
                                .inset_0(),
                            ),
                    )
            }
        }
        let window = cx.open_window(size(px(600.), px(600.)), |_, _| Host {
            backdrop: Default::default(),
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            let quads = window.painted_quads();
            let gradients: Vec<_> = quads
                .iter()
                .filter(|quad| quad.background.as_solid().is_none())
                .collect();
            assert_eq!(gradients.len(), 2);
            assert_eq!(gradients[0].bounds, gradients[1].bounds);
            assert_eq!(gradients[0].background, gradients[1].background);
            assert!(
                gradients[1].content_mask.bounds.size.height
                    < gradients[0].content_mask.bounds.size.height
            );
        })
        .unwrap();
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
