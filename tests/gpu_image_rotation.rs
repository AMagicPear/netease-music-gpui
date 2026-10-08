#![cfg(target_os = "macos")]

use std::borrow::Cow;

use gpui::{
    AtlasKey, Bounds, ContentMask, Corners, DevicePixels, ImageId, PlatformHeadlessRenderer,
    PolychromeSprite, RenderImageParams, ScaledPixels, Scene, TransformationMatrix, point, radians,
    size,
};
use gpui_apple::metal_renderer::MetalHeadlessRenderer;

#[test]
fn metal_rotates_cached_image_clockwise_and_keeps_circular_clip() {
    let mut renderer = MetalHeadlessRenderer::new();
    let atlas = renderer.sprite_atlas();
    let key: AtlasKey = RenderImageParams {
        image_id: ImageId(987654),
        frame_index: 0,
    }
    .into();
    // BGRA texture with a red marker above the center.
    let mut pixels = vec![32u8; 80 * 80 * 4];
    for y in 0..80 {
        for x in 0..80 {
            let offset = (y * 80 + x) * 4;
            pixels[offset + 3] = 255;
            if (34..46).contains(&x) && (14..26).contains(&y) {
                pixels[offset..offset + 4].copy_from_slice(&[0, 0, 255, 255]);
            }
        }
    }
    let tile = atlas
        .get_or_insert_with(key.clone(), &mut || {
            Ok(Some((
                size(DevicePixels(80), DevicePixels(80)),
                Cow::Borrowed(&pixels),
            )))
        })
        .unwrap()
        .unwrap();
    let bounds = Bounds::new(
        point(ScaledPixels(20.), ScaledPixels(20.)),
        size(ScaledPixels(80.), ScaledPixels(80.)),
    );
    let mut snapshots = Vec::new();
    for angle in [0., std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_2] {
        let center = bounds.center();
        let transformation = TransformationMatrix::unit()
            .translate(center)
            .rotate(radians(angle))
            .translate(center.map(|value| ScaledPixels(-value.0)));
        let mut scene = Scene::default();
        scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false.into(),
            opacity: 1.,
            bounds,
            content_mask: ContentMask {
                bounds: Bounds::new(
                    point(ScaledPixels(0.), ScaledPixels(0.)),
                    size(ScaledPixels(120.), ScaledPixels(120.)),
                ),
            },
            corner_radii: Corners::all(ScaledPixels(40.)),
            tile,
            transformation,
        });
        let snapshot = renderer
            .render_scene_to_image(&scene, size(DevicePixels(120), DevicePixels(120)))
            .unwrap();
        assert_eq!(
            snapshot.get_pixel(20, 20).0[..3],
            [0, 0, 0],
            "circle corner leaked at {angle}"
        );
        assert_eq!(snapshot.get_pixel(60, 60).0, [32, 32, 32, 255]);
        assert_eq!(scene.polychrome_sprites[0].tile, tile);
        assert_eq!(
            atlas
                .get_or_insert_with(key.clone(), &mut || panic!("texture was uploaded again"))
                .unwrap()
                .unwrap(),
            tile
        );
        snapshots.push(snapshot);
    }
    assert_eq!(snapshots[0].get_pixel(60, 40).0, [255, 0, 0, 255]);
    assert_eq!(snapshots[2].get_pixel(80, 60).0, [255, 0, 0, 255]);
    assert_eq!(snapshots[2].get_pixel(60, 40).0, [32, 32, 32, 255]);
    assert_eq!(std::mem::size_of::<PolychromeSprite>(), 120);
    assert_eq!(std::mem::offset_of!(PolychromeSprite, transformation), 96);
}

#[test]
fn rotated_transparent_image_does_not_sample_neighboring_atlas_tiles() {
    let mut renderer = MetalHeadlessRenderer::new();
    let atlas = renderer.sprite_atlas();
    let transparent = vec![0u8; 128 * 128 * 4];
    let neighbor = [255u8, 0, 255, 255].repeat(128 * 128);
    let mut tile = None;
    // Fill the atlas around a transparent image with a contrasting opaque color.
    for index in 0..64 {
        let pixels = if index == 0 { &transparent } else { &neighbor };
        let inserted = atlas
            .get_or_insert_with(
                RenderImageParams {
                    image_id: ImageId(100000 + index),
                    frame_index: 0,
                }
                .into(),
                &mut || {
                    Ok(Some((
                        size(DevicePixels(128), DevicePixels(128)),
                        Cow::Borrowed(pixels),
                    )))
                },
            )
            .unwrap()
            .unwrap();
        if index == 0 {
            tile = Some(inserted);
        } else {
            assert_eq!(inserted.texture_id, tile.unwrap().texture_id);
        }
    }
    let bounds = Bounds::new(
        point(ScaledPixels(64.), ScaledPixels(64.)),
        size(ScaledPixels(128.), ScaledPixels(128.)),
    );
    for angle in [0.01, 0.3, std::f32::consts::FRAC_PI_4] {
        let center = bounds.center();
        let mut scene = Scene::default();
        scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false.into(),
            opacity: 1.,
            bounds,
            content_mask: ContentMask {
                bounds: Bounds::new(
                    point(ScaledPixels(0.), ScaledPixels(0.)),
                    size(ScaledPixels(256.), ScaledPixels(256.)),
                ),
            },
            corner_radii: Corners::default(),
            tile: tile.unwrap(),
            transformation: TransformationMatrix::unit()
                .translate(center)
                .rotate(radians(angle))
                .translate(center.map(|value| ScaledPixels(-value.0))),
        });
        let snapshot = renderer
            .render_scene_to_image(&scene, size(DevicePixels(256), DevicePixels(256)))
            .unwrap();
        assert!(
            snapshot.pixels().all(|pixel| pixel.0[..3] == [0, 0, 0]),
            "neighboring atlas color leaked at angle {angle}"
        );
    }
}
