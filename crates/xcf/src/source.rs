//! Checked source access and transaction budgets. Policies are caller authority, never file data.
use crate::{ErrorKind, XcfError};
use std::cell::Cell;
use web_time::Instant;
#[path = "source/persistent.rs"]
mod persistent;
pub use persistent::{OwnedLedgerHeader, OwnedReservation, OwnedReservationBatch, PhysicalUsage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourcePolicy {
    NativeStandard,
    NativeLarge,
    WasmStandard,
}
impl Default for ResourcePolicy {
    fn default() -> Self {
        if cfg!(target_arch = "wasm32") { Self::WasmStandard } else { Self::NativeStandard }
    }
}
impl ResourcePolicy {
    pub fn name(self) -> &'static str {
        match self {
            Self::NativeStandard => "native_standard",
            Self::NativeLarge => "native_large",
            Self::WasmStandard => "wasm_standard",
        }
    }
}

macro_rules! kinds {
    ($($variant:ident => ($name:literal, $native:expr, $wasm:expr)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum BudgetKind { $($variant),+ }
        impl BudgetKind {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn name(self) -> &'static str { match self { $(Self::$variant=>$name),+ } }
            fn ceiling(self, policy: ResourcePolicy) -> u64 { match (self,policy) {
                $((Self::$variant, ResourcePolicy::WasmStandard)=>$wasm),+,
                $((Self::$variant,_)=>$native),+
            } }
            fn index(self) -> usize { self as usize }
        }
    }
}
kinds! {
    Encoded => ("encodedBytes", 1<<30, 256<<20),
    Logical => ("logicalBytes", 1<<30, 256<<20),
    Resident => ("residentBytes", 2<<30, 512<<20),
    Decoder => ("decoderBytes", 128<<20, 32<<20),
    Scratch => ("scratchBytes", 4<<30, 0),
    Cache => ("cacheBytes", 256<<20, 64<<20),
    Index => ("indexBytes", 64<<20, 16<<20),
    Resource => ("resourceBytes", 128<<20, 32<<20),
    Deadline => ("deadlineMs", 120000, 120000),
    Work => ("sampleWorkUnits", 1<<40, 1<<36),
    Side => ("side", 262144, 262144),
    Pixels => ("pixels", 1<<30, 1<<30),
    Drawables => ("drawables", 10000, 10000),
    Properties => ("properties", 100000, 100000),
    Visits => ("visits", 20000000, 20000000),
    GroupDepth => ("groupDepth", 100, 100),
    String => ("stringBytes", 1<<20, 1<<20),
    Profile => ("profileBytes", 16<<20, 16<<20),
    Property => ("propertyBytes", 64<<20, 64<<20),
    Metadata => ("metadataBytes", 256<<20, 256<<20),
    FiltersPerOwner => ("filtersPerOwner", 1024, 1024),
    FilterNodes => ("filterNodes", 100000, 100000),
    GraphNodes => ("graphNodes", 4096, 4096),
    GraphEdges => ("graphEdges", 16384, 16384),
    ConfigDepth => ("configDepth", 32, 32),
    ConfigEntries => ("configEntries", 100000, 100000),
    LinkedDepth => ("linkedDepth", 16, 16),
    LinkedDocuments => ("linkedDocuments", 64, 64),
    Text => ("textBytes", 64<<20, 64<<20),
    ArchiveEntries => ("archiveEntries", 1000000, 1000000),
    Output => ("outputBytes", 16<<30, 512<<20),
}
const BUDGET_COUNT: usize = BudgetKind::ALL.len();

#[derive(Clone, Debug)]
pub struct ResourceLimits {
    policy: ResourcePolicy,
    values: [u64; BUDGET_COUNT],
}
impl ResourceLimits {
    pub fn for_policy(policy: ResourcePolicy) -> Result<Self, XcfError> {
        if policy == ResourcePolicy::NativeLarge || cfg!(target_arch = "wasm32") && policy != ResourcePolicy::WasmStandard {
            return Err(XcfError::new(ErrorKind::Unsupported, 0, "native_large is not qualified; wasm permits only wasm_standard"));
        }
        let mut limits = Self { policy, values: [0; BUDGET_COUNT] };
        for &kind in BudgetKind::ALL {
            if let Some(value) = limits.values.get_mut(kind.index()) {
                *value = kind.ceiling(policy);
            }
        }
        Ok(limits)
    }
    pub fn policy(&self) -> ResourcePolicy {
        self.policy
    }
    pub fn get(&self, kind: BudgetKind) -> u64 {
        self.values.get(kind.index()).copied().unwrap_or(0)
    }
    /// Unknown keys and raises fail before any source I/O. Zero is a real zero ceiling.
    pub fn lowered(&self, overrides: &[(&str, u64)]) -> Result<Self, XcfError> {
        let mut out = self.clone();
        for &(name, value) in overrides {
            let kind = BudgetKind::ALL
                .iter()
                .find(|kind| kind.name() == name)
                .copied()
                .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, format!("unknown resource limit {name}")))?;
            if value > self.get(kind) {
                return Err(XcfError::new(
                    ErrorKind::Limit,
                    0,
                    format!("{} {name}: allowed {}, observed {value}; policy validation", self.policy.name(), self.get(kind)),
                ));
            }
            if let Some(slot) = out.values.get_mut(kind.index()) {
                *slot = value;
            }
        }
        Ok(out)
    }
}

