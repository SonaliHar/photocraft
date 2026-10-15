//! Typed publication of the opt-in owned XCF import, including runtime source
//! protection. The ordinary open service retains its color-policy decisions.
use crate::{DocState, EngineError, Result, Session, source_policy::SourceIdentity};
use photocraft_io::OwnedPlainXcfImport;
use photocraft_raster::backing::{BackingError, ReadControl};

#[path = "accounted_xcf_open/analytic_geometry.rs"]
pub(crate) mod analytic_geometry;
#[path = "accounted_xcf_open/isolated_normal_group.rs"]
pub(crate) mod isolated_normal_group;
#[path = "accounted_xcf_open/gaussian_group.rs"]
pub(crate) mod gaussian_group;

struct ImportedModel<'a> {
    document: &'a photocraft_doc::accounted_document::AccountedDocument,
    origin: photocraft_io::ImportOrigin,
    source_read_only: bool,
    preview_only: bool,
}

fn text(value: &str, control: &ReadControl<'_>) -> Result<String> {
    let mut copy = String::new();
    copy.try_reserve_exact(value.len()).map_err(|_| BackingError::Allocation)?;
    if copy.capacity() > value.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for part in value.as_bytes().chunks(4096) {
        control.check(part.len() as u64)?;
    }
    // UTF-8 text is copied once, after the caller's prospective work admission.
    copy.push_str(value);
    Ok(copy)
}

impl Session {
    /// Lower-level counterpart of `add_accounted_document` for the exact owned
    /// plain-XCF result. Shares its actual model/source owner, retains source
    /// exclusion even for a renamed input, and stages everything before tab
    /// publication. A supplied path is an inert identity from the open service;
    /// this function performs no filesystem access or resource authorization.
    ///
    /// This preserves the imported current interpretation. An open service must
    /// resolve color-policy questions/conversions separately before choosing
    /// this explicit endpoint. Default opens and native-large policy are unchanged.
    pub fn add_owned_plain_xcf_import(
        &mut self,
        imported: &OwnedPlainXcfImport,
        label: &str,
        source_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        if crate::accounted_native_open::is_current_isolated_normal_group_candidate(imported.document().document(), control)?
            || crate::accounted_native_open::is_current_isolated_gaussian_group_candidate(imported.document().document(), control)? {
            return Err(BackingError::Unsupported.into());
        }
        self.add_owned_xcf_model(
            ImportedModel {
                document: imported.document(),
                origin: imported.origin(),
                source_read_only: imported.source_read_only(),
                preview_only: imported.preview_only(),
            },
            label,
            source_path,
            control,
        )
    }

    /// Publish a genuinely retained qualified Threshold Alpha import. The
    /// current recipe and complete operative band must succeed before a tab,
    /// history or runtime source identity is installed. Original encoded source
    /// ownership remains with the import; persisted locators grant no access.
    pub fn add_owned_threshold_alpha_xcf_import(
        &mut self,
        imported: &photocraft_io::OwnedThresholdAlphaXcfImport,
        label: &str,
        source_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        control.check(1)?;
        let owner = imported.document();
        let builtin = match owner.document().mode {
            photocraft_doc::ColorMode::Rgb => photocraft_cms::Builtin::Srgb,
            photocraft_doc::ColorMode::Grayscale => photocraft_cms::Builtin::SGray,
            _ => return Err(BackingError::Unsupported.into()),
        };
        let profile = builtin.profile();
        control.check(1)?;
        let prepared =
            photocraft_compose::backed_threshold_alpha_context::PreparedBackedThresholdAlphaContext::new(owner, profile, Default::default(), control).map_err(
                |error| match error {
                    photocraft_compose::backed_normal::BackedNormalError::Backing(error) => EngineError::from(error),
                    other => EngineError::Other(format!("owned Threshold Alpha current recipe: {other}")),
                },
            )?;
        prepared.validate_operative(control).map_err(|error| match error {
            photocraft_compose::backed_point_filter::BackedPointError::Backing(error) => EngineError::from(error),
            other => EngineError::Other(format!("owned Threshold Alpha operative band: {other}")),
        })?;
        drop(prepared);
        self.add_owned_xcf_model(
            ImportedModel { document: owner, origin: imported.origin(), source_read_only: imported.source_read_only(), preview_only: imported.preview_only() },
            label,
            source_path,
            control,
        )
    }

