use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent, LayerId, accounted_document::AccountedDocument};
use photocraft_engine::{
    Session,
    accounted_commands::{self, PreparedAccountedCommand},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    },
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: Arc<AtomicUsize>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        control.check(output.len() as u64)?;
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = self.descriptor.default_bytes()[((offset + index as u64) % 16) as usize];
        }
        Ok(())
    }
}
fn session(nested: bool) -> (Session, BackingQuota, LayerId, Arc<AtomicUsize>) {
    let quota = BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 4 << 20, scratch: 0 });
    let retention = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let bytes = [0x0100_0001u32, 0x7fff_ffff, 0, u32::MAX].map(u32::to_ne_bytes).concat();
    let descriptor = NativePlaneDescriptor::new(PixelFormat::new(ColorMode::Rgb, SampleType::U32, true), Rect::new(-2, 1, 2, 3), &bytes).unwrap();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, reads: reads.clone() }), quota.clone(), 2, &control()).unwrap();
    let layer = Layer::new("leaf", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap()));
    let id = layer.id;
    let mut document = Document::new("accounted commands", Size::new(4, 4), ColorMode::Rgb, SampleType::U32);
    document.layers.push(if nested { Layer::group("parent", vec![layer]) } else { layer });
    let owner = AccountedDocument::from_owned(document, Arc::new(retention), quota.clone(), &control()).unwrap();
    let mut session = Session::new();
    session.add_accounted_document(owner, None, &control()).unwrap();
    (session, quota, id, reads)
}

#[test]
fn typed_properties_history_and_replace_selection_preserve_native_without_reads() {
    let (mut session, quota, layer, reads) = session(false);
    session.journal.push(("tools.set".into(), json!({"tool":"brush"})));
    let original = session.active().unwrap().doc.clone();
    let value = accounted_commands::dispatch(&mut session, "layer.setProps", &json!({"layer":layer.0,"name":"renamed"}), &control()).unwrap();
    assert_eq!(value, Value::Null);
    assert_eq!(session.journal[0], ("tools.set".into(), json!({"tool":"brush"})));
    assert_eq!(session.active().unwrap().doc.layer(layer).unwrap().name, "renamed");
    let original_layer = original.layer(layer).unwrap();
    let current_layer = session.active().unwrap().doc.layer(layer).unwrap();
    let (LayerContent::BackedRaster(before), LayerContent::BackedRaster(after)) = (&original_layer.content, &current_layer.content) else { panic!() };
    assert!(before == after);
    let value = accounted_commands::dispatch(&mut session, "edit.undo", &json!({}), &control()).unwrap();
    assert_eq!(value, Value::Bool(true));
    assert!(session.active().unwrap().doc.shares_state(&original));
    let count = session.active().unwrap().history.past_len();
    let _value = accounted_commands::dispatch(&mut session, "layer.select", &json!({"layer":layer.0,"mode":"replace"}), &control()).unwrap();
    assert_eq!(session.active().unwrap().selection_ids(), &[layer]);
    assert_eq!(session.active().unwrap().history.past_len(), count);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    drop(original);
    drop(session);
    assert_eq!(quota.usage().unwrap().resident, 0);
}

