use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{prelude::FluentBuilder, *};

use super::vinyl_stage;
use crate::playback::PlaybackController;
use crate::ui::assets::{track_cover_image, track_cover_url};

const LARGE_COVER_RATIO: f32 = 0.63;

/// 大小唱片共用时钟；暂停后保留角度，恢复播放时继续转。
#[derive(Clone, Copy, Default)]
pub(in crate::ui) struct RotationClock {
    elapsed: Duration,
    started: Option<Instant>,
}

impl RotationClock {
    fn sync(&mut self, playing: bool, now: Instant) {
        if playing {
            self.started.get_or_insert(now);
        } else if let Some(started) = self.started.take() {
            self.elapsed += now.duration_since(started);
        }
    }

    fn angle(self, now: Instant) -> f32 {
        let elapsed = self.elapsed
            + self
                .started
                .map_or(Duration::ZERO, |started| now.duration_since(started));
        (elapsed.as_secs_f64() % 40.) as f32 / 40. * std::f32::consts::TAU
    }
}

/// 旋转只通知这个小视图，歌曲信息和页面列表不随它逐帧重建。
pub(in crate::ui) struct Vinyl {
    clock: Rc<Cell<RotationClock>>,
    disc_path: &'static str,
    cover_url: Option<String>,
    cover: Option<SharedString>,
    playing: bool,
    placement: Rc<Cell<Option<Placement>>>,
    _playback_subscription: Subscription,
}

#[derive(Clone, Copy)]
struct Placement {
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
    opacity: f32,
    arm_angle: Option<f32>,
}