    /// Publish only a genuinely retained Histogram Threshold source import,
    /// after the separate current profile, mask and native recipe has succeeded.
    pub fn add_owned_histogram_threshold_xcf_import(
        &mut self,
        imported: &photocraft_io::OwnedHistogramThresholdXcfImport,
        label: &str,
        source_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        let owner = imported.document();
        crate::accounted_native_open::validate_current_threshold_storage(owner, control)?;
        self.add_owned_xcf_model(
            ImportedModel { document: owner, origin: imported.origin(), source_read_only: imported.source_read_only(), preview_only: imported.preview_only() },
            label,
            source_path,
            control,
        )
    }

    /// Publish a sealed source-selected Point import only after the separate
    /// current profile, mask and complete operative recipe have succeeded.
    pub fn add_owned_point_xcf_import(
        &mut self,
        imported: &photocraft_io::OwnedPointXcfImport,
        label: &str,
        source_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        let owner = imported.document();
        crate::accounted_native_open::validate_current_owned_filter_storage(owner, control)?;
        self.add_owned_xcf_model(
            ImportedModel { document: owner, origin: imported.origin(), source_read_only: imported.source_read_only(), preview_only: imported.preview_only() },
            label,
            source_path,
            control,
        )
    }

    fn add_owned_xcf_model(&mut self, imported: ImportedModel<'_>, label: &str, source_path: Option<&str>, control: &ReadControl<'_>) -> Result<usize> {
        control.check(1)?;
        if imported.origin != photocraft_io::ImportOrigin::xcf() || imported.preview_only {
            return Err(BackingError::Unsupported.into());
        }
        let path_len = source_path.map_or(0, str::len);
        if label.len() > 4096 || path_len > 4096 {
            return Err(BackingError::Length.into());
        }
        let input_len = label.len().checked_add(path_len).ok_or(BackingError::Length)?;
        control.check(input_len as u64)?;
        if label.contains('\0') || source_path.is_some_and(|p| p.contains('\0')) {
            return Err(BackingError::Length.into());
        }
        let document = imported.document;
        if self.docs.len() >= 4096 {
            return Err(BackingError::Length.into());
        }
        for states in self.docs.chunks(64) {
            control.check(states.len() as u64)?;
            if states.iter().any(|state| state.doc.id == document.document().id) {
                return Err(EngineError::Other("an open document already owns this accounted identity".into()));
            }
        }
        let history_states = self.prefs.get().performance.history_states.max(1) as usize;
        let mut state = DocState::new_accounted(document.clone(), None, history_states, control)?;
        // SourceIdentity's inline fields are already included in DocState. Pay
        // new String capacities before either allocation, retaining that charge
        // with the source fields on every later failure and through tab lifetime.
        let metadata = state._accounted_metadata.as_mut().ok_or(BackingError::Integrity)?;
        let previous = metadata.bytes();
        let envelope = (input_len as u64).checked_mul(2).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
        metadata.try_resize(previous.checked_add(envelope).ok_or(BackingError::Length)?).map_err(|_| BackingError::Quota)?;
        let label = text(label, control)?;
        let path = source_path.map(|p| text(p, control)).transpose()?;
        let actual = label.capacity().checked_add(path.as_ref().map_or(0, String::capacity)).ok_or(BackingError::Length)?;
        metadata
            .reconcile_capacity(previous.checked_add(actual as u64).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?)
            .map_err(|_| BackingError::Quota)?;
        state.source = Some(SourceIdentity::new(label, path, imported.origin));
        state.source_read_only = imported.source_read_only || imported.origin.is_read_only();
        control.check(1)?;
        self.publish_document_state(state, control)
    }
}

#[cfg(test)]
#[path = "accounted_xcf_open_tests.rs"]
mod tests;
#[cfg(all(test, feature = "corpus"))]
#[path = "accounted_threshold_alpha_xcf_open_tests.rs"]
mod threshold_alpha_tests;
