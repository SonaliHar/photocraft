//! Synchronous native tile delivery. Sink allocations share the parser ledger.
use crate::{reader::Reader, *};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneRole {
    Raster,
    Channel,
    Mask,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaneDescriptor {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub sample: Sample,
    pub source_sample: Sample,
    pub model: Model,
    pub alpha: bool,
    pub role: PlaneRole,
    pub record_offset: u64,
    pub offset: (i32, i32),
}

/// Checked operation control, not an IO grant. Charged retained allocations must
/// remain owned by the returned plane until the decoded result is dropped.
pub struct PlaneSinkControl<'r, 'a, 'o> {
    pub(crate) reader: &'r mut Reader<'a, 'o>,
}
impl PlaneSinkControl<'_, '_, '_> {
    pub fn poll(&mut self) -> Result<(), XcfError> {
        self.reader.poll()
    }
    pub fn consume_work(&mut self, units: u64) -> Result<(), XcfError> {
        self.poll()?;
        if let Some(budget) = self.reader.budget {
            budget.consume_work(units, "plane sink")?;
        }
        Ok(())
    }
    pub fn charge_retained(&mut self, bytes: usize) -> Result<(), XcfError> {
        self.reader.charge(bytes)
    }
    pub fn allocate_zeroed(&mut self, bytes: usize) -> Result<Vec<u8>, XcfError> {
        self.reader.vec(bytes)
    }
}

/// Callbacks borrow tile bytes only for the call. On any error the entire parse
/// fails; caller-owned sink staging must be discarded, never published partially.
/// Implementations are trusted accounting participants: precharge all retained
/// allocations through control OR keep their own live reservations against the
/// SAME caller Budget. Never charge both owners for one allocation. External
/// leases travel with the returned plane; control-charged allocations travel
/// with the decoded result's parser reservation through mapping.
#[derive(Clone,Copy,PartialEq,Eq)]
pub enum MaskCoveragePolicy{StrictUnit,CheckedF32Preimage}
pub trait PlaneSink {
    /// Called before hierarchy/index/native allocation for the actual plane.
    fn validate_plane_descriptor(&mut self,_:&PlaneDescriptor,_:&mut PlaneSinkControl<'_, '_, '_>)->Result<(),XcfError>{Ok(())}
    /// Trusted descriptor-bound parse permission only, never a Source grant.
    /// Called after begin validated the actual descriptor, before native decode.
    fn mask_coverage_policy(&self,_:&PlaneDescriptor)->MaskCoveragePolicy{MaskCoveragePolicy::StrictUnit}

    type Plane;
    fn begin(&mut self, descriptor: &PlaneDescriptor, control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError>;
    fn tile(&mut self, x: u32, y: u32, width: u32, height: u32, bytes: &[u8], control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError>;
    fn finish(&mut self, control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<Self::Plane, XcfError>;
    /// Destroy any active staging before parser reservations are refunded.
    /// Completed planes already returned by finish belong to the parsed graph.
    fn abort(&mut self);
}

/// Compatibility collector used by the existing eager decoding entry points.
#[derive(Default)]
pub struct CollectingPlaneSink {
    plane: Option<Plane>,
}
impl PlaneSink for CollectingPlaneSink {
    type Plane = Plane;
    fn begin(&mut self, d: &PlaneDescriptor, control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError> {
        if self.plane.is_some() {
            return Err(XcfError::at(ErrorKind::Structure, d.record_offset, "sink plane already active"));
        }
        let len = usize::try_from(d.width)
            .ok()
            .and_then(|w| usize::try_from(d.height).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|n| n.checked_mul(d.channels))
            .and_then(|n| n.checked_mul(d.sample.source_bytes()))
            .ok_or_else(|| XcfError::at(ErrorKind::Limit, d.record_offset, "collected plane length"))?;
        self.plane = Some(Plane { width: d.width, height: d.height, channels: d.channels, sample: d.sample, bytes: control.allocate_zeroed(len)? });
        Ok(())
    }
    fn tile(&mut self, x: u32, y: u32, width: u32, height: u32, bytes: &[u8], control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError> {
        let plane = self.plane.as_mut().ok_or_else(|| XcfError::at(ErrorKind::Structure, 0, "sink plane missing"))?;
        if x.checked_add(width).is_none_or(|end| end > plane.width) || y.checked_add(height).is_none_or(|end| end > plane.height) {
            return Err(XcfError::at(ErrorKind::Structure, 0, "sink tile extent"));
        }
        let bpp = plane.channels.checked_mul(plane.sample.source_bytes()).ok_or_else(|| XcfError::at(ErrorKind::Limit, 0, "sink pixel stride"))?;
        let row = (width as usize).checked_mul(bpp).ok_or_else(|| XcfError::at(ErrorKind::Limit, 0, "sink row stride"))?;
        if bytes.len() != row.checked_mul(height as usize).ok_or_else(|| XcfError::at(ErrorKind::Limit, 0, "sink tile length"))? {
            return Err(XcfError::at(ErrorKind::Structure, 0, "sink tile byte count"));
        }
        for dy in 0..height as usize {
            let at = ((y as usize + dy) * plane.width as usize + x as usize) * bpp;
            let source = bytes.get(dy * row..(dy + 1) * row).ok_or_else(|| XcfError::at(ErrorKind::Structure, 0, "sink row input"))?;
            let dest = plane.bytes.get_mut(at..at + row).ok_or_else(|| XcfError::at(ErrorKind::Structure, 0, "sink row output"))?;
            for (out, input) in dest.chunks_mut(4096).zip(source.chunks(4096)) {
                // The eager API's decoded-sample work covers conversion and
                // retention; preserve that existing accounting convention.
                control.poll()?;
                out.copy_from_slice(input);
            }
        }
        Ok(())
    }
    fn finish(&mut self, control: &mut PlaneSinkControl<'_, '_, '_>) -> Result<Plane, XcfError> {
        control.poll()?;
        self.plane.take().ok_or_else(|| XcfError::at(ErrorKind::Structure, 0, "sink finish without plane"))
    }
    fn abort(&mut self) {
        self.plane = None;
    }
}

/// Descriptor-only participant for the explicit metadata API. No Surface or
/// decoded native bytes are returned; every actual plane needs later decode.
pub(crate) struct MetadataPlaneSink(Option<PlaneDescriptor>);
impl MetadataPlaneSink{pub(crate) fn new()->Self{Self(None)}}
impl PlaneSink for MetadataPlaneSink{
 type Plane=PlaneDescriptor;
 fn begin(&mut self,d:&PlaneDescriptor,c:&mut PlaneSinkControl<'_, '_, '_>)->Result<(),XcfError>{c.consume_work(32)?;if self.0.is_some(){return Err(XcfError::at(ErrorKind::Structure,d.record_offset,"metadata plane active"));}self.0=Some(d.clone());Ok(())}
 fn tile(&mut self,_:u32,_:u32,_:u32,_:u32,_:&[u8],_:&mut PlaneSinkControl<'_, '_, '_>)->Result<(),XcfError>{Err(XcfError::at(ErrorKind::Structure,0,"metadata plan received pixel payload"))}
 fn finish(&mut self,c:&mut PlaneSinkControl<'_, '_, '_>)->Result<Self::Plane,XcfError>{c.poll()?;self.0.take().ok_or_else(||XcfError::at(ErrorKind::Structure,0,"metadata plane absent"))}
 fn abort(&mut self){self.0=None;}
}
