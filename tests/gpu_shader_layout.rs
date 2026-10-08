#[test]
fn image_transform_shaders_match_rust_layout_on_storage_and_webgl_backends() {
    let shared = include_str!("../vendor/gpui-pre-wgpu/src/shaders.wgsl");
    for transport in [
        include_str!("../vendor/gpui-pre-wgpu/src/shaders_storage.wgsl"),
        include_str!("../vendor/gpui-pre-wgpu/src/shaders_webgl.wgsl"),
    ] {
        let source = format!("{shared}\n{transport}");
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        let sprite = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("PolychromeSprite"))
            .unwrap()
            .1;
        let naga::TypeInner::Struct { members, span } = &sprite.inner else {
            panic!("PolychromeSprite must be a struct");
        };
        assert_eq!(
            *span as usize,
            std::mem::size_of::<gpui::PolychromeSprite>()
        );
        assert_eq!(
            members.last().unwrap().offset as usize,
            std::mem::offset_of!(gpui::PolychromeSprite, transformation)
        );
    }
}
