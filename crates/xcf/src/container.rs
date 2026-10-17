//! One strict transport wrapper; decoded bytes are still untrusted XCF syntax.
use crate::*;
use std::{borrow::Cow, cell::RefCell, io};
#[path = "decode_control.rs"]
mod decode_control;
#[path = "container_prefix.rs"]
mod prefix;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceContainer {
    Plain,
    Gzip,
    Bzip2,
    Xz,
}
/// Content classification only. Wrapper magic does not prove its logical payload is XCF.
pub fn classify_container(bytes: &[u8]) -> Option<SourceContainer> {
    if bytes.starts_with(b"gimp xcf ") {
        Some(SourceContainer::Plain)
    } else if bytes.starts_with(&[31, 139]) {
        Some(SourceContainer::Gzip)
    } else if bytes.starts_with(b"BZh") {
        Some(SourceContainer::Bzip2)
    } else if bytes.starts_with(&[253, 55, 122, 88, 90, 0]) {
        Some(SourceContainer::Xz)
    } else {
        None
    }
}
#[derive(Debug)]
pub struct DecodedImport<P = Plane> {
    pub file: XcfFile<P>,
    pub container: SourceContainer,
    pub encoded_bytes: u64,
    pub logical_bytes: u64,
}
struct Control<'a, 'b, 'l> {
    budget: &'a Budget<'b>,
    limits: &'l ResourceLimits,
    state: RefCell<Reservation<'a, 'b>>,
    index: RefCell<Reservation<'a, 'b>>,
    output: RefCell<Reservation<'a, 'b>>,
    failure: RefCell<Option<XcfError>>,
}
impl Control<'_, '_, '_> {
    fn io(&self, result: Result<(), XcfError>) -> io::Result<()> {
        result.map_err(|error| {
            if let Ok(mut failure) = self.failure.try_borrow_mut() {
                *failure = Some(error.clone());
            }
            io::Error::other(error.to_string())
        })
    }
    fn peak(&self, kind: BudgetKind, n: usize) -> io::Result<()> {
        let slot = match kind {
            BudgetKind::Decoder => &self.state,
            BudgetKind::Index => &self.index,
            _ => &self.output,
        };
        let mut lease = slot.try_borrow_mut().map_err(|_| io::Error::other("wrapper reservation already borrowed"))?;
        let old = lease.bytes();
        let component =
            self.budget.used(kind).checked_sub(old).and_then(|used| used.checked_add(n as u64)).ok_or_else(|| io::Error::other("component peak overflow"))?;
        self.io(self.budget.check(kind, component, "wrapper allocation"))?;
        if component > self.limits.get(kind) {
            return Err(io::Error::other("wrapper component budget exceeded"));
        }
        let resident = self
            .budget
            .used(BudgetKind::Resident)
            .checked_sub(old)
            .and_then(|v| v.checked_add(n as u64))
            .ok_or_else(|| io::Error::other("resident peak overflow"))?;
        self.io(self.budget.check(BudgetKind::Resident, resident, "wrapper peak"))?;
        if resident > self.limits.get(BudgetKind::Resident) {
            return Err(io::Error::other("wrapper resident budget exceeded"));
        }
        self.io(lease.resize(n as u64, "wrapper live allocation"))?;
        Ok(())
    }
    fn error(&self, error: io::Error) -> XcfError {
        if let Ok(mut failure) = self.failure.try_borrow_mut()
            && let Some(error) = failure.take()
        {
            return error;
        }
        let message = error.to_string();
        XcfError::new(
            if message.contains("budget") {
                ErrorKind::Limit
            } else if message.contains("allocation") {
                ErrorKind::Allocation
            } else {
                ErrorKind::Compression
            },
            0,
            format!("{} wrapper decode: {message}", self.budget.limits().policy().name()),
        )
    }
}
fn usize_limit(limits: &ResourceLimits, kind: BudgetKind) -> Result<usize, XcfError> {
    usize::try_from(limits.get(kind)).map_err(|_| XcfError::new(ErrorKind::Limit, 0, format!("{} exceeds address space", kind.name())))
}
fn unwrap<'a, 'b, 'c>(
    bytes: &'a [u8],
    kind: SourceContainer,
    budget: &'b Budget<'c>,
    limits: &ResourceLimits,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<(Cow<'a, [u8]>, Option<Reservation<'b, 'c>>), XcfError> {
    budget.check(BudgetKind::Encoded, bytes.len() as u64, "encoded source")?;
    if bytes.len() as u64 > limits.get(BudgetKind::Encoded) {
        return Err(XcfError::new(ErrorKind::Limit, 0, "encoded source budget exceeded"));
    }
    if kind == SourceContainer::Plain {
        budget.check(BudgetKind::Logical, bytes.len() as u64, "logical source")?;
        if bytes.len() as u64 > limits.get(BudgetKind::Logical) {
            return Err(XcfError::new(ErrorKind::Limit, 0, "logical source budget exceeded"));
        }
        return Ok((Cow::Borrowed(bytes), None));
    }
    let input = usize_limit(limits, BudgetKind::Encoded)?;
    let output = usize_limit(limits, BudgetKind::Logical)?;
    let state = usize_limit(limits, BudgetKind::Decoder)?;
    let index = usize_limit(limits, BudgetKind::Index)?;
    let control = Control {
        budget,
        limits,
        state: RefCell::new(budget.reserve(BudgetKind::Decoder, 0, "wrapper decoder entry")?),
        index: RefCell::new(budget.reserve(BudgetKind::Index, 0, "wrapper index entry")?),
        output: RefCell::new(budget.reserve(BudgetKind::Resident, 0, "wrapper output entry")?),
        failure: RefCell::new(None),
    };
    let mut poll = || {
        if cancelled.is_some_and(|poll| poll()) {
            budget.mark_cancelled();
            return control.io(Err(XcfError::new(ErrorKind::Cancelled, 0, "cancelled during wrapper decode")));
        }
        control.io(budget.poll("wrapper decode"))
    };
    let result = match kind {
        SourceContainer::Gzip => xcf_gzip::decode_with_reservation(bytes, input, output, state, &mut poll, &mut |kind, n| {
            control.peak(
                match kind {
                    xcf_gzip::Reservation::DecoderState => BudgetKind::Decoder,
                    xcf_gzip::Reservation::OutputOverlap => BudgetKind::Resident,
                },
                n,
            )
        }),
        SourceContainer::Bzip2 => xcf_bzip2::decode_with_reservation(bytes, input, output, state, &mut poll, &mut |kind, n| {
            control.peak(
                match kind {
                    xcf_bzip2::Reservation::DecoderState => BudgetKind::Decoder,
                    xcf_bzip2::Reservation::OutputOverlap => BudgetKind::Resident,
                },
                n,
            )
        }),
        SourceContainer::Xz => xcf_xz::decode_with_reservation(bytes, input, output, state, index, &mut poll, &mut |kind, n| {
            control.peak(
                match kind {
                    xcf_xz::Reservation::DecoderState => BudgetKind::Decoder,
                    xcf_xz::Reservation::IndexPeak => BudgetKind::Index,
                    xcf_xz::Reservation::OutputPeak => BudgetKind::Resident,
                },
                n,
            )
        }),
        SourceContainer::Plain => return Err(XcfError::new(ErrorKind::Structure, 0, "plain wrapper dispatch")),
    }
    .map_err(|error| control.error(error))?;
    // A backend's final allocation/copy is still inside cancellation authority;
    // do not hand a completed logical buffer to the parser without a final poll.
    poll().map_err(|error| control.error(error))?;
    budget.check(BudgetKind::Logical, result.len() as u64, "logical source")?;
    let retained_capacity = result.capacity() as u64;
    if retained_capacity > limits.get(BudgetKind::Resident) {
        return Err(XcfError::new(ErrorKind::Limit, 0, "logical buffer retained capacity exceeds resident ceiling"));
    }
    let mut logical_lease = control.output.into_inner();
    let retained_peak = budget
        .used(BudgetKind::Resident)
        .checked_sub(logical_lease.bytes())
        .and_then(|n| n.checked_add(retained_capacity))
        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "logical handoff resident accounting overflow"))?;
    if retained_peak > limits.get(BudgetKind::Resident) {
        return Err(XcfError::new(ErrorKind::Limit, 0, "lowered logical handoff resident ceiling exceeded"));
    }
    // Length describes the logical stream. Capacity describes the live Vec
    // allocation retained alongside parsed records; its spare bytes stay charged.
    logical_lease.resize(retained_capacity, "logical buffer retained capacity")?;
    if cancelled.is_some_and(|callback| callback()) {
        budget.mark_cancelled();
        return Err(XcfError::new(ErrorKind::Cancelled, 0, "cancelled at logical buffer handoff"));
    }
    budget.poll("logical buffer handoff")?;
    Ok((Cow::Owned(result), Some(logical_lease)))
}
/// Additive caller-owned policy, with original container provenance retained separately.
pub fn decode_with_resources(bytes: &[u8], options: &DecodeOptions<'_>, limits: &ResourceLimits) -> Result<DecodedImport, XcfError> {
    decode_candidate(bytes, options, limits)?.ok_or_else(|| XcfError::new(ErrorKind::Header, 0, "wrapper does not contain plain GIMP XCF"))
}
/// Bounded content-first routing. None means an unclaimed non-XCF prefix;
/// its actual format loader owns complete validation. Claimed XCF receives full
/// checksum/EOF validation with final refusal. Nested wrappers are rejected.
pub fn decode_candidate(bytes: &[u8], options: &DecodeOptions<'_>, limits: &ResourceLimits) -> Result<Option<DecodedImport>, XcfError> {
    let budget = Budget::new(limits.clone(), options.cancelled);
    Ok(decode_candidate_with_budget(bytes, options, &budget, SourceAccounting::Borrowed)?.map(|result| result.decoded))
}

