//! Readonly masked actual-class facade for a paid filtered flat Normal/Union stack.
use crate::{
    backed_filtered_flat_point_context::Context,
    backed_normal::{BackedNormalError, NormalBandLimits},
    backed_normal_stack::OwnedNormalStackBand,
};
use photocraft_cms::Profile;
use photocraft_doc::{SampleEncoding, accounted_document::AccountedDocument};
use photocraft_geom::Rect;
use photocraft_raster::backing::{
    ReadControl,
    quota::{QuotaKind, QuotaLease},
};
pub struct PreparedBackedMaskedFilteredFlatPointDocument<'a> {
    context: Context<'a>,
    _lease: QuotaLease,
}
impl<'a> PreparedBackedMaskedFilteredFlatPointDocument<'a> {
    pub fn new(owner: &'a AccountedDocument, limits: NormalBandLimits, control: &ReadControl<'_>) -> Result<Self, BackedNormalError> {
        control.check(1).map_err(BackedNormalError::Backing)?;
        let header = u64::try_from(std::mem::size_of::<Self>()).map_err(|_| BackedNormalError::Invalid("filtered flat facade header"))?;
        let lease = owner.quota().reserve(QuotaKind::Index, header).map_err(BackedNormalError::Quota)?;
        let context = Context::new_masked(owner, limits, control)?;
        control.check(0).map_err(BackedNormalError::Backing)?;
        Ok(Self { context, _lease: lease })
    }
    pub fn bounds(&self) -> Rect {
        self.context.bounds()
    }
    pub fn current_profile(&self) -> &Profile {
        self.context.current_profile()
    }
    pub fn sample_encoding(&self) -> SampleEncoding {
        self.context.sample_encoding()
    }
    pub fn validate_operative(&self, control: &ReadControl<'_>) -> Result<(), BackedNormalError> {
        self.context.validate_operative(control)
    }
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNormalStackBand, BackedNormalError> {
        self.context.render_band(rect, control)
    }
}