impl Vinyl {
    pub(in crate::ui) fn new(
        playback: Entity<PlaybackController>,
        clock: Rc<Cell<RotationClock>>,
        disc_path: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&playback, |this, playback, cx| {
            if this.sync(playback.read(cx)) {
                cx.notify();
            }
        });
        let mut this = Self {
            clock,
            disc_path,
            cover_url: None,
            cover: None,
            playing: false,
            placement: Rc::default(),
            _playback_subscription: subscription,
        };
        this.sync(playback.read(cx));
        this
    }

    pub(in crate::ui) fn clear(&self) {
        self.placement.set(None);
    }

    /// 页面只留下占位；动画视图挂在缓存页面旁边，不会使整页缓存失效。
    pub(in crate::ui) fn placeholder(
        &self,
        side: f32,
        opacity: f32,
        arm_angle: Option<f32>,
    ) -> Div {
        let placement = self.placement.clone();
        div().size(px(side)).flex_none().relative().child(
            canvas(
                move |bounds, window, _| {
                    placement.set(Some(Placement {
                        bounds,
                        mask: window.content_mask(),
                        opacity,
                        arm_angle,
                    }));
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
    }

    fn sync(&mut self, playback: &PlaybackController) -> bool {
        let snapshot = playback.snapshot();
        let playing_changed = self.playing != snapshot.is_playing;
        self.playing = snapshot.is_playing;
        let mut clock = self.clock.get();
        clock.sync(self.playing, Instant::now());
        self.clock.set(clock);
        let cover_url = snapshot.current_song.as_ref().map(|song| {
            track_cover_url(
                song.al.pic_url.as_deref(),
                if self.disc_path == "images/disc.png" {
                    480
                } else {
                    80
                },
            )
        });
        let cover_changed = self.cover_url != cover_url;
        if cover_changed {
            self.cover_url = cover_url.clone();
            self.cover = cover_url.map(Into::into);
        }
        playing_changed || cover_changed
    }
}

impl Render for Vinyl {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let placement = self.placement.clone();
        let clock = self.clock.clone();
        let disc_path = self.disc_path;
        let cover_ratio = if disc_path == "images/disc.png" {
            LARGE_COVER_RATIO
        } else {
            2. / 3.
        };
        let cover = self.cover.clone();
        let playing = self.playing;
        canvas(
            move |_, window, cx| {
                let placement = placement.get()?;
                if placement.opacity <= 0. || !placement.bounds.intersects(&placement.mask.bounds) {
                    return None;
                }
                let side = placement.bounds.size.width;
                let angle = clock.get().angle(Instant::now());
                let disc = div()
                    .size(side)
                    .relative()
                    .child(
                        img(disc_path)
                            .size_full()
                            .with_transformation(Transformation::rotate(radians(angle))),
                    )
                    .when_some(cover.clone(), |disc, cover| {
                        disc.child(
                            img(track_cover_image(cover))
                                .absolute()
                                .left(relative((1. - cover_ratio) / 2.))
                                .top(relative((1. - cover_ratio) / 2.))
                                .size(relative(cover_ratio))
                                .rounded_full()
                                .object_fit(ObjectFit::Cover)
                                .with_transformation(Transformation::rotate(radians(angle))),
                        )
                    });
                let content = match placement.arm_angle {
                    Some(arm_angle) => {
                        vinyl_stage(f32::from(side), Some(disc), arm_angle).into_any_element()
                    }
                    None => disc.into_any_element(),
                };
                let mut element = div()
                    .size(side)
                    .opacity(placement.opacity)
                    .child(content)
                    .into_any_element();
                window.with_content_mask(Some(placement.mask), |window| {
                    element.layout_as_root(size(side.into(), side.into()), window, cx);
                    element.prepaint_at(placement.bounds.origin, window, cx);
                });
                Some((element, placement.mask, playing && !cx.reduce_motion()))
            },
            |_, frame, window, cx| {
                if let Some((mut element, mask, animate)) = frame {
                    window.with_content_mask(Some(mask), |window| element.paint(window, cx));
                    if animate && window.is_visible() {
                        window.request_animation_frame();
                    }
                }
            },
        )
        .absolute()
        .inset_0()
    }
}

#[cfg(test)]
mod tests {
    use super::RotationClock;
    use std::time::{Duration, Instant};

    #[test]
    fn shared_clock_preserves_phase_across_pause_and_duplicate_updates() {
        let now = Instant::now();
        let mut clock = RotationClock::default();
        clock.sync(true, now);
        clock.sync(true, now + Duration::from_secs(5));
        let quarter = clock.angle(now + Duration::from_secs(10));
        assert!((quarter - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        clock.sync(false, now + Duration::from_secs(10));
        clock.sync(false, now + Duration::from_secs(20));
        assert_eq!(clock.angle(now + Duration::from_secs(30)), quarter);
        clock.sync(true, now + Duration::from_secs(30));
        assert!((clock.angle(now + Duration::from_secs(40)) - std::f32::consts::PI).abs() < 1e-6);
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn overlay_animates_without_rebuilding_content_and_stops_when_hidden(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::Vinyl;
        use crate::playback::PlaybackController;
        use gpui::{prelude::*, *};
        use std::{cell::Cell, rc::Rc};

        struct Content {
            vinyl: Entity<Vinyl>,
            renders: Rc<Cell<usize>>,
            visible: bool,
        }
        impl Render for Content {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.renders.set(self.renders.get() + 1);
                let vinyl = self.vinyl.read(cx);
                vinyl.clear();
                div().size_full().when(self.visible, |content| {
                    content.child(vinyl.placeholder(60., 1., None))
                })
            }
        }
        struct Host {
            content: Entity<Content>,
            vinyl: Entity<Vinyl>,
        }
        impl Render for Host {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .relative()
                    .size_full()
                    .child(
                        self.content
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    )
                    .child(self.vinyl.clone())
            }
        }
        let renders = Rc::new(Cell::new(0));
        let window = cx.open_window(size(px(200.), px(200.)), |_, cx| {
            let playback = cx.new(|_| PlaybackController::default());
            let vinyl = cx.new(|cx| {
                let mut vinyl = Vinyl::new(playback, Rc::default(), "images/miniVinyl.png", cx);
                vinyl.playing = true;
                let mut clock = vinyl.clock.get();
                clock.sync(true, Instant::now());
                vinyl.clock.set(clock);
                vinyl
            });
            let content = cx.new(|_| Content {
                vinyl: vinyl.clone(),
                renders: renders.clone(),
                visible: true,
            });
            Host { content, vinyl }
        });
        cx.run_until_parked();
        let initial_renders = renders.get();
        assert!(initial_renders > 0);
        for _ in 0..3 {
            assert!(
                window
                    .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                    .unwrap()
                    > 0
            );
            cx.run_until_parked();
            assert_eq!(renders.get(), initial_renders);
        }
        window
            .update(cx, |host, _, cx| {
                host.content.update(cx, |content, cx| {
                    content.visible = false;
                    cx.notify();
                })
            })
            .unwrap();
        cx.run_until_parked();
        // 已排队的旧帧可以完成，隐藏后不会再追加动画帧。
        window
            .update(cx, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            0
        );
        window
            .update(cx, |host, _, cx| {
                host.content.update(cx, |content, cx| {
                    content.visible = true;
                    cx.notify();
                })
            })
            .unwrap();
        cx.run_until_parked();
        assert!(
            window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap()
                > 0
        );
    }
}
