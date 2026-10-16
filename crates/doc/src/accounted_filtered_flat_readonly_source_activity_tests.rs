//! One ignored UNRUN data-custody group. Literal12-case activity words come
//! from independent producer captures. Native stores, masks, layer IDs and
//! larger canvas use our genuinely prepaid synthetic CURRENT fixture; this is
//! not an XCF import/evaluator/source-geometry or numerical acceptance test.
use super::*;
#[path = "../../engine/src/accounted_filtered_flat_source_activity_fixture.rs"]
mod captured;
// Exact authored672-byte effectiveICC, SHA256 b4643370870adf27d5480f7877caf222d3b9fff4982e12852b712ed4531ec251.
// Data-only producer role, distinct from generated current-profile equivalence.
const PROFILE_HEX: &str = "000002a06c636d73044000006d6e74725247422058595a2007ea000a0010000100090001616373704d5346540000000000000000000000000000000000000000000000000000f6d6000100000000d32d6c636d7300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000d64657363000001200000004063707274000001600000003677747074000001980000001463686164000001ac0000002c7258595a000001d8000000146258595a000001ec000000146758595a00000200000000147254524300000214000000206754524300000214000000206254524300000214000000206368726d0000023400000024646d6e640000025800000024646d64640000027c000000246d6c756300000000000000010000000c656e5553000000240000001c00470049004d00500020006200750069006c0074002d0069006e002000730052004700426d6c756300000000000000010000000c656e55530000001a0000001c005000750062006c0069006300200044006f006d00610069006e000058595a20000000000000f6d6000100000000d32d736633320000000000010c42000005defffff325000007930000fd90fffffba1fffffda2000003dc0000c06e58595a200000000000006fa0000038f50000039058595a20000000000000249f00000f840000b6c458595a2000000000000062970000b787000018d9706172610000000000030000000266660000f2a700000d59000013d000000a5b6368726d00000000000300000000a3d70000547c00004ccd0000999a0000266700000f5c6d6c756300000000000000010000000c656e5553000000080000001c00470049004d00506d6c756300000000000000010000000c656e5553000000080000001c0073005200470042";
fn profile() -> Vec<u8> {
    PROFILE_HEX
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |v: u8| match v {
                b'0'..=b'9' => v - b'0',
                b'a'..=b'f' => v - b'a' + 10,
                _ => panic!("literalhex"),
            };
            digit(pair[0]) * 16 + digit(pair[1])
        })
        .collect()
}
fn data_owner(case: &captured::Case) -> AccountedDocument {
    let q = quota();
    let sample = if case.precision == 650 { SampleType::F32 } else { SampleType::F64 };
    let (mut doc, guard) = fixture(&q, sample);
    // Existing16MiBIndex+16MiBResident preholds precede all model/mask/profile
    // clones below; nativeStore/BackedRaster receipts remain genuine same-q.
    let mut layer = doc.layers[2].clone();
    layer.id = LayerId(4);
    doc.layers.push(layer);
    let state = doc.xcf_state.as_mut().unwrap();
    let mut stack = state.filter_stacks[1].clone();
    stack.owner = ItemRef::Layer(LayerId(4));
    stack.nodes[0].id = FilterId(3);
    stack.nodes[0].mask = Some(ItemRef::Mask(MaskId(3)));
    state.filter_stacks.push(stack);
    let mut mask = state.filter_masks[1].clone();
    mask.id = MaskId(3);
    mask.owner = FilterId(3);
    state.filter_masks.push(mask);
    let mut row = state.items.iter().find(|i| i.item == ItemRef::Layer(LayerId(3))).unwrap().clone();
    row.item = ItemRef::Layer(LayerId(4));
    state.items.push(row);
    let mut row = state.items.iter().find(|i| i.item == ItemRef::Mask(MaskId(2))).unwrap().clone();
    row.item = ItemRef::Mask(MaskId(3));
    row.mask.as_mut().unwrap().owner = ItemRef::Layer(LayerId(4));
    state.items.push(row);
    state.provenance.source_hash = Some(case.source.sha256.into());
    state.provenance.source_label = case.id.into();
    for (index, root) in case.roots.iter().enumerate() {
        let layer = &mut doc.layers[index];
        layer.name = root.name.into();
        layer.visible = root.visible;
        layer.opacity = f32::from_bits(root.opacity_bits);
        let item = state.items.iter_mut().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap();
        item.name = layer.name.clone();
        item.visible = layer.visible;
        if let Some(enabled) = root.enabled {
            let stack = state.filter_stacks.iter_mut().find(|s| s.owner == ItemRef::Layer(layer.id)).unwrap();
            stack.nodes[0].enabled = enabled;
        }
        for property in root.layer_properties.iter().chain(root.effect_properties) {
            let bytes = Arc::new(property.bytes.to_vec());
            let digest = blake3::hash(&bytes).to_hex().to_string();
            state.opaque.push(crate::xcf_provenance::OpaqueRecord {
                owner: Some(ItemRef::Layer(layer.id)),
                namespace: "data-only-observed-activity".into(),
                identity: format!("{}:{}:{}", property.tag, property.offset, property.order),
                source_version: 22,
                order: property.order,
                flags: 0,
                bytes,
                blake3: digest,
                rendering_required: false,
            });
        }
    }
    doc.icc_profile = Some(Arc::new(profile()));
    doc.working_profile_interpretation =
        WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: blake3::hash(doc.icc_profile.as_ref().unwrap()).to_hex().to_string() };
    AccountedDocument::new_readonly_filtered_flat_point_storage(doc, guard, q, &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN: literal12-case author activity transport data, F32 precedence and copy custody; syntheticCURRENT signedzero separately, no source/numerical claim"]
fn author_activity_words_and_synthetic_signedzero_survive_paid_copy() {
    assert_eq!(captured::CASES.len(), 12);
    let mut fractions = 0;
    let mut hidden = false;
    let mut zero = false;
    let mut disabled = false;
    for case in &captured::CASES {
        let owner = data_owner(case);
        let state = owner.document().xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks.len(), 3);
        for (index, root) in case.roots.iter().enumerate() {
            let layer = &owner.document().layers[index];
            assert_eq!(layer.opacity.to_bits(), root.opacity_bits);
            assert_eq!(layer.visible, root.visible);
            assert_eq!(root.layer_properties.map(|p| p.tag), [6, 33, 8]);
            let compatibility = u32::from_be_bytes(root.layer_properties[0].bytes);
            let operative = u32::from_be_bytes(root.layer_properties[1].bytes);
            assert_eq!(operative, root.opacity_bits);
            if operative == 0.375f32.to_bits() {
                assert_eq!(compatibility, 95);
                assert_ne!((compatibility as f32 / 255.).to_bits(), operative);
                fractions += 1;
            }
            hidden |= !root.visible;
            zero |= f32::from_bits(operative) == 0.;
            disabled |= root.enabled == Some(false);
            assert_eq!(state.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap().visible, layer.visible);
        }
        let original_raw = state.opaque.clone(); // external expectedArc/string harness data
        let copied = owner.try_with_document_name("activity-copy.pcraft", &ctl()).unwrap();
        assert!(Arc::ptr_eq(&owner.owner.retention, &copied.owner.retention));
        let mut expected = owner.document().clone();
        expected.name = "activity-copy.pcraft".into();
        assert_eq!(copied.document(), &expected);
        assert!(Arc::ptr_eq(owner.document().icc_profile.as_ref().unwrap(), copied.document().icc_profile.as_ref().unwrap()));
        for (a, b) in owner.document().xcf_state.as_ref().unwrap().filter_masks.iter().zip(&copied.document().xcf_state.as_ref().unwrap().filter_masks) {
            assert_eq!(a.surface.default_bytes(), b.surface.default_bytes());
            for ((ca, ta), (cb, tb)) in a.surface.tiles().zip(b.surface.tiles()) {
                assert_eq!(ca, cb);
                assert!(Arc::ptr_eq(ta, tb));
            }
        }
        assert_eq!(copied.document().xcf_state.as_ref().unwrap().opaque, original_raw);
        // This signedzero variation is syntheticCURRENT, never a producer-alpha or
        // stored tag33 normalization claim. Original property bytes stay unchanged.
        let id = copied.document().layers[3].id;
        let negative = copied.try_edit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: Some(-0.) }, &ctl()).unwrap();
        assert_eq!(negative.document().layers[3].opacity.to_bits(), (-0f32).to_bits());
        let positive = negative.try_edit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: Some(0.) }, &ctl()).unwrap();
        assert!(!positive.shares_model(&negative));
        assert_eq!(positive.document().layers[3].opacity.to_bits(), 0f32.to_bits());
        assert!(Arc::ptr_eq(&owner.owner.retention, &positive.owner.retention));
        assert_eq!(positive.document().xcf_state.as_ref().unwrap().opaque, original_raw);
        assert_eq!(positive.document().source_color_context, owner.document().source_color_context);
        assert!(Arc::ptr_eq(owner.document().icc_profile.as_ref().unwrap(), positive.document().icc_profile.as_ref().unwrap()));
        for (a, b) in owner.document().layers.iter().zip(&positive.document().layers) {
            assert_eq!(a.content, b.content);
        }
        assert_eq!(positive.document().xcf_state.as_ref().unwrap().filter_stacks, state.filter_stacks);
        assert_eq!(positive.document().xcf_state.as_ref().unwrap().filter_masks, state.filter_masks);
    }
    assert!(fractions > 0 && hidden && zero && disabled);
}