pub struct Budget<'a> {
    limits: ResourceLimits,
    used: [Cell<u64>; BUDGET_COUNT],
    peak: [Cell<u64>; BUDGET_COUNT],
    started: Instant,
    cancelled: Option<&'a dyn Fn() -> bool>,
    cancelled_observed: Cell<bool>,
    shared: Option<persistent::SharedLedger>,
}
impl<'a> Budget<'a> {
    pub fn new(limits: ResourceLimits, cancelled: Option<&'a dyn Fn() -> bool>) -> Self {
        Self {
            limits,
            used: std::array::from_fn(|_| Cell::new(0)),
            peak: std::array::from_fn(|_| Cell::new(0)),
            started: Instant::now(),
            cancelled,
            cancelled_observed: Cell::new(false),
            shared: None,
        }
    }
    /// Explicit opt-in persistent ownership. Existing new() stays Cell-only with
    /// no Arc allocation or synchronization. Callback/deadline authority remains
    /// borrowed by this job-local Budget, never retained by an owned receipt.
    pub fn new_persistent(limits: ResourceLimits, cancelled: Option<&'a dyn Fn() -> bool>) -> Result<Self, XcfError> {
        let started = Instant::now();
        if cancelled.is_some_and(|f| f()) {
            return Err(XcfError::new(ErrorKind::Cancelled, 0, "cancelled during persistent budget construction"));
        }
        let shared = persistent::SharedLedger::new(&limits)?;
        Ok(Self {
            limits,
            used: std::array::from_fn(|_| Cell::new(0)),
            peak: std::array::from_fn(|_| Cell::new(0)),
            started,
            cancelled,
            cancelled_observed: Cell::new(false),
            shared: Some(shared),
        })
    }
    pub(crate) fn mark_cancelled(&self) {
        self.cancelled_observed.set(true);
    }
    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }
    pub fn used(&self, kind: BudgetKind) -> u64 {
        if let Some(shared) = &self.shared {
            return shared.used(kind);
        }
        self.used.get(kind.index()).map(Cell::get).unwrap_or(0)
    }
    /// High-water mark of successfully charged reservations or performed work.
    /// Admission checks alone are not charges; backend transient estimates and
    /// process RSS require their own measurements.
    pub fn peak(&self, kind: BudgetKind) -> u64 {
        if let Some(shared) = &self.shared {
            return shared.peak(kind);
        }
        self.peak.get(kind.index()).map(Cell::get).unwrap_or(0)
    }
    pub fn poll(&self, phase: &str) -> Result<(), XcfError> {
        if self.cancelled_observed.get() || self.cancelled.is_some_and(|f| f()) {
            self.cancelled_observed.set(true);
            return Err(XcfError::new(ErrorKind::Cancelled, 0, format!("cancelled during {phase}")));
        }
        if let Some(shared) = &self.shared {
            shared.valid(phase)?;
        }
        if self.started.elapsed().as_millis() >= u128::from(self.limits.get(BudgetKind::Deadline)) {
            return Err(self.denial(BudgetKind::Deadline, self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64, phase));
        }
        Ok(())
    }
    fn denial(&self, kind: BudgetKind, observed: u64, phase: &str) -> XcfError {
        XcfError::new(
            ErrorKind::Limit,
            0,
            format!("{} {}: allowed {}, observed {observed}; {phase}", self.limits.policy.name(), kind.name(), self.limits.get(kind)),
        )
    }
    pub fn check(&self, kind: BudgetKind, n: u64, phase: &str) -> Result<(), XcfError> {
        self.poll(phase)?;
        if n > self.limits.get(kind) { Err(self.denial(kind, n, phase)) } else { Ok(()) }
    }
    fn set(&self, kind: BudgetKind, n: u64) {
        if let Some(slot) = self.used.get(kind.index()) {
            slot.set(n);
        }
        if let Some(slot) = self.peak.get(kind.index()) {
            slot.set(slot.get().max(n));
        }
    }
    fn added(&self, kind: BudgetKind, n: u64, phase: &str) -> Result<u64, XcfError> {
        let total = self.used(kind).checked_add(n).ok_or_else(|| self.denial(kind, u64::MAX, phase))?;
        self.check(kind, total, phase)?;
        Ok(total)
    }
    /// Decoder/cache/index/resource reservations also consume the single resident total.
    pub fn reserve(&self, kind: BudgetKind, n: u64, phase: &str) -> Result<Reservation<'_, 'a>, XcfError> {
        self.poll(phase)?;
        if matches!(kind, BudgetKind::Work | BudgetKind::Visits | BudgetKind::Deadline) {
            return Err(XcfError::new(ErrorKind::Limit, 0, "performed work/visits and deadline cannot use refundable reservations"));
        }
        let resident = matches!(kind, BudgetKind::Decoder | BudgetKind::Cache | BudgetKind::Index | BudgetKind::Resource);
        if let Some(shared) = &self.shared {
            shared.reserve(kind, n, resident, phase)?;
            return Ok(Reservation { budget: self, kind, bytes: n, resident });
        }
        let total = self.added(kind, n, phase)?;
        let resident_total = if resident { Some(self.added(BudgetKind::Resident, n, phase)?) } else { None };
        self.set(kind, total);
        if let Some(total) = resident_total {
            self.set(BudgetKind::Resident, total);
        }
        Ok(Reservation { budget: self, kind, bytes: n, resident })
    }
    /// Performed work is never refunded. Kernels must call this at <=4096-unit boundaries.
    pub fn consume_work(&self, n: u64, phase: &str) -> Result<(), XcfError> {
        if let Some(shared) = &self.shared {
            self.poll(phase)?;
            return shared.reserve(BudgetKind::Work, n, false, phase);
        }
        let total = self.added(BudgetKind::Work, n, phase)?;
        self.set(BudgetKind::Work, total);
        Ok(())
    }
    pub fn consume_visits(&self, n: u64, phase: &str) -> Result<(), XcfError> {
        if let Some(shared) = &self.shared {
            self.poll(phase)?;
            return shared.reserve(BudgetKind::Visits, n, false, phase);
        }
        let total = self.added(BudgetKind::Visits, n, phase)?;
        self.set(BudgetKind::Visits, total);
        Ok(())
    }
}
pub struct Reservation<'b, 'a> {
    budget: &'b Budget<'a>,
    kind: BudgetKind,
    bytes: u64,
    resident: bool,
}
impl Reservation<'_, '_> {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    /// Transfer this exact charge without a refund/recharge or copied statistics.
    /// On failure the borrowed receipt is unchanged. The local Cell mode cannot
    /// manufacture a persistent receipt. Successful transfer empties this receipt.
    pub fn detach_owned(&mut self) -> Result<OwnedReservation, XcfError> {
        let shared = self.budget.shared.as_ref().ok_or_else(|| XcfError::new(ErrorKind::Unsupported, 0, "local XCF reservations cannot detach"))?;
        self.budget.poll("persistent reservation handoff")?;
        let owned = shared.owned(self.kind, self.bytes, self.resident);
        self.bytes = 0;
        Ok(owned)
    }
    /// Grow atomically; shrinking is cleanup and remains permitted after cancellation.
    pub fn resize(&mut self, bytes: u64, phase: &str) -> Result<(), XcfError> {
        if let Some(shared) = &self.budget.shared {
            if bytes >= self.bytes {
                self.budget.poll(phase)?;
            }
            shared.resize(self.kind, self.bytes, bytes, self.resident, phase)?;
            self.bytes = bytes;
            return Ok(());
        }
        if bytes >= self.bytes {
            let delta = bytes - self.bytes;
            let total = self.budget.added(self.kind, delta, phase)?;
            let resident_total = if self.resident { Some(self.budget.added(BudgetKind::Resident, delta, phase)?) } else { None };
            self.budget.set(self.kind, total);
            if let Some(total) = resident_total {
                self.budget.set(BudgetKind::Resident, total);
            }
        } else {
            let delta = self.bytes - bytes;
            self.budget.set(self.kind, self.budget.used(self.kind).saturating_sub(delta));
            if self.resident {
                self.budget.set(BudgetKind::Resident, self.budget.used(BudgetKind::Resident).saturating_sub(delta));
            }
        }
        self.bytes = bytes;
        Ok(())
    }
}
impl std::fmt::Debug for Reservation<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reservation").field("kind", &self.kind).field("bytes", &self.bytes).finish()
    }
}
impl Drop for Reservation<'_, '_> {
    fn drop(&mut self) {
        if let Some(shared) = &self.budget.shared {
            shared.refund(self.kind, self.bytes, self.resident);
            return;
        }
        self.budget.set(self.kind, self.budget.used(self.kind).saturating_sub(self.bytes));
        if self.resident {
            self.budget.set(BudgetKind::Resident, self.budget.used(BudgetKind::Resident).saturating_sub(self.bytes));
        }
    }
}

