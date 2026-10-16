//! Deferred wide plain Source fixture. This creates a real paid Legacy owner
//! through the unchanged plain XCF reader, never a copied/fake Source Document.
#![allow(dead_code)]
#[path = "accounted_wide_plain_xcf_fixture.rs"]
pub mod data;
use photocraft_color::SampleType;
use photocraft_doc::{Document, LayerContent};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
pub fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
pub fn index(sample: SampleType) -> usize {
    match sample {
        SampleType::U32 => 0,
        SampleType::F64 => 1,
        _ => panic!("wide fixture format"),
    }
}
pub fn encoded(gray: bool, sample: SampleType, alpha_lock: bool) -> Vec<u8> {
    data::image(gray, index(sample), alpha_lock)
}
pub fn expected(gray: bool, sample: SampleType) -> Vec<u8> {
    data::native(gray, index(sample))
}
pub fn import(gray: bool, sample: SampleType, alpha_lock: bool) -> photocraft_io::OwnedPlainXcfImport {
    let q =
        BackingQuota::new_with_aggregate_initial(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 }, 512 << 20, QuotaUsage::default()).unwrap().0;
    photocraft_io::import_owned_staged_plain_bytes_in_quota(
        "legacy-wide.xcf",
        std::sync::Arc::new(encoded(gray, sample, alpha_lock)),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        q,
        &ctl(),
    )
    .unwrap()
}
pub fn payload(doc: &Document) -> Vec<u8> {
    let LayerContent::BackedRaster(r) = &doc.layers[0].content else { panic!("real native layer") };
    let area = r.operative_bounds();
    let bpp = r.format().bytes_per_pixel();
    let mut bytes = vec![0; area.width() as usize * area.height() as usize * bpp];
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let at = ((y - area.y0) as usize * area.width() as usize + (x - area.x0) as usize) * bpp;
            r.checked_view().read_logical_pixel_native_into(x, y, &mut bytes[at..at + bpp], &ctl()).unwrap();
        }
    }
    bytes
}
pub fn patch() -> serde_json::Value {
    serde_json::json!({"points":[[254.0,254.0,0.7],[257.0,258.0,1.0]],"size":7.0,"hardness":0.6,"spacing":0.3,"color":[0.8,0.2,0.1,0.75],"seed":53})
}
