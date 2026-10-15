//! Single-decode selection between explicit owned source families.
//! Selection consumes original parsed records; a failed family never retries a
//! broader importer. This does not change either existing typed entry point.
use super::owned_plain_source::existing_quota::{Context, PreparationHold};
use super::*;
use photocraft_raster::backing::{BackingError, ReadControl, quota::BackingQuota};
use photocraft_xcf::{BudgetKind, ResourceLimits};
#[path = "owned_xcf_source/promotion.rs"]
mod promotion;
pub(in crate::xcf_import) use promotion::{ParsedPromotion, PromotionSource};
fn invalid(message: &str) -> IoError {
    IoError::Unsupported(message.into())
}
fn backing(error: BackingError) -> IoError {
    let kind = match error {
        BackingError::Cancelled => ErrorKind::Cancelled,
        BackingError::Unsupported => ErrorKind::Unsupported,
        _ => ErrorKind::Limit,
    };
    accounting_error(XcfError { kind, offset: 0, item: String::new(), message: format!("selected owned XCF: {error}") })
}
/// Actual typed owner returned by the selected original-source family. The enum
/// itself is not a persisted admission flag or a source/render authority.
pub enum OwnedXcfImport {
    Plain(owned_plain_source::OwnedPlainXcfImport),
    AnalyticGeometry(owned_analytic_geometry_source::OwnedAnalyticGeometryXcfImport),
    Point(owned_point_source::OwnedPointXcfImport),
    ThresholdAlpha(owned_threshold_source::OwnedThresholdAlphaXcfImport),
    HistogramThreshold(owned_histogram_threshold_source::OwnedHistogramThresholdXcfImport),
}
impl std::fmt::Debug for OwnedXcfImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedXcfImport").field("document_id", &self.document().document().id).finish_non_exhaustive()
    }
}
impl OwnedXcfImport {
    pub fn document(&self) -> &photocraft_doc::accounted_document::AccountedDocument {
        match self {
            Self::AnalyticGeometry(v) => v.document(),
            Self::Plain(v) => v.document(),
            Self::Point(v) => v.document(),
            Self::ThresholdAlpha(v) => v.document(),
            Self::HistogramThreshold(v) => v.document(),
        }
    }
    pub fn warnings(&self) -> &[String] {
        match self {
            Self::AnalyticGeometry(v) => v.warnings(),
            Self::Plain(v) => v.warnings(),
            Self::Point(v) => v.warnings(),
            Self::ThresholdAlpha(v) => v.warnings(),
            Self::HistogramThreshold(v) => v.warnings(),
        }
    }
    pub fn origin(&self) -> ImportOrigin {
        match self {
            Self::AnalyticGeometry(v) => v.origin(),
            Self::Plain(v) => v.origin(),
            Self::Point(v) => v.origin(),
            Self::ThresholdAlpha(v) => v.origin(),
            Self::HistogramThreshold(v) => v.origin(),
        }
    }
    pub fn assessment(&self) -> Option<&crate::xcf_assessment::ImportAssessment> {
        match self {
            Self::AnalyticGeometry(v) => v.assessment(),
            Self::Plain(v) => v.assessment(),
            Self::Point(v) => v.assessment(),
            Self::ThresholdAlpha(v) => v.assessment(),
            Self::HistogramThreshold(v) => v.assessment(),
        }
    }
    pub fn source_read_only(&self) -> bool {
        match self {
            Self::AnalyticGeometry(v) => v.source_read_only(),
            Self::Plain(v) => v.source_read_only(),
            Self::Point(v) => v.source_read_only(),
            Self::ThresholdAlpha(v) => v.source_read_only(),
            Self::HistogramThreshold(v) => v.source_read_only(),
        }
    }
    pub fn preview_only(&self) -> bool {
        match self {
            Self::AnalyticGeometry(v) => v.preview_only(),
            Self::Plain(v) => v.preview_only(),
            Self::Point(v) => v.preview_only(),
            Self::ThresholdAlpha(v) => v.preview_only(),
            Self::HistogramThreshold(v) => v.preview_only(),
        }
    }
    pub fn into_accounted_document(self) -> photocraft_doc::accounted_document::AccountedDocument {
        match self {
            Self::AnalyticGeometry(v) => v.into_accounted_document(),
            Self::Plain(v) => v.into_accounted_document(),
            Self::Point(v) => v.into_accounted_document(),
            Self::ThresholdAlpha(v) => v.into_accounted_document(),
            Self::HistogramThreshold(v) => v.into_accounted_document(),
        }
    }
}
struct PreparationInput {
    encoded: Option<Arc<Vec<u8>>>,
    hold: Option<PreparationHold>,
}
/// Explicit caller-quota memory route. Initial capture remains the caller's
/// responsibility and must keep its fee through this call. A real lowered R/I
/// preparation hold precedes Budget creation; Scratch0 is required. The strict
/// source record chooses one family after the single decode and before mapping.
/// All failure work remains consumed and no family retries another route.
pub fn import_owned_xcf_bytes_in_quota(
    name: &str,
    bytes: Arc<Vec<u8>>,
    options: &DecodeOptions<'_>,
    limits: &ResourceLimits,
    quota: BackingQuota,
    control: &ReadControl<'_>,
) -> Result<OwnedXcfImport, IoError> {
    control.check(1).map_err(backing)?;
    if name.len() > 1 << 20 {
        return Err(invalid("selected owned XCF name ceiling"));
    }
    let effective =
        limits.lowered(&[("residentBytes", limits.get(BudgetKind::Resident).min(options.limits.resident_bytes as u64))]).map_err(accounting_error)?;
    let aggregate = effective.get(BudgetKind::Resident);
    let mut preparation = PreparationInput { encoded: Some(bytes), hold: None };
    preparation.hold = Some(PreparationHold::reserve(&quota, &effective, control)?);
    let cancelled = || control.interrupt.cancelled() || options.cancelled.is_some_and(|f| f());
    let observer = |units| {
        control.check(units).map_err(|error| XcfError {
            kind: if error == BackingError::Cancelled { ErrorKind::Cancelled } else { ErrorKind::Limit },
            offset: 0,
            item: String::new(),
            message: format!("selected owned XCF enclosing work: {error}"),
        })
    };
    let budget = photocraft_xcf::Budget::new_persistent_with_work(effective.clone(), Some(&cancelled), Some(&observer)).map_err(accounting_error)?;
    let source = staged_selected_source::import_staged_selected_bytes_with_budget(
        name,
        preparation.encoded.take().ok_or_else(|| invalid("selected source already moved"))?,
        control.interrupt,
        options,
        &budget,
    )?;
    let family = source.family();
    let context = Context { quota: quota.clone(), control };
    match family {
        staged_selected_source::SelectedStagedFamily::AnalyticGeometry => owned_analytic_geometry_source::promote_source(source, &budget, &effective, aggregate, &context, &mut preparation.hold, control.interrupt).map(OwnedXcfImport::AnalyticGeometry),
        staged_selected_source::SelectedStagedFamily::Point(_) | staged_selected_source::SelectedStagedFamily::ResidualPoint(_) | staged_selected_source::SelectedStagedFamily::ConfiguredPoint(_) | staged_selected_source::SelectedStagedFamily::PointStackEdits | staged_selected_source::SelectedStagedFamily::LegacyScalar(_) | staged_selected_source::SelectedStagedFamily::Neighborhood(_) => owned_point_source::promote_source(source, &budget, quota, control, &mut preparation.hold).map(OwnedXcfImport::Point),
        staged_selected_source::SelectedStagedFamily::Plain => {
            owned_plain_source::promote_source(source, &budget, &effective, aggregate, Some(&context), &mut preparation.hold, control.interrupt)
                .map(OwnedXcfImport::Plain)
        }
        staged_selected_source::SelectedStagedFamily::ThresholdAlpha => {
            owned_threshold_source::promote_source(source, &budget, quota, control, &mut preparation.hold).map(OwnedXcfImport::ThresholdAlpha)
        }
        staged_selected_source::SelectedStagedFamily::Threshold => {
            owned_histogram_threshold_source::promote_source(source, &budget, quota, control, &mut preparation.hold).map(OwnedXcfImport::HistogramThreshold)
        }
    }
}

#[cfg(test)]
#[path = "owned_xcf_source/tests.rs"]
mod tests;
