//! Default bundled rectilinear preparation plus the scoped underline member.
//! Authorized-font imports defer these same checks to their budgeted renderer so
//! all geometry/outline preparation follows its shared prospective admission.
use super::*;

pub(super) fn validate<P>(
    mapped: &crate::xcf_sources::MappedSource,
    owner: &Drawable<P>,
    canvas: Size,
    budget: &mut Budget<'_>,
    shared: Option<&photocraft_xcf::Budget<'_>>,
    ctl: &Interrupt<'_>,
) -> Result<(), IoError> {
    use photocraft_text::xcf::{ExactFont, TextSourceError};
    let resource = mapped.resources.first().ok_or_else(|| IoError::Unsupported("missing text font identity".into()))?;
    let origin = [owner.offset.0, owner.offset.1];
    // Both requests are bounded stack-only descriptors; boxing would add an uncharged allocation.
    #[allow(clippy::large_enum_variant)]
    enum Request<'a> {
        Rectilinear(photocraft_text::xcf::QualifiedRectilinearText<'a>),
        Underline(photocraft_text::xcf::QualifiedUnderlinedText<'a>),
    }
    let request = match photocraft_text::xcf::qualify_placed_layout_colored_rectilinear_source(&mapped.item, resource, origin, ctl) {
        Ok(qualified) => Request::Rectilinear(qualified),
        Err(TextSourceError::Unsupported(_)) if canvas == Size::new(128, 64) && origin == [8, 4] && matches!(owner.height, 20 | 39) => {
            Request::Underline(photocraft_text::xcf::qualify_underlined_source(&mapped.item, resource, origin, ctl).map_err(|error| text_error(error, ctl))?)
        }
        Err(error) => return Err(text_error(error, ctl)),
    };
    let font = ExactFont::bundled_inter(ctl).map_err(|error| text_error(error, ctl))?;
    if let Request::Rectilinear(qualified) = &request {
        photocraft_text::xcf::qualify_layout_rectilinear_geometry(&font, *qualified, ctl).map_err(|error| text_error(error, ctl))?;
    }
    let denied = std::cell::RefCell::new(None);
    let mut temporary_bytes = 0_usize;
    let mut resident = |bytes| {
        let result = (|| {
            let bytes = usize::try_from(bytes).map_err(|_| IoError::Unsupported("XCF text preparation exceeds address space".into()))?;
            temporary_bytes = temporary_bytes.checked_add(bytes).ok_or_else(|| IoError::Unsupported("XCF text preparation accounting overflow".into()))?;
            budget.charge(bytes)
        })();
        result.map_err(|error| remember(&denied, error))
    };
    let mut work = |units| {
        ctl.check().map_err(|_| TextSourceError::Cancelled)?;
        if let Some(shared) = shared {
            shared.consume_work(units, "XCF underline preparation").map_err(|error| remember(&denied, accounting_error(error)))?;
        }
        Ok(())
    };
    let prepared = match request {
        Request::Rectilinear(qualified) => photocraft_text::xcf::prepare_qualified_rectilinear_with_charge(&font, qualified, 8 << 20, ctl, &mut resident),
        Request::Underline(qualified) => {
            photocraft_text::xcf::underline::prepare_underlined_with_charge(&font, qualified, 8 << 20, ctl, &mut resident, &mut work)
        }
    };
    if let Some(error) = denied.into_inner() {
        return Err(error);
    }
    {
        let _prepared = prepared.map_err(|error| text_error(error, ctl))?;
    }
    budget.release(temporary_bytes)
}
fn text_error(error: photocraft_text::xcf::TextSourceError, ctl: &Interrupt<'_>) -> IoError {
    if ctl.cancelled() { IoError::Cancelled } else { IoError::Unsupported(error.to_string()) }
}
fn remember(cell: &std::cell::RefCell<Option<IoError>>, error: IoError) -> photocraft_text::xcf::TextSourceError {
    if let Ok(mut slot) = cell.try_borrow_mut() {
        *slot = Some(error);
    }
    photocraft_text::xcf::TextSourceError::Budget
}