/// Offsets remain U64 until the bounded local read window is admitted.
pub trait Source {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError>;
}
pub struct ByteSource<'a> {
    bytes: &'a [u8],
}
impl<'a> ByteSource<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}
impl Source for ByteSource<'_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        budget.check(BudgetKind::Encoded, self.len(), "source admission")?;
        let end = offset.checked_add(out.len() as u64).ok_or_else(|| XcfError::new(ErrorKind::Structure, 0, "U64 source extent overflow"))?;
        if end > self.len() {
            return Err(XcfError::new(ErrorKind::Truncated, 0, "source extent outside input"));
        }
        let start = usize::try_from(offset).map_err(|_| XcfError::new(ErrorKind::Limit, 0, "source window exceeds address space"))?;
        let data = self
            .bytes
            .get(start..usize::try_from(end).map_err(|_| XcfError::new(ErrorKind::Limit, 0, "source extent exceeds address space"))?)
            .ok_or_else(|| XcfError::new(ErrorKind::Truncated, 0, "source extent"))?;
        for (to, from) in out.chunks_mut(65536).zip(data.chunks(65536)) {
            budget.poll("source read")?;
            to.copy_from_slice(from);
        }
        budget.poll("source read completed")
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct FileSource {
    file: std::fs::File,
    len: u64,
}
#[cfg(not(target_arch = "wasm32"))]
impl FileSource {
    /// The caller supplies an already authorized handle; this never resolves a pathname.
    pub fn new(file: std::fs::File, budget: &Budget<'_>) -> Result<Self, XcfError> {
        budget.poll("source metadata")?;
        let metadata = file.metadata();
        budget.poll("source metadata completed")?;
        let len = metadata.map_err(|e| XcfError::new(ErrorKind::Structure, 0, format!("source metadata: {e}")))?.len();
        budget.check(BudgetKind::Encoded, len, "source admission")?;
        Ok(Self { file, len })
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn read_exact_controlled<R: std::io::Read>(reader: &mut R, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
    let mut calls = 0usize;
    let mut interrupted = 0usize;
    for chunk in out.chunks_mut(65536) {
        let mut filled = 0usize;
        while filled < chunk.len() {
            budget.poll("source partial read")?;
            calls = calls.checked_add(1).filter(|n| *n <= 1_000_000).ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "source read call limit exceeded"))?;
            budget.consume_work(1, "source read call")?;
            let target = chunk.get_mut(filled..).ok_or_else(|| XcfError::new(ErrorKind::Structure, 0, "source read bounds"))?;
            let offered = target.len();
            let read = reader.read(target);
            // Cooperative only: this cannot interrupt a syscall that has not returned.
            budget.poll("source partial read completed")?;
            let count = match read {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                    interrupted = interrupted
                        .checked_add(1)
                        .filter(|n| *n <= 1024)
                        .ok_or_else(|| XcfError::new(ErrorKind::Limit, 0, "source interrupted retry limit exceeded"))?;
                    continue;
                }
                Err(error) => return Err(XcfError::new(ErrorKind::Truncated, 0, format!("source read: {error}"))),
                Ok(0) => return Err(XcfError::new(ErrorKind::Truncated, 0, "source ended during bounded read")),
                Ok(count) if count > offered => return Err(XcfError::new(ErrorKind::Structure, 0, "source returned an invalid read count")),
                Ok(count) => count,
            };
            filled = filled.checked_add(count).ok_or_else(|| XcfError::new(ErrorKind::Structure, 0, "source read offset overflow"))?;
        }
    }
    budget.poll("source read completed")
}
#[cfg(not(target_arch = "wasm32"))]
impl Source for FileSource {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        use std::io::{Seek, SeekFrom};
        budget.check(BudgetKind::Encoded, self.len, "source admission")?;
        let end = offset.checked_add(out.len() as u64).ok_or_else(|| XcfError::new(ErrorKind::Structure, 0, "U64 source extent overflow"))?;
        if end > self.len {
            return Err(XcfError::new(ErrorKind::Truncated, 0, "source extent outside handle"));
        }
        budget.poll("source seek")?;
        let seek = self.file.seek(SeekFrom::Start(offset));
        budget.poll("source seek completed")?;
        seek.map_err(|e| XcfError::new(ErrorKind::Structure, 0, format!("source seek: {e}")))?;
        read_exact_controlled(&mut self.file, out, budget)
    }
}
