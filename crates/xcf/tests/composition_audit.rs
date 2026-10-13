#[test]
#[ignore = "requires reviewed independently authored composition originals"]
fn hard_mix_original_source_envelopes_are_exact() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/composition-matrix");
    for name in ["mode-hard-mix.xcf", "mode-hard-mix.edit.xcf"] {
        let file = photocraft_xcf::decode(&std::fs::read(base.join(name)).unwrap(), &Default::default()).unwrap();
        println!("{name}: header={} precision={} icc={}", file.version, file.precision, file.icc.len());
        assert_eq!(file.layers.len(), 2);
        for layer in file.layers {
            println!(
                "{}: mode={} tuple={}/{}/{} opacityBits={} offset={:?} size={}x{} mask={} effects={}",
                layer.name,
                layer.mode,
                layer.composite_mode,
                layer.composite_space,
                layer.blend_space,
                layer.opacity.to_bits(),
                layer.offset,
                layer.width,
                layer.height,
                layer.mask.is_some(),
                layer.effects.len()
            );
            assert!(layer.mask.is_none() && layer.effects.is_empty());
            assert!(matches!(layer.mode, 28 | 51));
        }
    }
}
