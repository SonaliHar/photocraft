//! UNRUN contracts. Encoded source/decoded negative buffers are external test
//! inputs; only real reader adoption claims actual same-quota custody.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use photocraft_xcf::{BudgetKind, ByteSource, PlainSourceAccounting};
use sha2::{Digest, Sha256};
#[path = "plain_quantized_hierarchy_fixture.rs"]
mod data;
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 })
}
fn import(bytes: &[u8], q: BackingQuota) -> Result<crate::OwnedXcfImport, IoError> {
    crate::import_owned_xcf_from_reader_in_quota(
        "synthetic integer hierarchy",
        &mut std::io::Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        q,
        &ctl(),
    )
}
fn old_point_stack() -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full/filter-stack-edits-evidence/stack.source.xcf");
    let b = std::fs::read(p).unwrap();
    assert_eq!(b.len(), 2348);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), "414a7c66f0cbb78d772b2766592472353acb2e520bec71c9f46daee3070c12f8");
    b
}
#[test]
#[ignore = "UNRUN: synthetic plain U8/U16 original XCF real Source reader/adoption/native words/parents/emptygroups/sourceversions/samequota, not producer-authored or render oracle"]
fn synthetic_originals_preserve_native_and_source_custody() {
    for depth in 0..2 {
        for layout in 0..2 {
            for version in [12, 13, 22] {
                let bytes = data::image_version(depth, layout, version);
                let before = bytes.clone();
                let decoded = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
                assert_eq!(decoded.precision, if depth == 0 { 150 } else { 250 });
                assert!(decoded.layers.iter().all(|d| d.effects.is_empty() && d.mask.is_none()));
                let q = quota();
                let source = import(&bytes, q.clone()).unwrap();
                let crate::OwnedXcfImport::NormalHierarchy(source) = source else { panic!("actual hierarchy source") };
                let owner = source.document();
                assert!(owner.is_readonly_normal_hierarchy_storage());
                let d = owner.document();
                assert_eq!(d.xcf_state.as_ref().unwrap().provenance.format_version, version);
                assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
                let state = d.xcf_state.as_ref().unwrap();
                assert!(state.filter_stacks.iter().all(|s| s.nodes.is_empty()) && state.filter_masks.is_empty());
                let rows = owner.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
                for row in rows.rows() {
                    if let LayerContent::BackedRaster(native) = &row.layer.content {
                        let index = match row.layer.name.as_str() {
                            "LeafA" => 0,
                            "LeafB" => 1,
                            "Background" => 2,
                            _ => panic!("bounded native leaf"),
                        };
                        let expected = data::native(depth, layout, index);
                        let mut actual = Vec::new();
                        let mut pixel = vec![0; if depth == 0 { 4 } else { 8 }];
                        for y in 0..d.size.height {
                            for x in 0..d.size.width {
                                native.checked_view().read_pixel_native_into(x as i32, y as i32, &mut pixel, &ctl()).unwrap();
                                actual.extend_from_slice(&pixel);
                            }
                        }
                        assert_eq!(actual, expected);
                        assert!(row.parent.is_some() || row.layer.name == "Background");
                    } else {
                        assert!(matches!(&row.layer.content, LayerContent::Group(_)));
                        assert!(state.items[row.canonical_item_index].raster_bounds.is_none());
                    }
                }
                assert_eq!(d.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Background", "Group"]);
                if layout == 1 {
                    let empty = rows.rows().iter().find(|r| r.layer.name == "Empty").unwrap();
                    let LayerContent::Group(g) = &empty.layer.content else { panic!("emptygroup") };
                    assert!(g.children.is_empty());
                }
                drop(rows);
                assert!(q.usage().unwrap().resident > 0);
                drop(source);
                assert_eq!(q.usage().unwrap().resident, 0);
                assert_eq!(q.usage().unwrap().index, 0);
                assert_eq!(bytes, before);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: paid four-pair modernimplicit plain partition preserves24genuine oldfixed successes/invalidoldidentity and genuine U8PointStack preplan/source priority; no qualifier-failure retry"]
fn modern_partition_and_old_source_priority_are_disjoint() {
    let encoded = data::image_layout(0, 0);
    let f = photocraft_xcf::decode(&encoded, &Default::default()).unwrap();
    for (sample, precision) in [(Sample::U8, 150), (Sample::U16, 250), (Sample::F32, 650), (Sample::F64, 750)] {
        // Metadata-only role controls. Native buffers are deliberately not retyped or
        // published as these alternative samples; actual integer adoption is above.
        let mut m = f.clone();
        m.sample = sample;
        m.precision = precision;
        let b = budget();
        assert!(!super::super::staged_normal_hierarchy::intrinsic_old_partition(&m, &Interrupt::NONE, &b).unwrap());
        assert!(super::super::staged_normal_hierarchy::modern_plain_old_partition(&m, &Interrupt::NONE, &b).unwrap());
        let expected = if matches!(sample, Sample::U8 | Sample::U16) {
            SelectedStagedFamily::PlainQuantizedNormalHierarchy
        } else {
            SelectedStagedFamily::NormalHierarchy
        };
        assert_eq!(SourceSelectedPolicy::family_controlled(&m, &Interrupt::NONE, Some(&b)).unwrap(), expected);
        m.layers[1].path = vec![9, 0];
        assert!(!super::super::staged_normal_hierarchy::modern_plain_old_partition(&m, &Interrupt::NONE, &b).unwrap());
        assert_eq!(SourceSelectedPolicy::family_controlled(&m, &Interrupt::NONE, Some(&b)).unwrap(), SelectedStagedFamily::IsolatedNormalGroup);
    }
    let (root, rows) = staged_accounted_group::tests::records();
    for row in rows {
        let bytes = staged_accounted_group::tests::read(&root, &row);
        let f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(!super::super::staged_normal_hierarchy::modern_plain_old_partition(&f, &Interrupt::NONE, &budget()).unwrap());
        let q = quota();
        let source = import(&bytes, q.clone()).unwrap();
        assert!(matches!(&source, crate::OwnedXcfImport::IsolatedNormalGroup(_)));
        drop(source);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
        let mut broken = f.clone();
        broken.icc.clear();
        assert!(!super::super::staged_normal_hierarchy::modern_plain_old_partition(&broken, &Interrupt::NONE, &budget()).unwrap());
        assert_eq!(SourceSelectedPolicy::family_controlled(&broken, &Interrupt::NONE, Some(&budget())).unwrap(), SelectedStagedFamily::IsolatedNormalGroup);
        let mut rawonly = f;
        for d in &mut rawonly.layers {
            d.composite_space = -1;
            d.composite_mode = -1;
        }
        assert!(!super::super::staged_normal_hierarchy::modern_plain_old_partition(&rawonly, &Interrupt::NONE, &budget()).unwrap());
    }
    let old = old_point_stack();
    let b = budget();
    let meta =
        photocraft_xcf::read_plain_source_metadata_with_budget(&mut ByteSource::new(&old), &Default::default(), &b, PlainSourceAccounting::BorrowedMemory)
            .unwrap();
    assert!(super::super::staged_normal_hierarchy::descriptors(meta.file(), &Interrupt::NONE, &b).unwrap().is_none());
    drop(meta);
    assert!(
        super::super::staged_masked_filtered_flat::preplan(
            &mut ByteSource::new(&old),
            &Default::default(),
            &b,
            PlainSourceAccounting::BorrowedMemory,
            &Interrupt::NONE
        )
        .unwrap()
        .is_none()
    );
    let q = quota();
    let source = import(&old, q.clone()).unwrap();
    assert!(matches!(&source, crate::OwnedXcfImport::Point(_)));
    drop(source);
    assert_eq!(q.usage().unwrap().resident, 0);
    assert_eq!(q.usage().unwrap().index, 0);
    let mut bad = photocraft_xcf::decode(&old, &Default::default()).unwrap();
    bad.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert_eq!(SourceSelectedPolicy::family_controlled(&bad, &Interrupt::NONE, Some(&budget())).unwrap(), SelectedStagedFamily::PointStackEdits);
    assert!(!super::super::point_stack::qualify(&bad, &Interrupt::NONE, Some(&budget())).unwrap());
}
struct Hostile {
    sample: Sample,
    width: u32,
    payload: Vec<u8>,
    declared: Option<u64>,
    chunk: usize,
    late: bool,
}
impl ExactParsedPlane for Hostile {
    fn descriptor(&self) -> ParsedPlaneDescriptor {
        ParsedPlaneDescriptor { width: self.width, height: 1, channels: 4, sample: self.sample }
    }
    fn native_byte_len(&self) -> Option<u64> {
        self.declared
    }
    fn visit_native(&self, i: &Interrupt<'_>, v: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        for (n, c) in self.payload.chunks(self.chunk).enumerate() {
            i.check().map_err(|_| IoError::Cancelled)?;
            v(c)?;
            if self.late && n == 0 {
                return Err(IoError::Unsupported("external late provider error".into()));
            }
        }
        Ok(())
    }
}
#[test]
#[ignore = "UNRUN: private integer runtime-bpp64pixel visitor exact counts/unknown/oversized/misaligned/short/extra/lateerror/cumulative Work/cancel; no numeric cast or fullplane allocation"]
fn private_native_visitor_counts_actual_bytes_and_controls() {
    for sample in [Sample::U8, Sample::U16] {
        let word = sample.source_bytes();
        let len = 65 * 4 * word;
        let mut p = Hostile { sample, width: 65, payload: vec![0xff; len], declared: Some(len as u64), chunk: 64 * 4 * word, late: false };
        let b = budget();
        assert!(native(&p, sample, 65, 1, &Interrupt::NONE, &b).unwrap());
        let one = 17 + len as u64 + (len / word) as u64;
        assert_eq!(b.used(BudgetKind::Work), one);
        assert!(native(&p, sample, 65, 1, &Interrupt::NONE, &b).unwrap());
        assert_eq!(b.used(BudgetKind::Work), 2 * one);
        p.chunk = len;
        assert!(!native(&p, sample, 65, 1, &Interrupt::NONE, &budget()).unwrap());
        p.chunk = 3;
        assert!(!native(&p, sample, 65, 1, &Interrupt::NONE, &budget()).unwrap());
        p.chunk = 64 * 4 * word;
        p.payload.pop();
        assert!(!native(&p, sample, 65, 1, &Interrupt::NONE, &budget()).unwrap());
        p.payload = vec![0xff; len + 4 * word];
        assert!(!native(&p, sample, 65, 1, &Interrupt::NONE, &budget()).unwrap());
        p.payload.truncate(len);
        p.declared = None;
        assert!(!native(&p, sample, 65, 1, &Interrupt::NONE, &budget()).unwrap());
        p.declared = Some(len as u64);
        p.late = true;
        assert!(native(&p, sample, 65, 1, &Interrupt::NONE, &budget()).is_err());
        p.late = false;
        assert!(matches!(native(&p, sample, 65, 1, &Interrupt::cancel_only(&|| true), &budget()), Err(IoError::Cancelled)));
        let low = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", one - 1)]).unwrap();
        assert!(matches!(native(&p,sample,65,1,&Interrupt::NONE,&photocraft_xcf::Budget::new(low,None)),Err(IoError::Xcf(e)) if e.kind==ErrorKind::Limit));
    }
}
#[test]
#[ignore = "UNRUN: new descriptor preallocation/globalfx0 semantic miss old-only vs final selected Source refusal, typed Work/cancel/quota refunds; external metadata negative buffers"]
fn plain_plan_permissions_and_selected_refusal_remain_separate() {
    for depth in 0..2 {
        let encoded = data::image(depth);
        let b = budget();
        let meta = photocraft_xcf::read_plain_source_metadata_with_budget(
            &mut ByteSource::new(&encoded),
            &Default::default(),
            &b,
            PlainSourceAccounting::BorrowedMemory,
        )
        .unwrap();
        let mut f = meta.file().clone();
        let mut plan = super::super::staged_normal_hierarchy::descriptors(&f, &Interrupt::NONE, &b).unwrap().unwrap();
        assert!(plan.hierarchy_upgrade() && !plan.hierarchy_coverage());
        assert!(plan.complete().is_err());
        for d in &f.layers {
            assert!(d.effects.is_empty());
            if let Some(p) = &d.plane {
                plan.verify(p, &b).unwrap();
            }
        }
        plan.complete().unwrap();
        drop(plan);
        let old = photocraft_xcf::decode(&old_point_stack(), &Default::default()).unwrap();
        let e = &old.layers[0].effects[0];
        let mut e = photocraft_xcf::Effect {
            name: e.name.clone(),
            icon: e.icon.clone(),
            operation: e.operation.clone(),
            version: e.version.clone(),
            properties: e.properties.clone(),
            arguments: e.arguments.clone(),
            state: e.state.clone(),
            mask: None,
        };
        e.state.enabled = Some(false);
        f.layers.iter_mut().find(|d| !d.group).unwrap().effects.push(e);
        assert!(!plan_eligible(&f, &Interrupt::NONE, &b).unwrap());
        assert!(super::super::staged_normal_hierarchy::descriptors(&f, &Interrupt::NONE, &b).unwrap().is_none());
        assert!(preflight(&f, &Interrupt::NONE, &b).is_err());
        assert!(matches!(plan_eligible(&f, &Interrupt::cancel_only(&|| true), &b), Err(IoError::Cancelled)));
        let denied = photocraft_xcf::Budget::new(
            photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 1)]).unwrap(),
            None,
        );
        assert!(matches!(plan_eligible(&f,&Interrupt::NONE,&denied),Err(IoError::Xcf(e)) if e.kind==ErrorKind::Limit));
        drop(meta);
        let q = BackingQuota::new(QuotaLimits { resident: 1, index: 1, scratch: 0 });
        assert!(import(&encoded, q.clone()).is_err());
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
}
