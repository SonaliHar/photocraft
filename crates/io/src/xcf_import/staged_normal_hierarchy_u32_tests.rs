//! UNRUN U32-only additions. Generated encoded/native/negative buffers are
//! external harness data; actual source/model/native custody uses real reader.
use super::tests::data;
use super::*;
use crate::xcf_import::plane::ParsedPlaneDescriptor;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use photocraft_xcf::{BudgetKind, ByteSource, PlainSourceAccounting};
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 })
}
#[test]
#[ignore = "UNRUN: generated U32/350 originalSource6case matrix (2layouts x3versions), explicitplain hierarchy/native lowbits/alphawords/sourceversion/profile/oldlegacyRGBGray priority; no producer numeric oracle"]
fn u32_sources_preserve_lowbits_and_exact_plain_source_role() {
    for layout in 0..2 {
        for version in [12, 13, 22] {
            let encoded = data::image_version(2, layout, version);
            let before = encoded.clone();
            let b = budget();
            let meta = photocraft_xcf::read_plain_source_metadata_with_budget(
                &mut ByteSource::new(&encoded),
                &Default::default(),
                &b,
                PlainSourceAccounting::BorrowedMemory,
            )
            .unwrap();
            let plan = super::super::staged_normal_hierarchy::descriptors(meta.file(), &Interrupt::NONE, &b).unwrap().unwrap();
            assert!(plan.hierarchy_upgrade() && !plan.hierarchy_coverage());
            drop(plan);
            drop(meta);
            let q = quota();
            let source = crate::import_owned_xcf_bytes_in_quota(
                "u32-plain.xcf",
                std::sync::Arc::new(encoded.clone()),
                &Default::default(),
                &photocraft_xcf::ResourceLimits::for_policy(Default::default())
                    .unwrap()
                    .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
                    .unwrap(),
                q.clone(),
                &ctl(),
            )
            .unwrap();
            let crate::OwnedXcfImport::NormalHierarchy(source) = source else { panic!("explicit integer Sourcecarrier") };
            let owner = source.document();
            assert!(owner.is_readonly_normal_hierarchy_storage());
            assert!(!owner.is_legacy_storage());
            let d = owner.document();
            assert_eq!(d.depth, photocraft_color::SampleType::U32);
            assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, 350);
            assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
            assert_eq!(d.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
            let s = d.xcf_state.as_ref().unwrap();
            assert_eq!(s.provenance.format_version, version);
            assert!(s.filter_masks.is_empty() && s.filter_stacks.iter().all(|s| s.nodes.is_empty()));
            for item in &s.items {
                assert_eq!(item.composition.as_ref().unwrap().compatibility_epoch, format!("xcf-{version}-normal"));
            }
            let rows = owner.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
            for row in rows.rows() {
                if let LayerContent::BackedRaster(native) = &row.layer.content {
                    let leaf = match row.layer.name.as_str() {
                        "LeafA" => 0,
                        "LeafB" => 1,
                        "Background" => 2,
                        _ => panic!("native source leaf"),
                    };
                    let expected = data::native(2, layout, leaf);
                    let mut actual = Vec::new();
                    let mut pixel = [0; 16];
                    for y in 0..d.size.height {
                        for x in 0..d.size.width {
                            native.checked_view().read_pixel_native_into(x as i32, y as i32, &mut pixel, &ctl()).unwrap();
                            actual.extend_from_slice(&pixel);
                        }
                    }
                    assert_eq!(actual, expected);
                    if leaf == 0 {
                        assert_eq!(u32::from_ne_bytes(actual[..4].try_into().unwrap()), 0x80000000);
                        assert_eq!(u32::from_ne_bytes(actual[16..20].try_into().unwrap()), 0x80000001);
                    }
                } else {
                    assert!(s.items[row.canonical_item_index].raster_bounds.is_none());
                }
            }
            drop(rows);
            drop(source);
            assert_eq!(q.usage().unwrap().resident, 0);
            assert_eq!(q.usage().unwrap().index, 0);
            assert_eq!(encoded, before);
        }
    }
    // Raw distinction survives although an intermediate normalizedF32 loses it.
    let a = 0x80000000u32 as f64 / u32::MAX as f64;
    let b = 0x80000001u32 as f64 / u32::MAX as f64;
    assert_ne!(a.to_bits(), b.to_bits());
    assert_eq!((a as f32).to_bits(), (b as f32).to_bits());
    for gray in [false, true] {
        let words = if gray {
            vec![0x80000000u32, u32::MAX, 0x80000001, 0xc0000001]
        } else {
            vec![0x80000000u32, 0x40000003, 0xc0000005, u32::MAX, 0x80000001, 0x40000007, 0xc0000009, 0xc0000001]
        };
        let raw: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
        let expected: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        let options = crate::xcf_fixture::FixtureOptions { model: u32::from(gray), ..Default::default() };
        let encoded = crate::xcf_fixture::image_custom(13, 350, 0, 2, 1, &raw, &options);
        let b = budget();
        let meta = photocraft_xcf::read_plain_source_metadata_with_budget(
            &mut ByteSource::new(&encoded),
            &Default::default(),
            &b,
            PlainSourceAccounting::BorrowedMemory,
        )
        .unwrap();
        assert!(super::super::staged_normal_hierarchy::descriptors(meta.file(), &Interrupt::NONE, &b).unwrap().is_none());
        drop(meta);
        let q = quota();
        let source = crate::import_owned_xcf_bytes_in_quota(
            "old-u32.xcf",
            std::sync::Arc::new(encoded),
            &Default::default(),
            &photocraft_xcf::ResourceLimits::for_policy(Default::default())
                .unwrap()
                .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
                .unwrap(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let crate::OwnedXcfImport::Plain(source) = source else { panic!("old nongroup Legacy class") };
        assert!(source.document().is_legacy_storage());
        let LayerContent::BackedRaster(native) = &source.document().document().layers[0].content else { panic!("old native") };
        let mut actual = Vec::new();
        let mut pixel = vec![0; if gray { 8 } else { 16 }];
        for x in 0..2 {
            native.checked_view().read_pixel_native_into(x, 0, &mut pixel, &ctl()).unwrap();
            actual.extend_from_slice(&pixel);
        }
        assert_eq!(actual, expected);
        drop(source);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
}
struct Words {
    data: Vec<u8>,
    chunk: usize,
    declared: Option<u64>,
    late: bool,
}
impl ExactParsedPlane for Words {
    fn descriptor(&self) -> ParsedPlaneDescriptor {
        ParsedPlaneDescriptor { width: 65, height: 1, channels: 4, sample: Sample::U32 }
    }
    fn native_byte_len(&self) -> Option<u64> {
        self.declared
    }
    fn visit_native(&self, i: &Interrupt<'_>, v: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        for c in self.data.chunks(self.chunk) {
            i.check().map_err(|_| IoError::Cancelled)?;
            v(c)?;
            if self.late {
                return Err(IoError::Unsupported("late external source".into()));
            }
        }
        Ok(())
    }
}
#[test]
#[ignore = "UNRUN: privateU32word4 native64wholepixels/cumulativework/control/hostile provider; newplan semantic miss old-only/globaldisabledfx/refusal and immutable oldfloat helpers"]
fn u32_native_control_and_preallocation_do_not_grant_filters() {
    let mut p = Words { data: vec![0xff; 65 * 16], chunk: 64 * 16, declared: Some(65 * 16), late: false };
    let b = budget();
    assert!(native(&p, Sample::U32, 65, 1, &Interrupt::NONE, &b).unwrap());
    assert_eq!(b.used(BudgetKind::Work), 17 + 65 * 16 + 65 * 4);
    assert!(native(&p, Sample::U32, 65, 1, &Interrupt::NONE, &b).unwrap());
    assert_eq!(b.used(BudgetKind::Work), 2 * (17 + 65 * 16 + 65 * 4));
    assert!(!crate::xcf_import::staged_filtered_flat::native(&p, Sample::U32, 65, 1, 4, false, &Interrupt::NONE, Some(&budget())).unwrap());
    p.chunk = 65 * 16;
    assert!(!native(&p, Sample::U32, 65, 1, &Interrupt::NONE, &budget()).unwrap());
    p.chunk = 17;
    assert!(!native(&p, Sample::U32, 65, 1, &Interrupt::NONE, &budget()).unwrap());
    p.chunk = 64 * 16;
    p.data.pop();
    assert!(!native(&p, Sample::U32, 65, 1, &Interrupt::NONE, &budget()).unwrap());
    p.data = vec![0; 66 * 16];
    assert!(!native(&p, Sample::U32, 65, 1, &Interrupt::NONE, &budget()).unwrap());
    p.data.truncate(65 * 16);
    p.declared = None;
    assert!(!native(&p, Sample::U32, 65, 1, &Interrupt::NONE, &budget()).unwrap());
    p.declared = Some(65 * 16);
    p.late = true;
    assert!(native(&p, Sample::U32, 65, 1, &Interrupt::NONE, &budget()).is_err());
    p.late = false;
    assert!(matches!(native(&p, Sample::U32, 65, 1, &Interrupt::cancel_only(&|| true), &budget()), Err(IoError::Cancelled)));
    let low = photocraft_xcf::Budget::new(
        photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 1316)]).unwrap(),
        None,
    );
    assert!(matches!(native(&p,Sample::U32,65,1,&Interrupt::NONE,&low),Err(IoError::Xcf(e)) if e.kind==ErrorKind::Limit));
    let encoded = data::image(2);
    let b = budget();
    let meta =
        photocraft_xcf::read_plain_source_metadata_with_budget(&mut ByteSource::new(&encoded), &Default::default(), &b, PlainSourceAccounting::BorrowedMemory)
            .unwrap();
    let mut f = meta.file().clone();
    let leaf = f.layers.iter_mut().find(|d| !d.group).unwrap();
    leaf.effects.push(photocraft_xcf::Effect {
        name: "external disabled record".into(),
        icon: String::new(),
        operation: "gegl:invert-gamma".into(),
        version: Some("0:0".into()),
        properties: Vec::new(),
        arguments: Vec::new(),
        mask: None,
        state: photocraft_xcf::EffectState { enabled: Some(false), ..Default::default() },
    });
    assert!(!plan_eligible(&f, &Interrupt::NONE, &b).unwrap());
    assert!(super::super::staged_normal_hierarchy::descriptors(&f, &Interrupt::NONE, &b).unwrap().is_none());
    assert!(preflight(&f, &Interrupt::NONE, &b).is_err());
    assert!(matches!(plan_eligible(&f, &Interrupt::cancel_only(&|| true), &b), Err(IoError::Cancelled)));
    f.layers.iter_mut().find(|d| !d.group).unwrap().effects.clear();
    f.precision = 375;
    assert!(!exact_depth(&f));
    assert!(super::super::staged_normal_hierarchy::descriptors(&f, &Interrupt::NONE, &b).unwrap().is_none());
}