/// Borrowed input receives source leases here; CallerReserved requires live caller
/// Encoded and Resident leases covering these exact immutable bytes until mapping ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceAccounting {
    Borrowed,
    CallerReserved,
}
#[derive(Debug)]
pub struct DecodeReservations<'b, 'a> {
    source_encoded: Option<Reservation<'b, 'a>>,
    source_resident: Option<Reservation<'b, 'a>>,
    retained: Reservation<'b, 'a>,
}
impl DecodeReservations<'_, '_> {
    /// Atomically own the actual encoded/source/model charges. On any error,
    /// every borrowed receipt remains unchanged. This does not census mutable
    /// decoded fields or grant source/pixel correspondence.
    pub fn detach_owned(&mut self) -> Result<crate::OwnedReservationBatch, XcfError> {
        match (&mut self.source_encoded, &mut self.source_resident) {
            (Some(encoded), Some(resident)) => Reservation::detach_owned_batch(&mut [encoded, resident, &mut self.retained]),
            (Some(encoded), None) => Reservation::detach_owned_batch(&mut [encoded, &mut self.retained]),
            (None, Some(resident)) => Reservation::detach_owned_batch(&mut [resident, &mut self.retained]),
            (None, None) => Reservation::detach_owned_batch(&mut [&mut self.retained]),
        }
    }
    pub fn retained_bytes(&self) -> u64 {
        self.retained.bytes()
    }
    pub fn source_bytes(&self) -> u64 {
        self.source_resident.as_ref().map_or(0, Reservation::bytes)
    }
    pub fn resize_retained(&mut self, bytes: u64, phase: &str) -> Result<(), XcfError> {
        self.retained.resize(bytes, phase)
    }
    pub fn encoded_bytes(&self) -> u64 {
        self.source_encoded.as_ref().map_or(0, Reservation::bytes)
    }
}
#[derive(Debug)]
pub struct BudgetDecodedImport<'b, 'a, P = Plane> {
    pub decoded: DecodedImport<P>,
    pub reservations: DecodeReservations<'b, 'a>,
}
pub fn decode_candidate_with_budget<'b, 'a>(
    bytes: &[u8],
    options: &DecodeOptions<'_>,
    budget: &'b Budget<'a>,
    accounting: SourceAccounting,
) -> Result<Option<BudgetDecodedImport<'b, 'a>>, XcfError> {
    let limits = budget.limits();
    let effective = limits.lowered(&[
        ("encodedBytes", limits.get(BudgetKind::Encoded).min(options.limits.input_bytes as u64)),
        ("logicalBytes", limits.get(BudgetKind::Logical).min(options.limits.input_bytes as u64)),
        ("residentBytes", limits.get(BudgetKind::Resident).min(options.limits.resident_bytes as u64)),
    ])?;
    let limits = &effective;
    decode_control::poll(budget, options.cancelled, "import entry")?;
    let Some(kind) = classify_container(bytes) else {
        return Ok(None);
    };
    let size = bytes.len() as u64;
    budget.check(BudgetKind::Encoded, size, "source admission")?;
    if size > limits.get(BudgetKind::Encoded) {
        return Err(XcfError::new(ErrorKind::Limit, 0, "encoded source budget exceeded"));
    }
    let (source_encoded, source_resident) = match accounting {
        SourceAccounting::Borrowed => {
            (Some(budget.reserve(BudgetKind::Encoded, size, "encoded source")?), Some(budget.reserve(BudgetKind::Resident, size, "source snapshot")?))
        }
        SourceAccounting::CallerReserved => {
            if budget.used(BudgetKind::Encoded) < size || budget.used(BudgetKind::Resident) < size {
                return Err(XcfError::new(ErrorKind::Limit, 0, "caller source reservations missing"));
            }
            (None, None)
        }
    };
    if kind != SourceContainer::Plain {
        // A prefix only chooses authority. Its bytes are dropped before their
        // live fee; a claimed XCF still goes through the complete old decoder.
        struct Probe<'p, 'b, 'a> {
            bytes: Cow<'p, [u8]>,
            _lease: Option<Reservation<'b, 'a>>,
        }
        let (bytes, lease) = prefix::probe(bytes, kind, budget, limits, options.cancelled)?;
        let probe = Probe { bytes, _lease: lease };
        budget.consume_work(probe.bytes.len() as u64 + 1, "logical signature routing")?;
        if !is_xcf(&probe.bytes) {
            if classify_container(&probe.bytes).is_some() {
                return Err(XcfError::new(ErrorKind::Header, 0, "recursive wrapper rejected"));
            }
            return Ok(None);
        }
        drop(probe);
    }
    let (logical, logical_lease) = unwrap(bytes, kind, budget, limits, options.cancelled)?;
    if !is_xcf(&logical) {
        if classify_container(&logical).is_some() {
            return Err(XcfError::new(ErrorKind::Header, 0, "recursive wrapper rejected"));
        }
        return Ok(None);
    }
    let retained_encoded = size;
    let resident = limits
        .get(BudgetKind::Resident)
        .checked_sub(budget.used(BudgetKind::Resident))
        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "encoded source exhausts resident policy"))?;
    let mut legacy = options.limits.clone();
    macro_rules! lower {
        ($field:ident,$kind:ident) => {
            legacy.$field = legacy.$field.min(usize_limit(limits, BudgetKind::$kind)?);
        };
    }
    lower!(input_bytes, Logical);
    legacy.resident_bytes = legacy.resident_bytes.min(usize::try_from(resident).map_err(|_| XcfError::new(ErrorKind::Limit, 0, "resident address space"))?);
    lower!(drawables, Drawables);
    lower!(properties, Properties);
    lower!(visits, Visits);
    lower!(group_depth, GroupDepth);
    lower!(string_bytes, String);
    lower!(profile_bytes, Profile);
    lower!(property_bytes, Property);
    lower!(metadata_bytes, Metadata);
    legacy.side = legacy.side.min(u32::try_from(limits.get(BudgetKind::Side)).map_err(|_| XcfError::new(ErrorKind::Limit, 0, "side width"))?);
    legacy.pixels = legacy.pixels.min(limits.get(BudgetKind::Pixels));
    let cancelled = || {
        if budget.poll("XCF parse").is_err() {
            return true;
        }
        if options.cancelled.is_some_and(|poll| poll()) {
            budget.mark_cancelled();
            return true;
        }
        false
    };
    let local = DecodeOptions {
        limits: legacy,
        allow_finite_drawable_alpha: options.allow_finite_drawable_alpha,
        cancelled: Some(&cancelled),
        progress: options.progress,
    };
    let logical_bytes = logical.len() as u64;
    let result = crate::reader::decode(&logical, &local, budget, true);
    decode_control::poll(budget, options.cancelled, "import completion")?;
    let mut file = result?;
    // Parsed records own their retained bytes; the logical transport buffer is freed.
    drop(logical);
    drop(logical_lease);
    file.charged_bytes = file
        .charged_bytes
        .checked_add(usize::try_from(retained_encoded).map_err(|_| XcfError::new(ErrorKind::Limit, 0, "retained source width"))?)
        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "resident accounting overflow"))?;
    let aggregate = budget
        .used(BudgetKind::Resident)
        .checked_add((file.charged_bytes as u64).saturating_sub(size))
        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "handoff resident overflow"))?;
    if aggregate > limits.get(BudgetKind::Resident) {
        return Err(XcfError::new(ErrorKind::Limit, 0, "lowered aggregate resident handoff denied"));
    }
    let retained = budget.reserve(
        BudgetKind::Resident,
        (file.charged_bytes as u64).checked_sub(size).ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "retained source accounting"))?,
        "parsed state handoff",
    )?;
    decode_control::poll(budget, options.cancelled, "parsed state handoff")?;
    Ok(Some(BudgetDecodedImport {
        decoded: DecodedImport { file, container: kind, encoded_bytes: size, logical_bytes },
        reservations: DecodeReservations { source_encoded, source_resident, retained },
    }))
}

