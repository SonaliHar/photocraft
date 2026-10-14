//! Bounded runtime-only exact external font input for existing text candidates.
//! Supplied bytes are caller-authorized, immutable and borrowed. No file read,
//! locator/name resolution, global registration, Document field or serialization
//! occurs. This narrows only the already supported Inter Regular face authority.
use super::{ExactFont, INTER_SHA, TextSourceError, poll};
use photocraft_raster::Interrupt;
type Result<T> = std::result::Result<T, TextSourceError>;
const INTER_BYTES: usize = 411640;
/// SHA identifies the entire supplied font payload; index identifies its face.
/// Names and a saved resource locator are not font authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceIdentity {
    pub sha256: [u8; 32],
    pub face_index: u32,
}
impl FaceIdentity {
    pub fn inter_regular() -> Self {
        Self { sha256: INTER_SHA, face_index: 0 }
    }
}
#[derive(Clone, Copy)]
pub struct RuntimeInput<'a> {
    pub bytes: &'a [u8],
    pub identity: FaceIdentity,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: 1 << 20, work_units: 1_000_000 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub borrowed_input_bytes: u64,
    pub scratch_reserve_bytes: u64,
    pub peak_bytes: u64,
    /// One unit per supplied byte plus a bounded metadata/finalization reserve.
    pub work_units: u64,
}
/// Opaque verified runtime handle. Borrowing prevents safe caller mutation of
/// the byte region while candidates consume it. No persistent font grant exists.
pub struct PreparedRuntimeFont<'a> {
    font: ExactFont<'a>,
    identity: FaceIdentity,
    requirements: Requirements,
}
impl<'a> PreparedRuntimeFont<'a> {
    /// Direct input to existing fixed-box and binary-outline render candidates.
    /// Those render APIs retain their own lower limits and source gates; their
    /// conservative font residency accounting may charge this input again.
    pub fn exact_font(&self) -> &ExactFont<'a> {
        &self.font
    }
    pub fn identity(&self) -> FaceIdentity {
        self.identity
    }
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }
}
/// Allocation-free admission before any hashing/parsing or callback charge.
/// Unsupported faces do not become valid by raising caller limits.
pub fn requirements(input: RuntimeInput<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Requirements> {
    poll(control)?;
    if input.bytes.len() != INTER_BYTES || input.identity != FaceIdentity::inter_regular() {
        return Err(TextSourceError::FontIdentity);
    }
    let defaults = Limits::default();
    let borrowed_input_bytes = u64::try_from(input.bytes.len()).map_err(|_| TextSourceError::Budget)?;
    // Existing authorization hashes in4096-byte chunks without a font copy.
    // Includes a conservative scalar hash/identity/handle/stack reserve rather
    // than pretending the caller-owned font allocation has no resident cost.
    let scratch_reserve_bytes = 4096u64
        .checked_add(u64::try_from(std::mem::size_of::<sha2::Sha256>() + std::mem::size_of::<PreparedRuntimeFont<'_>>()).map_err(|_| TextSourceError::Budget)?)
        .ok_or(TextSourceError::Budget)?;
    let peak_bytes = borrowed_input_bytes.checked_add(scratch_reserve_bytes).ok_or(TextSourceError::Budget)?;
    let work_units = borrowed_input_bytes.checked_add(1024).ok_or(TextSourceError::Budget)?;
    if limits.resident_bytes > defaults.resident_bytes
        || limits.work_units > defaults.work_units
        || peak_bytes > limits.resident_bytes
        || work_units > limits.work_units
    {
        return Err(TextSourceError::Budget);
    }
    poll(control)?;
    Ok(Requirements { borrowed_input_bytes, scratch_reserve_bytes, peak_bytes, work_units })
}
/// Reuses the existing SHA verifier, including its per4096-byte cancellation.
/// Charges are monotonic caller leases: a later refusal does not refund earlier
/// work/residency. Only a successful final token fence publishes the handle.
pub fn prepare_with_charge<'a>(
    input: RuntimeInput<'a>,
    limits: Limits,
    control: &Interrupt<'_>,
    resident: &mut dyn FnMut(u64) -> Result<()>,
    work: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<PreparedRuntimeFont<'a>> {
    let req = requirements(input, limits, control)?;
    resident(req.peak_bytes)?;
    work(req.work_units)?;
    poll(control)?;
    let font = ExactFont::authorize(input.bytes, &input.identity.sha256, control)?;
    poll(control)?;
    Ok(PreparedRuntimeFont { font, identity: input.identity, requirements: req })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bad_identity_and_lower_budget_do_not_hash_or_charge() {
        let mut charged = 0;
        let bytes = vec![0; INTER_BYTES];
        let mut identity = FaceIdentity::inter_regular();
        identity.face_index = 1;
        assert!(
            prepare_with_charge(
                RuntimeInput { bytes: &bytes, identity },
                Limits::default(),
                &Interrupt::NONE,
                &mut |_| {
                    charged += 1;
                    Ok(())
                },
                &mut |_| Ok(())
            )
            .is_err()
        );
        assert_eq!(charged, 0);
        assert!(
            requirements(
                RuntimeInput { bytes: &bytes, identity: FaceIdentity::inter_regular() },
                Limits { resident_bytes: 1, ..Limits::default() },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(
            requirements(
                RuntimeInput { bytes: &bytes, identity: FaceIdentity::inter_regular() },
                Limits { work_units: 1, ..Limits::default() },
                &Interrupt::NONE
            )
            .is_err()
        );
    }
    #[test]
    fn input_is_not_a_name_or_hash_only_grant() {
        let bytes = vec![0; INTER_BYTES];
        assert!(
            prepare_with_charge(
                RuntimeInput { bytes: &bytes, identity: FaceIdentity::inter_regular() },
                Limits::default(),
                &Interrupt::NONE,
                &mut |_| Ok(()),
                &mut |_| Ok(())
            )
            .is_err()
        );
        assert!(
            requirements(RuntimeInput { bytes: &bytes, identity: FaceIdentity::inter_regular() }, Limits::default(), &Interrupt::cancel_only(&|| true))
                .is_err()
        );
    }
    #[test]
    fn existing_authorized_face_handle_reaches_candidates_without_copy() {
        let input = RuntimeInput { bytes: &crate::fonts::INTER_REGULAR, identity: FaceIdentity::inter_regular() };
        let font = prepare_with_charge(input, Limits::default(), &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).expect("existing exact face");
        assert_eq!(font.identity(), FaceIdentity::inter_regular());
        assert!(std::ptr::eq(font.exact_font().bytes, input.bytes));
        assert_eq!(font.requirements().borrowed_input_bytes, INTER_BYTES as u64);
    }
}
