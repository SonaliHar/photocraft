//! Current fidelity after actual semantic qualification; no resource locator is read.
use crate::IoError;
use photocraft_doc::Document;
use photocraft_doc::xcf_items::EditableSource;
use photocraft_doc::xcf_resources::{ResourceFidelity, ResourceKind};
use photocraft_raster::Interrupt;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    Full,
    Reduced,
    DependencyLimited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyStatus {
    ExactAvailable,
    Unresolved,
    PendingActivation,
    Substituted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyAssessment {
    pub resource_id: String,
    pub kind: &'static str,
    pub status: DependencyStatus,
    pub consumer_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessDiagnostic {
    pub code: &'static str,
    pub severity: &'static str,
    pub resource_id: String,
    pub message: &'static str,
    pub affected_capability: &'static str,
    pub recovery_actions: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAssessment {
    pub fidelity: Fidelity,
    pub source_format: &'static str,
    pub source_version: u32,
    pub container: String,
    pub diagnostics: Vec<SuccessDiagnostic>,
    pub dependencies: Vec<DependencyAssessment>,
}

/// Reevaluate retained current semantics, not a persisted success flag. A
/// caller-budgeted scene must use its actual prepared import job instead.
pub fn assess_xcf_document(doc: &Document, control: &Interrupt<'_>) -> Result<Option<ImportAssessment>, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    if doc.xcf_state.as_ref().is_none_or(|state| state.provenance.format != "xcf") {
        return Ok(None);
    }
    use photocraft_compose::gimp_render::PreparedRenderer;
    // Membership selects an accounting constructor; failed qualification never
    // retries through compatibility rendering or reuses an old success flag.
    if PreparedRenderer::supports_plain_normal_budgeted(doc)
        || PreparedRenderer::supports_plain_point_budgeted(doc)
        || PreparedRenderer::requires_preparation_budget(doc)
        || PreparedRenderer::supports_accounted_scalar_preparation(doc)
    {
        return crate::xcf_current_assessment::assess(doc, photocraft_format::PreparationLimits::default(), control);
    }
    // Existing legacy/source consumers retain their qualified compatibility
    // route. This does not claim aggregate preparation accounting for them.
    allocation_bound(doc)?;
    let prepared = PreparedRenderer::new_with_interrupt(doc, control)
        .map_err(|error| if control.cancelled() { IoError::Cancelled } else { IoError::Unsupported(format!("current XCF assessment: {error}")) })?;
    assess_prepared_xcf_document(&prepared, control)
}

/// Assess the exact immutable document already qualified by a real renderer.
/// This also supports caller-budgeted ordinary scenes without an untrusted
/// boolean or a persisted claim of qualification.
pub fn assess_prepared_xcf_document(
    prepared: &photocraft_compose::gimp_render::PreparedRenderer<'_>,
    control: &Interrupt<'_>,
) -> Result<Option<ImportAssessment>, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    let doc = prepared.document();
    if doc.xcf_state.as_ref().is_none_or(|state| state.provenance.format != "xcf") {
        return Ok(None);
    }
    after_qualified_import(doc, control).map(Some)
}

/// Crate-private: invoked only after the mapper's real renderer/source
/// qualification, including its accounted ordinary-scene preparation.
pub(crate) fn allocation_bound(doc: &Document) -> Result<usize, IoError> {
    let state = doc.xcf_state.as_ref().ok_or_else(|| IoError::Unsupported("XCF assessment requires retained source state".into()))?;
    if state.resources.len() > 1000
        || state.provenance.container.len() > 64
        || state.resources.len().checked_mul(state.source_items.len()).is_none_or(|work| work > 1_000_000)
    {
        return Err(IoError::Unsupported("XCF assessment output/work limit exceeded".into()));
    }
    // Two exact decimal U64 strings and two recovery-action slots per row;
    // diagnostic prose and kind/code fields are static, never source locators.
    let row = std::mem::size_of::<DependencyAssessment>()
        .checked_add(std::mem::size_of::<SuccessDiagnostic>())
        .and_then(|bytes| bytes.checked_add(40 + 2 * std::mem::size_of::<&str>()))
        .ok_or_else(|| IoError::Unsupported("XCF assessment size overflow".into()))?;
    state
        .resources
        .len()
        .checked_mul(row)
        .and_then(|bytes| bytes.checked_add(64 + std::mem::size_of::<ImportAssessment>()))
        .ok_or_else(|| IoError::Unsupported("XCF assessment size overflow".into()))
}
pub(crate) fn after_qualified_import(doc: &Document, control: &Interrupt<'_>) -> Result<ImportAssessment, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    doc.validate_xcf_state_with(control)
        .map_err(|error| if control.cancelled() { IoError::Cancelled } else { IoError::Unsupported(format!("XCF assessment state: {error}")) })?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| IoError::Unsupported("XCF assessment requires retained source state".into()))?;
    if state.provenance.format != "xcf" || state.provenance.format_version > 26 || matches!(state.provenance.format_version, 4..=6) {
        return Err(IoError::Unsupported("unsupported XCF assessment source epoch".into()));
    }
    allocation_bound(doc)?;
    let mut dependencies = Vec::new();
    let mut diagnostics = Vec::new();
    dependencies.try_reserve_exact(state.resources.len()).map_err(|_| IoError::Unsupported("XCF assessment allocation denied".into()))?;
    diagnostics.try_reserve_exact(state.resources.len()).map_err(|_| IoError::Unsupported("XCF assessment allocation denied".into()))?;
    let mut fidelity = Fidelity::Full;
    for resource in &state.resources {
        control.check().map_err(|_| IoError::Cancelled)?;
        let kind = match resource.kind {
            ResourceKind::Font => "font",
            ResourceKind::Pattern => "pattern",
            ResourceKind::Gradient => "gradient",
            ResourceKind::Brush => "brush",
            ResourceKind::Palette => "palette",
            ResourceKind::ColorProfile => "color-profile",
            ResourceKind::LinkedImage => "linked-image",
        };
        // Byte recovery and current consumer activation are separate. The
        // renderer already qualified every source, including hidden owners.
        let mut linked = 0usize;
        let mut linked_active = true;
        let mut exact_font = false;
        for source in &state.source_items {
            control.check().map_err(|_| IoError::Cancelled)?;
            match &source.source {
                EditableSource::Linked { resource: id, last_verified_hash, .. } if *id == resource.id => {
                    linked = linked.checked_add(1).ok_or_else(|| IoError::Unsupported("XCF assessment work limit".into()))?;
                    linked_active &= last_verified_hash
                        .as_deref()
                        .is_some_and(|hash| resource.resolution.as_ref().is_some_and(|resolution| resolution.payload_hash == hash));
                }
                // A declared modified source already proved its retained raster
                // authority. Its font is an inactive editing dependency, not an
                // exact regeneration consumer to activate during assessment.
                EditableSource::Text { font, source_modified: false, .. } if *font == resource.id && resource.kind == ResourceKind::Font => {
                    photocraft_compose::gimp_text_items::qualify_current_source(doc, source, control).map_err(|error| {
                        if control.cancelled() { IoError::Cancelled } else { IoError::Unsupported(format!("XCF exact-font assessment: {error}")) }
                    })?;
                    exact_font = true;
                }
                _ => {}
            }
        }
        let owned_exact = resource.owned_bytes.is_some() && resource.content_hash.is_some();
        let consumer_available = if resource.kind == ResourceKind::LinkedImage {
            linked != 0 && linked_active
        } else if resource.kind == ResourceKind::Font {
            exact_font
        } else {
            // A type-specific resource evaluator must prove activation before
            // any future recovery service can promote these dependencies.
            false
        };
        let substituted = resource.resolution.as_ref().is_some_and(|resolution| resolution.fidelity == ResourceFidelity::Substituted);
        let status = if substituted {
            fidelity = Fidelity::Reduced;
            DependencyStatus::Substituted
        } else if consumer_available && (resource.resolution.as_ref().is_some_and(|r| r.fidelity == ResourceFidelity::Exact) || owned_exact || exact_font) {
            DependencyStatus::ExactAvailable
        } else if resource.owned_bytes.is_some() {
            if fidelity != Fidelity::Reduced {
                fidelity = Fidelity::DependencyLimited;
            }
            DependencyStatus::PendingActivation
        } else {
            if fidelity != Fidelity::Reduced {
                fidelity = Fidelity::DependencyLimited;
            }
            DependencyStatus::Unresolved
        };
        if status != DependencyStatus::ExactAvailable {
            let (code, message) = match status {
                DependencyStatus::Unresolved => ("missing_resource", "A retained dependency is unavailable; its stored locator grants no read permission."),
                DependencyStatus::PendingActivation => ("resource_not_activated", "Supplied bytes have not established an exact active resource consumer."),
                DependencyStatus::Substituted => ("substituted_resource", "An explicitly substituted dependency lowers this project's fidelity."),
                DependencyStatus::ExactAvailable => return Err(IoError::Unsupported("inconsistent XCF assessment diagnostic".into())),
            };
            let recovery_actions = if resource.kind == ResourceKind::LinkedImage { vec!["xcf.resource.upload", "xcf.link.refresh"] } else { Vec::new() };
            diagnostics.push(SuccessDiagnostic {
                code,
                severity: "warning",
                resource_id: resource.id.0.to_string(),
                message,
                affected_capability: if resource.kind == ResourceKind::LinkedImage { "linked_image_regeneration" } else { "resource_editing" },
                recovery_actions,
            });
        }
        dependencies.push(DependencyAssessment { resource_id: resource.id.0.to_string(), kind, status, consumer_available });
    }
    control.check().map_err(|_| IoError::Cancelled)?;
    Ok(ImportAssessment {
        fidelity,
        source_format: "xcf",
        source_version: state.provenance.format_version,
        container: state.provenance.container.clone(),
        diagnostics,
        dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, SampleType, Size, item_identity::ResourceId, xcf_resources::*, xcf_state::XcfState};

    fn document() -> Document {
        let mut doc = Document::new("assessment", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let mut state = XcfState::default();
        state.provenance.format = "xcf".into();
        state.provenance.format_version = 22;
        state.provenance.container = "plain".into();
        state.resources.push(ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "inert".into(),
            collection: None,
            internal: false,
            locator: Some("../never-read-private.png".into()),
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        });
        doc.xcf_state = Some(state);
        doc
    }

    #[test]
    fn missing_and_identity_only_recovery_never_claim_full_or_activation() {
        let mut doc = document();
        let before = doc.clone();
        let report = after_qualified_import(&doc, &Interrupt::NONE).unwrap();
        assert_eq!(report.fidelity, Fidelity::DependencyLimited);
        assert_eq!(report.dependencies[0].status, DependencyStatus::Unresolved);
        assert!(!report.dependencies[0].consumer_available);
        assert_eq!(report.diagnostics[0].code, "missing_resource");
        assert_eq!(doc, before);
        let resource = &mut doc.xcf_state.as_mut().unwrap().resources[0];
        let payload = vec![1, 2, 3];
        let hash = blake3::hash(&payload).to_hex().to_string();
        resource.content_hash = Some(hash.clone());
        resource.owned_bytes = Some(std::sync::Arc::new(payload));
        resource.resolution = Some(ResourceResolution { fidelity: ResourceFidelity::Exact, payload_hash: hash });
        let report = after_qualified_import(&doc, &Interrupt::NONE).unwrap();
        assert_eq!(report.fidelity, Fidelity::DependencyLimited);
        assert_eq!(report.dependencies[0].status, DependencyStatus::PendingActivation);
        assert!(!report.dependencies[0].consumer_available);
    }

    #[test]
    fn explicit_substitution_is_reduced_even_before_refresh_and_hash_mismatch_fails() {
        let mut doc = document();
        let resource = &mut doc.xcf_state.as_mut().unwrap().resources[0];
        let payload = vec![1, 2, 3];
        resource.resolution = Some(ResourceResolution { fidelity: ResourceFidelity::Substituted, payload_hash: blake3::hash(&payload).to_hex().to_string() });
        resource.owned_bytes = Some(std::sync::Arc::new(payload));
        let report = after_qualified_import(&doc, &Interrupt::NONE).unwrap();
        assert_eq!(report.fidelity, Fidelity::Reduced);
        assert_eq!(report.dependencies[0].status, DependencyStatus::Substituted);
        let original = doc.clone();
        assert!(std::sync::Arc::ptr_eq(
            original.xcf_state.as_ref().unwrap().resources[0].owned_bytes.as_ref().unwrap(),
            doc.xcf_state.as_ref().unwrap().resources[0].owned_bytes.as_ref().unwrap()
        ));
        std::sync::Arc::make_mut(doc.xcf_state.as_mut().unwrap().resources[0].owned_bytes.as_mut().unwrap())[0] ^= 1;
        assert_eq!(original.xcf_state.as_ref().unwrap().resources[0].owned_bytes.as_ref().unwrap().as_slice(), &[1, 2, 3]);
        assert!(!std::sync::Arc::ptr_eq(
            original.xcf_state.as_ref().unwrap().resources[0].owned_bytes.as_ref().unwrap(),
            doc.xcf_state.as_ref().unwrap().resources[0].owned_bytes.as_ref().unwrap()
        ));
        assert!(after_qualified_import(&doc, &Interrupt::NONE).is_err());
    }

    #[test]
    fn non_xcf_absence_and_cancel_do_not_publish_a_fidelity_result() {
        let doc = Document::new("ordinary", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        assert_eq!(assess_xcf_document(&doc, &Interrupt::NONE).unwrap(), None);
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(matches!(after_qualified_import(&document(), &cancelled), Err(IoError::Cancelled)));
    }
}
