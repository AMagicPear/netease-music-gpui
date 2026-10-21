use std::{cell::Cell, rc::Rc, time::Instant};

use gpui::*;

#[derive(Clone, Copy)]
struct Placement {
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
}

#[derive(Clone)]
pub struct PlayingIndicator {
    placement: Rc<Cell<Option<Placement>>>,
    started_at: Instant,
}

impl Default for PlayingIndicator {
    fn default() -> Self {
        Self {
            placement: Rc::new(Cell::new(None)),
            started_at: Instant::now(),
        }
    }
}

impl PlayingIndicator {
    pub fn clear(&self) {
        self.placement.set(None);
    }

    pub fn anchor(&self) -> impl IntoElement {
        let placement = self.placement.clone();
        canvas(
            move |bounds, window, _| {
                placement.set(Some(Placement {
                    bounds,
                    mask: window.content_mask(),
                }));
            },
            |_, _, _, _| {},
        )
        .w(px(12.))
        .h(px(15.))
    }

    pub fn overlay(&self, color: Hsla) -> impl IntoElement {
        let placement = self.placement.clone();
        let started_at = self.started_at;
        canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                let Some(placement) = placement.get() else {
                    return;
                };
                if !placement.bounds.intersects(&placement.mask.bounds) {
                    return;
                }
                let reduce_motion = cx.reduce_motion();
                let progress = if reduce_motion {
                    0.
                } else {
                    started_at.elapsed().as_secs_f32() / 2.
                };
                window.with_content_mask(Some(placement.mask), |window| {
                    for bounds in indicator_bars(placement.bounds, progress) {
                        window.paint_quad(fill(bounds, color));
                    }
                });
                if !reduce_motion {
                    window.request_animation_frame();
                }
            },
        )
        .absolute()
        .inset_0()
    }
}

fn indicator_bars(bounds: Bounds<Pixels>, progress: f32) -> [Bounds<Pixels>; 3] {
    [0, 1, 2].map(|index| {
        let phase = index as f32 * 0.33;
        let height = px(2. + ((progress + phase) * std::f32::consts::TAU).sin().abs() * 13.);
        Bounds::new(
            point(
                bounds.left() + px(index as f32 * 5.),
                bounds.bottom() - height,
            ),
            size(px(2.), height),
        )
    })
}