/// Plain-only bounded random access. CallerReserved variants are trusted accounting
/// contracts, never grants or proof that arbitrary Source implementations are immutable.
/// Actual snapshot ownership and source consistency belong to the authorized caller.
pub fn decode_plain_source_with_budget<'b, 'a>(
    source: &mut dyn Source,
    options: &DecodeOptions<'_>,
    budget: &'b Budget<'a>,
    accounting: PlainSourceAccounting,
) -> Result<BudgetDecodedImport<'b, 'a>, XcfError> {
    decode_plain_source_into_sink_with_budget(source, options, budget, accounting, &mut CollectingPlaneSink::default())
}

pub fn decode_plain_source_into_sink_with_budget<'b, 'a, S: PlaneSink>(
    source: &mut dyn Source,
    options: &DecodeOptions<'_>,
    budget: &'b Budget<'a>,
    accounting: PlainSourceAccounting,
    sink: &mut S,
) -> Result<BudgetDecodedImport<'b, 'a, S::Plane>, XcfError> {
    decode_plain_source_into_sink_mode(source, options, budget, accounting, sink, false)
}
/// Read bounded original metadata and plane descriptors without accessing pixel
/// hierarchy/tile payloads. This is NOT pixel validation or a source grant.
/// Returned metadata owns the same real parser reservations as actual decode;
/// a later decode must use the same immutable Source and original Budget.
/// Retains actual metadata allocations and entry/header allowance. Pixel
/// payloads are absent and this type cannot be used as a decoded native plane.
pub struct PlainSourceMetadata<'b, 'a> {
    parsed: BudgetDecodedImport<'b, 'a, PlaneDescriptor>,
    _header: Reservation<'b, 'a>,
}
impl PlainSourceMetadata<'_, '_> {
    pub fn file(&self) -> &XcfFile<PlaneDescriptor> {
        &self.parsed.decoded.file
    }
}
pub fn read_plain_source_metadata_with_budget<'b, 'a>(
    source: &mut dyn Source,
    options: &DecodeOptions<'_>,
    budget: &'b Budget<'a>,
    accounting: PlainSourceAccounting,
) -> Result<PlainSourceMetadata<'b, 'a>, XcfError> {
    let header = (std::mem::size_of::<PlainSourceMetadata<'_, '_>>() as u64)
        .checked_add(std::mem::size_of::<crate::sink::MetadataPlaneSink>() as u64)
        .and_then(|n| n.checked_add(std::mem::size_of::<crate::reader::Reader<'_, '_>>() as u64))
        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "metadata planning entry size"))?;
    let fee = budget.reserve(BudgetKind::Resident, header, "metadata planning entry/header")?;
    budget.consume_work(512, "metadata planning entry")?;
    let parsed = decode_plain_source_into_sink_mode(source, options, budget, accounting, &mut crate::sink::MetadataPlaneSink::new(), true)?;
    Ok(PlainSourceMetadata { parsed, _header: fee })
}
fn decode_plain_source_into_sink_mode<'b, 'a, S: PlaneSink>(
    source: &mut dyn Source,
    options: &DecodeOptions<'_>,
    budget: &'b Budget<'a>,
    accounting: PlainSourceAccounting,
    sink: &mut S,
    metadata_only: bool,
) -> Result<BudgetDecodedImport<'b, 'a, S::Plane>, XcfError> {
    decode_control::poll(budget, options.cancelled, "plain source admission")?;
    let size = source.len();
    let limits = budget.limits();
    budget.check(BudgetKind::Encoded, size, "plain encoded source")?;
    budget.check(BudgetKind::Logical, size, "plain logical source")?;
    if size > options.limits.input_bytes as u64 {
        return Err(XcfError::new(ErrorKind::Limit, 0, "plain source input limit"));
    }
    let (source_encoded, source_resident) = match accounting {
        PlainSourceAccounting::BorrowedMemory => {
            (Some(budget.reserve(BudgetKind::Encoded, size, "plain encoded memory")?), Some(budget.reserve(BudgetKind::Resident, size, "plain source memory")?))
        }
        PlainSourceAccounting::CallerReservedMemory | PlainSourceAccounting::CallerReservedScratch => {
            let storage = if accounting == PlainSourceAccounting::CallerReservedMemory { BudgetKind::Resident } else { BudgetKind::Scratch };
            if budget.used(BudgetKind::Encoded) < size || budget.used(storage) < size {
                return Err(XcfError::new(ErrorKind::Limit, 0, "caller plain source reservations missing"));
            }
            (None, None)
        }
    };
    let remaining = limits
        .get(BudgetKind::Resident)
        .checked_sub(budget.used(BudgetKind::Resident))
        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "plain source resident admission"))?;
    let mut legacy = options.limits.clone();
    macro_rules! lower {
        ($field:ident,$kind:ident) => {
            legacy.$field = legacy.$field.min(usize_limit(limits, BudgetKind::$kind)?);
        };
    }
    lower!(input_bytes, Logical);
    lower!(drawables, Drawables);
    lower!(properties, Properties);
    lower!(visits, Visits);
    lower!(group_depth, GroupDepth);
    lower!(string_bytes, String);
    lower!(profile_bytes, Profile);
    lower!(property_bytes, Property);
    lower!(metadata_bytes, Metadata);
    legacy.resident_bytes =
        legacy.resident_bytes.min(usize::try_from(remaining).map_err(|_| XcfError::new(ErrorKind::Limit, 0, "plain resident address space"))?);
    legacy.side = legacy.side.min(u32::try_from(limits.get(BudgetKind::Side)).map_err(|_| XcfError::new(ErrorKind::Limit, 0, "plain side width"))?);
    legacy.pixels = legacy.pixels.min(limits.get(BudgetKind::Pixels));
    let local = DecodeOptions {
        limits: legacy,
        allow_finite_drawable_alpha: options.allow_finite_drawable_alpha,
        cancelled: options.cancelled,
        progress: options.progress,
    };
    let mut file = if metadata_only {
        crate::reader::decode_source_metadata(source, &local, budget, sink)?
    } else {
        crate::reader::decode_source(source, &local, budget, sink)?
    };
    decode_control::poll(budget, options.cancelled, "plain parsed handoff")?;
    let parsed = file.charged_bytes as u64;
    let retained = budget.reserve(BudgetKind::Resident, parsed, "plain parsed state handoff")?;
    // Keep the existing decoded accounting convention for downstream mapping.
    file.charged_bytes = file
        .charged_bytes
        .checked_add(usize::try_from(size).map_err(|_| XcfError::new(ErrorKind::Limit, 0, "plain accounting address space"))?)
        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "plain handoff accounting overflow"))?;
    decode_control::poll(budget, options.cancelled, "plain parsed state handoff")?;
    Ok(BudgetDecodedImport {
        decoded: DecodedImport { file, container: SourceContainer::Plain, encoded_bytes: size, logical_bytes: size },
        reservations: DecodeReservations { source_encoded, source_resident, retained },
    })
}

#[cfg(test)]
#[path = "container_owned_reservation_tests.rs"]
mod owned_reservation_tests;
