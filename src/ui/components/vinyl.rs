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
