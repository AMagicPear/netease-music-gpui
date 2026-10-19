//! 音质选项、标识及浮层交互。

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_kit::base::{Button, ColorTokens};

use super::PlayerBar;
use crate::models::AudioQualityLevel;
use crate::ui::components::{SurfacePopover, icon_hover_color};

impl PlayerBar {
    pub(super) fn quality_popover(
        &self,
        selected_quality: AudioQualityLevel,
        quality_label: &'static str,
        colors: ColorTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 点外捕获可能先关闭弹层，需按本次渲染的状态防止重新打开。
        let was_open = self.quality_open;
        let trigger = Button::new("player-quality-button")
            .aria_label("音质选项")
            .h(px(16.))
            .px(px(2.))
            .flex_none()
            .border_1()
            .border_color(colors.muted_foreground)
            .rounded(px(4.))
            .text_size(px(10.))
            .text_color(colors.muted_foreground)
            .hover(|style| {
                let hover = icon_hover_color(colors.muted_foreground, colors);
                style.text_color(hover).border_color(hover)
            })
            .child(quality_label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.quality_open == was_open {
                        this.set_quality_popover(!was_open, cx);
                    }
                    cx.stop_propagation();
                }),
            );

        let dark = colors.background.l < 0.5;
        let title_color = if dark {
            white().alpha(0.92)
        } else {
            colors.secondary_foreground
        };
        let detail_color = if dark {
            white().alpha(0.45)
        } else {
            colors.muted_foreground.alpha(0.4)
        };
        let hover_bg = if dark {
            white().alpha(0.07)
        } else {
            colors.muted
        };

        #[derive(Clone, Copy)]
        enum QualityMark {
            Gold(&'static str),
            Red(&'static str),
            Neutral(&'static str),
        }
        let mark = move |kind: QualityMark| -> Div {
            let (bg, fg) = match kind {
                QualityMark::Gold(_) => (
                    if dark {
                        Hsla::from(rgb(0xe8bd77)).alpha(0.16)
                    } else {
                        Hsla::from(rgb(0xf7edd6))
                    },
                    if dark {
                        Hsla::from(rgb(0xe8bd77))
                    } else {
                        Hsla::from(rgb(0xd9a648))
                    },
                ),
                QualityMark::Red(_) => (
                    if dark {
                        Hsla::from(rgb(0xc97f71)).alpha(0.18)
                    } else {
                        Hsla::from(rgb(0xfdf0ed))
                    },
                    if dark {
                        Hsla::from(rgb(0xe59d93))
                    } else {
                        Hsla::from(rgb(0xc97f71))
                    },
                ),
                QualityMark::Neutral(_) => (
                    if dark {
                        white().alpha(0.1)
                    } else {
                        colors.muted
                    },
                    if dark {
                        white().alpha(0.6)
                    } else {
                        colors.muted_foreground
                    },
                ),
            };
            let inner: AnyElement = match kind {
                QualityMark::Gold(icon) => svg()
                    .path(icon)
                    .size(px(18.))
                    .text_color(fg)
                    .into_any_element(),
                QualityMark::Red(text) | QualityMark::Neutral(text) => {
                    div().text_size(px(11.)).child(text).into_any_element()
                }
            };
            div()
                .size(px(32.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(bg)
                .text_color(fg)
                .child(inner)
        };

        let entries = [
            (
                AudioQualityLevel::Sky,
                "沉浸环绕声",
                "环绕音感 最高5.1声道",
                QualityMark::Gold("icons/音质选项/immersive_audio.svg"),
                "icons/VIP/svip.svg",
            ),
            (
                AudioQualityLevel::JyMaster,
                "超清母带",
                "极致细节 192kHz/24bit",
                QualityMark::Gold("icons/音质选项/master_audio.svg"),
                "icons/VIP/svip.svg",
            ),
            (
                AudioQualityLevel::JyEffect,
                "高清臻音 (Spatial Audio)",
                "高频细节还原与清晰沉浸感，96kHz/24bit",
                QualityMark::Red("臻"),
                "icons/VIP/vip.svg",
            ),
            (
                AudioQualityLevel::HiRes,
                "高解析度无损 (Hi-Res)",
                "更饱满清晰的高解析度音质，最高192kHz/24bit",
                QualityMark::Red("H"),
                "icons/VIP/vip.svg",
            ),
            (
                AudioQualityLevel::Lossless,
                "无损 (SQ)",
                "高保真无损音质，最高48kHz/16bit",
                QualityMark::Red("SQ"),
                "icons/VIP/vip.svg",
            ),
            (
                AudioQualityLevel::ExHigh,
                "极高 (HQ)",
                "近 CD 音质的细节体验，最高 320kbps",
                QualityMark::Neutral("HQ"),
                "",
            ),
            (
                AudioQualityLevel::Standard,
                "标准",
                "128kbps",
                QualityMark::Neutral("标"),
                "",
            ),
        ];

        SurfacePopover::new("player-quality-popover", was_open, trigger)
            .anchor(Anchor::BottomCenter)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.set_quality_popover(false, cx);
            }))
            .w(px(340.))
            .flex()
            .flex_col()
            .pb(px(8.))
            .when(dark, |panel| {
                panel
                    .bg(hsla(0., 0., 0.12, 1.))
                    .border_1()
                    .border_color(white().alpha(0.08))
            })
            .text_color(title_color)
            .child(
                div()
                    .px(px(16.))
                    .pt(px(12.))
                    .pb(px(8.))
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("当前歌曲音质"),
            )
            .children(
                entries
                    .into_iter()
                    .map(|(quality, title, detail, mark_kind, badge)| {
                        let selected = selected_quality == quality;
                        div()
                            .id((
                                match quality {
                                    AudioQualityLevel::Sky => "quality-sky",
                                    AudioQualityLevel::JyMaster => "quality-master",
                                    AudioQualityLevel::JyEffect => "quality-jyeffect",
                                    AudioQualityLevel::HiRes => "quality-hires",
                                    AudioQualityLevel::Lossless => "quality-lossless",
                                    AudioQualityLevel::ExHigh => "quality-exhigh",
                                    _ => "quality-standard",
                                },
                                0usize,
                            ))
                            .h(px(52.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .mx(px(8.))
                            .px(px(6.))
                            .rounded(px(8.))
                            .hover(|style| style.bg(hover_bg))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.playback
                                    .update(cx, |playback, cx| playback.set_quality(quality, cx));
                            }))
                            .child(mark(mark_kind))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .text_size(px(14.))
                                            .line_height(px(16.))
                                            .child(title)
                                            .when(!badge.is_empty(), |line| {
                                                line.child(
                                                    img(badge).w_auto().h(px(13.5)).flex_none(),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .line_height(px(14.))
                                            .text_color(detail_color)
                                            .child(detail),
                                    ),
                            )
                            .when(selected, |line| {
                                line.child(
                                    img("icons/音质选项/quality_selected.svg")
                                        .size(px(24.))
                                        .flex_none(),
                                )
                            })
                    }),
            )
    }

    fn set_quality_popover(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.quality_open == open {
            return;
        }
        self.quality_open = open;
        cx.notify();
    }
}