#[test]
fn hostile_requests_and_unqualified_callbacks_refuse_before_state_or_journal_change() {
    let (mut session, quota, layer, reads) = session(false);
    let original = session.active().unwrap().doc.clone();
    let usage = quota.usage().unwrap();
    for params in [
        json!({"layer":layer.0,"opacity":1.1}),
        json!({"layer":layer.0,"visible":1}),
        json!({"layer":layer.0,"name":"a","visible":true}),
        json!({"layer":layer.0,"blend":"Normal"}),
        json!({"layer":layer.0,"name":"a","coalesce":false}),
        json!({"layer":layer.0,"name":"a","coalesce":"drag"}),
        json!({"layer":layer.0,"name":"a","coalesce":""}),
    ] {
        assert!(accounted_commands::dispatch(&mut session, "layer.setProps", &params, &control()).is_err());
        assert!(session.active().unwrap().doc.shares_state(&original));
        assert!(session.journal.is_empty());
        assert_eq!(quota.usage().unwrap(), usage);
    }
    session.actions.recording = Some((0, 0));
    assert!(accounted_commands::dispatch(&mut session, "layer.setProps", &json!({"name":"a"}), &control()).is_err());
    session.actions.recording = None;
    assert!(accounted_commands::dispatch(&mut session, "layer.select", &json!({"layer":layer.0,"mode":"toggle"}), &control()).is_err());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn journal_quota_cancellation_and_stale_preparation_are_atomic() {
    let (mut session, quota, layer, reads) = session(false);
    let original = session.active().unwrap().doc.clone();
    let usage = quota.usage().unwrap();
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(accounted_commands::dispatch(&mut session, "layer.setProps", &json!({"name":"a"}), &deny).is_err());
    assert_eq!(quota.usage().unwrap(), usage);
    let held = quota.reserve(QuotaKind::Index, (4 << 20) - usage.index - 1).unwrap();
    assert!(accounted_commands::dispatch(&mut session, "layer.setProps", &json!({"name":"a"}), &control()).is_err());
    drop(held);
    let params = json!({"layer":layer.0,"name":"new"});
    let prepared = PreparedAccountedCommand::prepare(&session, "layer.setProps", &params, &control()).unwrap();
    session.journal.push(("tools.set".into(), json!({})));
    assert!(prepared.execute(&mut session, &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(quota.usage().unwrap(), usage);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn nested_replace_and_name_use_owned_lookup_without_native_reads() {
    let (mut session, _, layer, reads) = session(true);
    let _value = accounted_commands::dispatch(&mut session, "layer.select", &json!({"layer":layer.0}), &control()).unwrap();
    let _value = accounted_commands::dispatch(&mut session, "layer.setProps", &json!({"layer":layer.0,"name":"nested renamed"}), &control()).unwrap();
    assert_eq!(session.active().unwrap().doc.layer(layer).unwrap().name, "nested renamed");
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn registered_accounted_property_command_runs_through_actual_dispatch() {
    let (mut session, _, layer, reads) = session(true);
    assert_eq!(session.execute("layer.setProps", json!({"layer":layer.0,"name":"registered"})).unwrap(), Value::Null);
    assert_eq!(session.active().unwrap().doc.layer(layer).unwrap().name, "registered");
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn null_enabled_queries_and_history_calls_preserve_existing_shapes() {
    let (mut session, _, layer, _) = session(false);
    assert_eq!(accounted_commands::disabled_reason(&session, "layer.setProps", &Value::Null).unwrap(), None);
    assert_eq!(accounted_commands::disabled_reason(&session, "edit.undo", &Value::Null).unwrap(), Some("nothing to undo"));
    assert_eq!(accounted_commands::dispatch(&mut session, "edit.undo", &Value::Null, &control()).unwrap(), Value::Bool(false));
    assert_eq!(
        accounted_commands::dispatch(&mut session, "layer.setProps", &json!({"layer":layer.0,"visible":false,"coalesce":null}), &control()).unwrap(),
        Value::Null
    );
    assert_eq!(session.active().unwrap().history.undo_label(), Some("Layer Visibility"));
    assert_eq!(accounted_commands::dispatch(&mut session, "edit.undo", &Value::Null, &control()).unwrap(), Value::Bool(true));
    assert_eq!(accounted_commands::dispatch(&mut session, "edit.redo", &Value::Null, &control()).unwrap(), Value::Bool(true));
}

#[test]
fn excessive_inherited_journal_spare_capacity_refuses_before_clone_or_edit() {
    let (mut session, quota, layer, _) = session(false);
    let mut spare = String::with_capacity(256 * 1024);
    spare.push('x');
    session.journal.push(("tools.set".into(), Value::String(spare)));
    let original = session.active().unwrap().doc.clone();
    let usage = quota.usage().unwrap();
    assert!(accounted_commands::dispatch(&mut session, "layer.setProps", &json!({"layer":layer.0,"name":"new"}), &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(session.journal.len(), 1);
    assert_eq!(quota.usage().unwrap(), usage);
}

#[test]
fn cancellation_during_journal_scan_clone_and_edit_refunds_all_staging() {
    let complete_polls = {
        let (mut baseline, _, layer, _) = session(false);
        baseline.journal.push(("tools.set".into(), json!({"nested":["retained",true,42]})));
        let polls = AtomicUsize::new(0);
        let count = || {
            polls.fetch_add(1, Ordering::SeqCst);
            false
        };
        let interrupt = Interrupt::cancel_only(&count);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        accounted_commands::dispatch(&mut baseline, "layer.setProps", &json!({"layer":layer.0,"name":"changed"}), &ctl).unwrap();
        polls.load(Ordering::SeqCst)
    };
    let mut rejected = 0;
    let mut completed = 0;
    for stop in 0..=complete_polls {
        let (mut session, quota, layer, reads) = session(false);
        session.journal.push(("tools.set".into(), json!({"nested":["retained",true,42]})));
        let original = session.active().unwrap().doc.clone();
        let usage = quota.usage().unwrap();
        let revision = session.active().unwrap().revision;
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::SeqCst) >= stop;
        let interrupt = Interrupt::cancel_only(&cancel);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        let result = accounted_commands::dispatch(&mut session, "layer.setProps", &json!({"layer":layer.0,"name":"changed"}), &ctl);
        if result.is_err() {
            rejected += 1;
            assert!(session.active().unwrap().doc.shares_state(&original));
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.journal.len(), 1);
            assert_eq!(quota.usage().unwrap(), usage);
        } else {
            completed += 1;
            assert_eq!(session.journal.len(), 2);
        }
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        drop(original);
        drop(session);
        assert_eq!(quota.usage().unwrap().resident, 0);
    }
    assert!(rejected >= 16);
    assert_eq!(completed, 1);
}

#[derive(Clone, Default)]
struct CountedPresetBackend {
    inner: photocraft_engine::preset_store::MemBackend,
    calls: Arc<AtomicUsize>,
}
impl photocraft_engine::preset_store::PresetBackend for CountedPresetBackend {
    fn list(&self) -> Result<Vec<(String, u64)>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list()
    }
    fn read(&self, name: &str, max: u64) -> Result<Vec<u8>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.read(name, max)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.write(name, bytes)
    }
    fn remove(&self, name: &str) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.remove(name)
    }
}

#[test]
fn clean_attached_preset_store_allows_registered_commands_without_backend_work() {
    let (mut session, quota, layer, reads) = session(false);
    let backend = CountedPresetBackend::default();
    session.attach_preset_store(photocraft_engine::preset_store::open(Box::new(backend.clone())));
    let calls = backend.calls.load(Ordering::SeqCst);
    let files = backend.inner.files.lock().unwrap().clone();
    let presets_rev = session.tools.presets_rev;
    let actions_rev = session.actions.rev;
    for (id, params) in [
        ("layer.select", json!({"layer":layer.0})),
        ("layer.setProps", json!({"layer":layer.0,"name":"attached"})),
        ("edit.undo", Value::Null),
        ("edit.redo", Value::Null),
    ] {
        session.execute(id, params).unwrap();
        assert_eq!(backend.calls.load(Ordering::SeqCst), calls);
        assert_eq!(*backend.inner.files.lock().unwrap(), files);
        assert_eq!(session.tools.presets_rev, presets_rev);
        assert_eq!(session.actions.rev, actions_rev);
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    drop(session);
    assert_eq!(quota.usage().unwrap().resident, 0);
}

#[test]
fn dirty_attached_preset_revisions_refuse_registered_commands_before_edit() {
    for actions_dirty in [false, true] {
        let (mut session, quota, layer, reads) = session(false);
        let backend = CountedPresetBackend::default();
        session.attach_preset_store(photocraft_engine::preset_store::open(Box::new(backend.clone())));
        if actions_dirty {
            session.actions.rev += 1;
        } else {
            session.tools.presets_rev += 1;
        }
        let original = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let usage = quota.usage().unwrap();
        let calls = backend.calls.load(Ordering::SeqCst);
        for (id, params) in [
            ("layer.select", json!({"layer":layer.0})),
            ("layer.setProps", json!({"layer":layer.0,"name":"dirty"})),
            ("edit.undo", Value::Null),
            ("edit.redo", Value::Null),
        ] {
            assert!(session.execute(id, params).is_err());
            assert!(session.active().unwrap().doc.shares_state(&original));
            assert_eq!(session.active().unwrap().revision, revision);
            assert!(session.journal.is_empty());
            assert_eq!(quota.usage().unwrap(), usage);
            assert_eq!(backend.calls.load(Ordering::SeqCst), calls);
        }
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }
}
