#![cfg(windows)]
#[path = "common/xcf.rs"]
mod common;
use photocraft_io::xcf_source::{OwnedXcfSnapshot, ServiceAuthorizedScratchDirectory};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Budget, BudgetKind, DecodeOptions, ErrorKind, PlainSourceAccounting, ResourceLimits, ResourcePolicy, Source, XcfError};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-root-source-import-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
struct Padded {
    prefix: Vec<u8>,
    size: u64,
}
impl Source for Padded {
    fn len(&self) -> u64 {
        self.size
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], _: &Budget<'_>) -> Result<(), XcfError> {
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = self.prefix.get(usize::try_from(offset).unwrap() + index).copied().unwrap_or(0);
        }
        Ok(())
    }
}
struct Failing<'s> {
    source: &'s mut dyn Source,
    reads: usize,
    cancel: Option<&'s std::sync::atomic::AtomicBool>,
}
impl Source for Failing<'_> {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        self.reads += 1;
        // Padding is read only by full-source provenance hashing, after the
        // valid raster and document have already been staged.
        if offset >= 65536 {
            if let Some(cancel) = self.cancel {
                cancel.store(true, Ordering::SeqCst);
            } else {
                return Err(XcfError { kind: ErrorKind::Structure, offset, item: String::new(), message: "injected late provenance read failure".into() });
            }
        }
        self.source.read_exact_at(offset, out, budget)
    }
}
fn refunded(budget: &Budget<'_>, root: &Root) {
    assert!(std::fs::read_dir(&root.0).unwrap().next().is_none());
    for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
        assert_eq!(budget.used(kind), 0);
    }
}
#[test]
fn late_hash_read_error_and_cancel_discard_staged_document_and_refund() {
    for cancel_case in [false, true] {
        let flag = std::sync::atomic::AtomicBool::new(false);
        let stop = || flag.load(Ordering::SeqCst);
        let limits = ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap().lowered(&[("residentBytes", 4 << 20)]).unwrap();
        let budget = Budget::new(limits, Some(&stop));
        let root = Root::new();
        {
            let mut original = Padded { prefix: common::image(22, 150, 0, 1, 1, &[20, 70, 120, 255]), size: 8 << 20 };
            let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
            let mut snapshot = directory.capture(&mut original).unwrap();
            let held = budget.used(BudgetKind::Resident);
            let mut failing = Failing { source: &mut snapshot, reads: 0, cancel: cancel_case.then_some(&flag) };
            let result = photocraft_io::xcf_import::import_plain_source_with_budget(
                "late.xcf",
                &mut failing,
                &Interrupt::NONE,
                &DecodeOptions::default(),
                &budget,
                PlainSourceAccounting::CallerReservedScratch,
            );
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("late failure must never publish the staged document"),
            };
            if cancel_case {
                assert!(matches!(error, photocraft_io::IoError::Cancelled));
            } else {
                assert!(
                    matches!(error, photocraft_io::IoError::Xcf(error) if error.kind == ErrorKind::Structure && error.message.contains("injected late provenance"))
                );
            }
            assert!(failing.reads > 16);
            assert_eq!(budget.used(BudgetKind::Resident), held);
            assert_eq!(budget.used(BudgetKind::Encoded), original.size);
            assert_eq!(budget.used(BudgetKind::Scratch), original.size);
            assert_eq!(snapshot.len(), original.size);
        }
        refunded(&budget, &root);
    }
}
#[test]
fn absent_storage_leases_and_memory_budget_fail_before_source_reads() {
    let mut original = Padded { prefix: common::image(22, 150, 0, 1, 1, &[20, 70, 120, 255]), size: 8 << 20 };
    for accounting in [PlainSourceAccounting::CallerReservedScratch, PlainSourceAccounting::CallerReservedMemory, PlainSourceAccounting::BorrowedMemory] {
        let limits = ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap().lowered(&[("residentBytes", 4 << 20)]).unwrap();
        let budget = Budget::new(limits, None);
        let mut source = Failing { source: &mut original, reads: 0, cancel: None };
        let result = photocraft_io::xcf_import::import_plain_source_with_budget(
            "unowned.xcf",
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            accounting,
        );
        assert!(matches!(result, Err(photocraft_io::IoError::Xcf(error)) if error.kind == ErrorKind::Limit));
        assert_eq!(source.reads, 0);
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        assert_eq!(budget.used(BudgetKind::Encoded), 0);
        assert_eq!(budget.used(BudgetKind::Scratch), 0);
    }
}
#[test]
fn real_scratch_snapshot_imports_without_encoded_resident_vec() {
    let mut original = Padded { prefix: common::image(22, 150, 0, 1, 1, &[20, 70, 120, 255]), size: 8 << 20 };
    // Prove unused padding is valid through the existing importer first. The
    // input-sized oracle buffer is destroyed before the small-budget capture.
    let expected = {
        let mut bytes = original.prefix.clone();
        bytes.resize(original.size as usize, 0);
        photocraft_io::import("padding.xcf", &bytes).unwrap().document
    };
    let limits = ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap().lowered(&[("residentBytes", 4 << 20)]).unwrap();
    let budget = Budget::new(limits, None);
    assert_eq!(OwnedXcfSnapshot::from_source(&mut original, &budget).err().unwrap().kind, ErrorKind::Limit);
    let root = Root::new();
    {
        let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
        let mut snapshot = directory.capture(&mut original).unwrap();
        original.prefix.fill(0);
        let actual = photocraft_io::xcf_import::import_plain_source_with_budget(
            "padding.xcf",
            &mut snapshot,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::CallerReservedScratch,
        )
        .unwrap()
        .document;
        // Separate imports deliberately receive fresh application IDs. Rebind
        // only those identities before comparing every source/model field.
        let mut normalized = actual.clone();
        normalized.id = expected.id;
        let old_owner = photocraft_doc::item_identity::ItemRef::Layer(normalized.layers[0].id);
        let owner = photocraft_doc::item_identity::ItemRef::Layer(expected.layers[0].id);
        normalized.layers[0].id = expected.layers[0].id;
        let state = normalized.xcf_state.as_mut().unwrap();
        assert_eq!(state.items.len(), 1);
        assert!(state.resources.is_empty() && state.source_items.is_empty() && state.filter_stacks.is_empty() && state.item_sets.is_empty());
        assert_eq!(state.items[0].item, old_owner);
        state.items[0].item = owner;
        for selected in &mut state.selection.selected {
            assert_eq!(*selected, old_owner);
            *selected = owner;
        }
        if let Some(active) = &mut state.selection.active {
            assert_eq!(*active, old_owner);
            *active = owner;
        }
        assert_eq!(normalized, expected);
        let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), actual);
        assert!(budget.peak(BudgetKind::Resident) < original.size);
    }
    assert!(std::fs::read_dir(&root.0).unwrap().next().is_none());
    for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
        assert_eq!(budget.used(kind), 0);
    }
}
