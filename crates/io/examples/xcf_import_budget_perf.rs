//! Development-only eager/staged comparison, not an admission or display oracle.
//! Run with INPUT.xcf OUTPUT.pcraft [eager|staged] [full|import-only|late-cancel]; paths are authorized scratch inputs.
//! Staged mode creates a private OUTPUT.pcraft.scratch directory and removes it.
//! JSON lines delimit phases for an external process-memory observer. Component budget
//! peaks are distinct from RSS. No limits are raised and no source is rewritten.
#[cfg(not(target_arch = "wasm32"))]
mod desktop {
    use photocraft_raster::Interrupt;
    use photocraft_xcf::{Budget, ErrorKind, Source, XcfError};
    use std::{
        error::Error,
        fs::File,
        io::{Read, Seek, SeekFrom, Write},
        path::Path,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    struct Input {
        file: File,
        len: u64,
        started: Option<mpsc::Sender<Instant>>,
    }
    impl Source for Input {
        fn len(&self) -> u64 {
            self.len
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
            budget.poll("performance input")?;
            if offset.checked_add(out.len() as u64).is_none_or(|end| end > self.len) {
                return Err(XcfError { kind: ErrorKind::Truncated, offset: 0, item: String::new(), message: "performance source extent".into() });
            }
            self.file.seek(SeekFrom::Start(offset)).and_then(|_| self.file.read_exact(out)).map_err(|_| XcfError {
                kind: ErrorKind::Truncated,
                offset,
                item: String::new(),
                message: "performance source read".into(),
            })?;
            if let Some(sender) = self.started.take() {
                let _ = sender.send(Instant::now());
            }
            budget.poll("performance input complete")
        }
    }
    fn event(phase: &str, status: &str, elapsed: Duration, detail: serde_json::Value) -> Result<(), Box<dyn Error>> {
        let unix_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
        println!("{}", serde_json::json!({"phase":phase,"status":status,"unix_ms":unix_ms,"ms":elapsed.as_secs_f64()*1000.0,"detail":detail}));
        std::io::stdout().flush()?;
        Ok(())
    }
    fn input(path: &Path) -> Result<Input, Box<dyn Error>> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Input { file, len, started: None })
    }
    fn verify_declared(doc: &photocraft_doc::Document) -> Result<String, Box<dyn Error>> {
        let surface = doc.layers.first().and_then(|layer| layer.surface()).ok_or("missing synthetic raster")?;
        let format = surface.format();
        if !matches!(format.channels(), 2 | 4) || doc.layers.len() != 1 {
            return Err("synthetic RGB/Gray alpha raster expected".into());
        }
        let mut hash = blake3::Hasher::new();
        for y in (0..doc.size.height).step_by(64) {
            for x in (0..doc.size.width).step_by(64) {
                let x1 = x.saturating_add(64).min(doc.size.width);
                let y1 = y.saturating_add(64).min(doc.size.height);
                let bytes =
                    surface.try_read_interleaved(photocraft_geom::Rect::new(i32::try_from(x)?, i32::try_from(y)?, i32::try_from(x1)?, i32::try_from(y1)?))?;
                hash.update(&bytes);
                for (index, pixel) in bytes.chunks_exact(format.bytes_per_pixel()).enumerate() {
                    let px = x + u32::try_from(index % usize::try_from(x1 - x)?)?;
                    let py = y + u32::try_from(index / usize::try_from(x1 - x)?)?;
                    for channel in 0..u32::try_from(format.channels())? {
                        let marker = (px * 97 + py * 193 + channel * 8191) & 65535;
                        let (expected, n) = match format.sample {
                            photocraft_color::SampleType::U8 => {
                                let value = if usize::try_from(channel)? == format.channels() - 1 { (px * 13 + py * 7) & 255 } else { marker & 255 };
                                (value.to_le_bytes(), 1usize)
                            }
                            photocraft_color::SampleType::U16 => {
                                let value = if channel == 3 { 65535 } else { marker };
                                (value.to_le_bytes(), 2usize)
                            }
                            photocraft_color::SampleType::F32 => {
                                let bits = if channel == 3 { 1.0f32.to_bits() } else { 0x3e800000 | (marker << 5) };
                                (bits.to_le_bytes(), 4usize)
                            }
                            _ => return Err("synthetic U8/U16/F32 storage expected".into()),
                        };
                        let start = usize::try_from(channel)?.checked_mul(n).ok_or("sample offset")?;
                        if pixel.get(start..start + n) != expected.get(..n) {
                            return Err("declared native sample or opaque alpha mismatch".into());
                        }
                    }
                }
            }
        }
        Ok(hash.finalize().to_hex().to_string())
    }
    fn retained_surface_hash(doc: &photocraft_doc::Document) -> Result<String, Box<dyn Error>> {
        let surface = doc.layers.first().and_then(|layer| layer.surface()).ok_or("missing raster")?;
        let mut hash = blake3::Hasher::new();
        hash.update(surface.default_bytes());
        for (coord, tile) in surface.tiles() {
            hash.update(&coord.tx.to_le_bytes());
            hash.update(&coord.ty.to_le_bytes());
            hash.update(tile.bytes());
        }
        Ok(hash.finalize().to_hex().to_string())
    }
    fn staged_import(source_path: &Path, native_path: &Path) -> Result<Option<(photocraft_doc::Document, String)>, Box<dyn Error>> {
        use photocraft_xcf::BudgetKind;
        let scratch_path = native_path.with_file_name(format!("{}.scratch", native_path.file_name().ok_or("native filename")?.to_string_lossy()));
        std::fs::create_dir(&scratch_path)?;
        let operation = (|| -> Result<Option<(photocraft_doc::Document, String)>, Box<dyn Error>> {
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())?;
            let policy = photocraft_io::xcf_policy::for_resources(&limits)?;
            let budget = Budget::new(policy.resources, None);
            let mut source = input(source_path)?;
            event(
                "capture",
                "begin",
                Duration::ZERO,
                serde_json::json!({"route":"explicit private kernel-lifetime scratch snapshot","encoded_bytes":source.len(),"aggregate_capture_import_ledger":true}),
            )?;
            let before = Instant::now();
            let directory =
                photocraft_io::xcf_source::ServiceAuthorizedScratchDirectory::from_platform_authorized(std::fs::canonicalize(&scratch_path)?, &budget)?;
            let mut snapshot = directory.capture(&mut source)?;
            let source_hash = {
                let _lease = budget.reserve(BudgetKind::Resident, 4096, "benchmark source hash scratch")?;
                let mut bytes = [0u8; 4096];
                let mut hash = blake3::Hasher::new();
                let mut offset = 0;
                while offset < snapshot.len() {
                    let n = usize::try_from((snapshot.len() - offset).min(bytes.len() as u64))?;
                    let chunk = bytes.get_mut(..n).ok_or("source hash bounds")?;
                    snapshot.read_exact_at(offset, chunk, &budget)?;
                    budget.consume_work(n as u64, "benchmark source hash")?;
                    hash.update(chunk);
                    offset += n as u64;
                }
                hash.finalize().to_hex().to_string()
            };
            event(
                "capture",
                "ok",
                before.elapsed(),
                serde_json::json!({"blake3":source_hash,"resident_peak":budget.peak(BudgetKind::Resident),"scratch_peak":budget.peak(BudgetKind::Scratch)}),
            )?;
            event(
                "decode_map",
                "begin",
                Duration::ZERO,
                serde_json::json!({"route":"explicit staged public Source endpoint","scope":"combined parser/mapping/provenance; no phase hook","same_capture_budget":true}),
            )?;
            let before = Instant::now();
            let options = photocraft_xcf::DecodeOptions { limits: policy.parser, ..Default::default() };
            let result = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
                "Synthetic performance source",
                &mut snapshot,
                &Interrupt::NONE,
                &options,
                &budget,
                photocraft_xcf::PlainSourceAccounting::CallerReservedScratch,
            );
            let elapsed = before.elapsed();
            drop(snapshot);
            drop(directory);
            match result {
                Ok(result) => {
                    event(
                        "decode_map",
                        "ok",
                        elapsed,
                        serde_json::json!({"resident_charged_peak":budget.peak(BudgetKind::Resident),"scratch_peak":budget.peak(BudgetKind::Scratch),"work_units":budget.used(BudgetKind::Work),"resident_after_handoff":budget.used(BudgetKind::Resident),"scratch_after_drop":budget.used(BudgetKind::Scratch),"document_ownership_transferred_outside_operation_ledger":true,"warnings":result.warnings}),
                    )?;
                    Ok(Some((result.document, source_hash)))
                }
                Err(error) => {
                    event(
                        "decode_map",
                        "error",
                        elapsed,
                        serde_json::json!({"error":error.to_string(),"resident_after_drop":budget.used(BudgetKind::Resident),"scratch_after_drop":budget.used(BudgetKind::Scratch)}),
                    )?;
                    Ok(None)
                }
            }
        })();
        std::fs::remove_dir(&scratch_path)?;
        event("snapshot_cleanup", "ok", Duration::ZERO, serde_json::json!({"private_directory_removed":true,"remaining_snapshot_files":0}))?;
        operation
    }
    fn cancel_capture(path: &Path) -> Result<(), Box<dyn Error>> {
        event("cancel_capture", "begin", Duration::ZERO, serde_json::json!({"signal_after_first_read_ms":20}))?;
        let cancelled = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        let mut source = input(path)?;
        source.started = Some(tx);
        std::thread::scope(|scope| -> Result<(), Box<dyn Error>> {
            let signal_flag = &cancelled;
            let worker = scope.spawn(move || {
                rx.recv_timeout(Duration::from_secs(5)).ok()?;
                std::thread::sleep(Duration::from_millis(20));
                let signal = Instant::now();
                signal_flag.store(true, Ordering::Release);
                Some(signal)
            });
            let check = || cancelled.load(Ordering::Acquire);
            let budget = Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default())?, Some(&check));
            let before = Instant::now();
            let result = photocraft_io::xcf_source::OwnedXcfSnapshot::from_source(&mut source, &budget);
            let returned = Instant::now();
            let outcome = result.as_ref().map(|_| "completed before cancellation".to_owned()).unwrap_or_else(|e| e.to_string());
            drop(result);
            let signal = worker.join().map_err(|_| std::io::Error::other("cancellation measurement thread failed"))?;
            event(
                "cancel_capture",
                "measured",
                before.elapsed(),
                serde_json::json!({
                    "outcome":outcome,"signal_to_return_ms":signal.map(|s| returned.saturating_duration_since(s).as_secs_f64()*1000.0),
                    "signal_preceded_return":signal.is_some_and(|s| s <= returned),"resident_after_drop":budget.used(photocraft_xcf::BudgetKind::Resident)
                }),
            )
        })
    }
    struct NotifySource<'a> {
        source: &'a mut dyn Source,
        first: Option<mpsc::Sender<()>>,
    }
    impl Source for NotifySource<'_> {
        fn len(&self) -> u64 {
            self.source.len()
        }
        fn read_exact_at(&mut self, offset: u64, output: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
            self.source.read_exact_at(offset, output, budget)?;
            if let Some(sender) = self.first.take() {
                let _ = sender.send(());
            }
            Ok(())
        }
    }
    fn cancel_staged_import(source_path: &Path, native_path: &Path) -> Result<(), Box<dyn Error>> {
        use photocraft_xcf::BudgetKind;
        let scratch_path = native_path.with_file_name(format!("{}.cancel-scratch", native_path.file_name().ok_or("native filename")?.to_string_lossy()));
        std::fs::create_dir(&scratch_path)?;
        let operation = (|| -> Result<(), Box<dyn Error>> {
            let cancelled = AtomicBool::new(false);
            let check = || cancelled.load(Ordering::Acquire);
            let control = Interrupt::cancel_only(&check);
            let policy = photocraft_io::xcf_policy::for_resources(&photocraft_xcf::ResourceLimits::for_policy(Default::default())?)?;
            let budget = Budget::new(policy.resources, Some(&check));
            let directory =
                photocraft_io::xcf_source::ServiceAuthorizedScratchDirectory::from_platform_authorized(std::fs::canonicalize(&scratch_path)?, &budget)?;
            let mut source = input(source_path)?;
            let mut snapshot = directory.capture(&mut source)?;
            let source_resident = budget.used(BudgetKind::Resident);
            let source_scratch = budget.used(BudgetKind::Scratch);
            let (tx, rx) = mpsc::channel();
            let mut source = NotifySource { source: &mut snapshot, first: Some(tx) };
            event(
                "cancel_staged_import",
                "begin",
                Duration::ZERO,
                serde_json::json!({"signal_after_first_import_read_ms":20,"source_resident":source_resident,"source_scratch":source_scratch}),
            )?;
            std::thread::scope(|scope| -> Result<(), Box<dyn Error>> {
                let flag = &cancelled;
                let worker = scope.spawn(move || {
                    rx.recv_timeout(Duration::from_secs(5)).ok()?;
                    std::thread::sleep(Duration::from_millis(20));
                    let signal = Instant::now();
                    flag.store(true, Ordering::Release);
                    Some(signal)
                });
                let before = Instant::now();
                let options = photocraft_xcf::DecodeOptions { limits: policy.parser, ..Default::default() };
                let result = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
                    "Synthetic performance source",
                    &mut source,
                    &control,
                    &options,
                    &budget,
                    photocraft_xcf::PlainSourceAccounting::CallerReservedScratch,
                );
                let returned = Instant::now();
                let outcome = result.as_ref().map(|_| "completed before signal".to_owned()).unwrap_or_else(|e| e.to_string());
                drop(result);
                let signal = worker.join().map_err(|_| std::io::Error::other("cancellation measurement thread failed"))?;
                let refunded = budget.used(BudgetKind::Resident) == source_resident && budget.used(BudgetKind::Scratch) == source_scratch;
                if !refunded {
                    return Err("staged cancellation did not refund to source baseline".into());
                }
                event(
                    "cancel_staged_import",
                    "measured",
                    before.elapsed(),
                    serde_json::json!({"outcome":outcome,"signal_to_return_ms":signal.map(|s|returned.saturating_duration_since(s).as_secs_f64()*1000.0),"signal_preceded_return":signal.is_some_and(|s|s<=returned),"source_baseline_exact":refunded}),
                )
            })?;
            drop(snapshot);
            drop(directory);
            event(
                "cancel_staged_cleanup",
                "ok",
                Duration::ZERO,
                serde_json::json!({"resident":budget.used(BudgetKind::Resident),"scratch":budget.used(BudgetKind::Scratch)}),
            )
        })();
        std::fs::remove_dir(&scratch_path)?;
        operation
    }

    // This phase detector is intentionally specific to our uncompressed fixture:
    // parser reaches EOF, then provenance restarts at zero in sequential 4096-byte reads.
    struct LateHashSource<'a> {
        source: &'a mut dyn Source,
        reached_end: bool,
        next_hash_offset: u64,
        hash_started: bool,
        notify: Option<mpsc::Sender<()>>,
        calls: u64,
    }
    impl Source for LateHashSource<'_> {
        fn len(&self) -> u64 {
            self.source.len()
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
            self.source.read_exact_at(offset, out, budget)?;
            self.calls = self.calls.saturating_add(1);
            if self.reached_end && offset == 0 && out.len() == 4096 {
                self.hash_started = true;
                self.next_hash_offset = 0;
            }
            if self.hash_started {
                if offset == self.next_hash_offset && out.len() == 4096 {
                    self.next_hash_offset = self.next_hash_offset.saturating_add(4096);
                    if self.next_hash_offset >= 1_048_576
                        && let Some(tx) = self.notify.take()
                    {
                        let _ = tx.send(());
                    }
                } else {
                    self.hash_started = false;
                }
            }
            if offset.checked_add(out.len() as u64) == Some(self.len()) {
                self.reached_end = true;
            }
            Ok(())
        }
    }
    fn cancel_late_hash(path: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
        use photocraft_xcf::BudgetKind;
        let scratch = output.with_extension("late-hash-scratch");
        std::fs::create_dir(&scratch)?;
        let operation = (|| -> Result<(), Box<dyn Error>> {
            let flag = AtomicBool::new(false);
            let check = || flag.load(Ordering::Acquire);
            let control = Interrupt::cancel_only(&check);
            let policy = photocraft_io::xcf_policy::for_resources(&photocraft_xcf::ResourceLimits::for_policy(Default::default())?)?;
            let budget = Budget::new(policy.resources, Some(&check));
            let directory = photocraft_io::xcf_source::ServiceAuthorizedScratchDirectory::from_platform_authorized(std::fs::canonicalize(&scratch)?, &budget)?;
            let mut original = input(path)?;
            let mut snapshot = directory.capture(&mut original)?;
            let baseline = (budget.used(BudgetKind::Resident), budget.used(BudgetKind::Scratch));
            let (tx, rx) = mpsc::channel();
            let mut source = LateHashSource { source: &mut snapshot, reached_end: false, next_hash_offset: 0, hash_started: false, notify: Some(tx), calls: 0 };
            std::thread::scope(|scope| -> Result<(), Box<dyn Error>> {
                let cancellation = &flag;
                let worker = scope.spawn(move || {
                    rx.recv_timeout(Duration::from_secs(30)).ok()?;
                    let signal = Instant::now();
                    cancellation.store(true, Ordering::Release);
                    Some(signal)
                });
                let before = Instant::now();
                let options = photocraft_xcf::DecodeOptions { limits: policy.parser, ..Default::default() };
                let result = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
                    "Late hash fixture",
                    &mut source,
                    &control,
                    &options,
                    &budget,
                    photocraft_xcf::PlainSourceAccounting::CallerReservedScratch,
                );
                let returned = Instant::now();
                let outcome = result.as_ref().map(|_| "completed".to_owned()).unwrap_or_else(|e| e.to_string());
                drop(result);
                // Close notification if the recognized phase did not occur.
                drop(source.notify.take());
                let signal = worker.join().map_err(|_| std::io::Error::other("late hash thread"))?;
                let refunded = baseline == (budget.used(BudgetKind::Resident), budget.used(BudgetKind::Scratch));
                if !refunded {
                    return Err("late hash source baseline refund mismatch".into());
                }
                event(
                    "cancel_late_provenance_hash",
                    "measured",
                    before.elapsed(),
                    serde_json::json!({"outcome":outcome,"parser_reached_input_end":source.reached_end,"sequential_second_pass_bytes":source.next_hash_offset,"read_calls":source.calls,"phase_detected":signal.is_some(),"signal_preceded_return":signal.is_some_and(|s|s<=returned),"signal_to_return_ms":signal.map(|s|returned.saturating_duration_since(s).as_secs_f64()*1000.0),"source_baseline_exact":refunded,"phase_scope":"own uncompressed fixture EOF then sequential4096 second pass; not generic phase hook"}),
                )
            })?;
            drop(snapshot);
            drop(directory);
            if budget.used(BudgetKind::Resident) != 0 || budget.used(BudgetKind::Scratch) != 0 {
                return Err("late hash cleanup lease leak".into());
            }
            event("late_hash_cleanup", "ok", Duration::ZERO, serde_json::json!({"resident":0,"scratch":0,"no_native_publication":!output.exists()}))
        })();
        std::fs::remove_dir(&scratch)?;
        operation
    }
    pub fn run() -> Result<(), Box<dyn Error>> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let (Some(source_path), Some(native_path)) = (args.first(), args.get(1)) else {
            return Err("usage: xcf_import_budget_perf INPUT.xcf NEW_OUTPUT.pcraft [eager|staged]".into());
        };
        let source_path = Path::new(source_path);
        let native_path = Path::new(native_path);
        let route = args.get(2).map(|s| s.to_str().ok_or("route text")).transpose()?.unwrap_or("eager");
        if !(2..=4).contains(&args.len()) || !matches!(route, "eager" | "staged") || native_path.exists() || source_path == native_path {
            return Err("requires a new, distinct scratch output path".into());
        }
        let mode = args.get(3).map(|s| s.to_str().ok_or("mode text")).transpose()?.unwrap_or("full");
        if !matches!(mode, "full" | "import-only" | "late-cancel") {
            return Err("unknown measurement mode".into());
        }
        if mode == "late-cancel" {
            return cancel_late_hash(source_path, native_path);
        }
        let (doc, source_hash) = if route == "staged" {
            let Some(result) = staged_import(source_path, native_path)? else {
                return cancel_capture(source_path);
            };
            result
        } else {
            let mut source = input(source_path)?;
            event(
                "capture",
                "begin",
                Duration::ZERO,
                serde_json::json!({"encoded_bytes":source.len(),"policy":"native_standard","route":"owned immutable Vec snapshot"}),
            )?;
            let budget = Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default())?, None);
            let before = Instant::now();
            let snapshot = photocraft_io::xcf_source::OwnedXcfSnapshot::from_source(&mut source, &budget)?;
            let source_hash = blake3::hash(snapshot.bytes()).to_hex().to_string();
            event(
                "capture",
                "ok",
                before.elapsed(),
                serde_json::json!({"blake3":source_hash,"resident_peak":budget.peak(photocraft_xcf::BudgetKind::Resident)}),
            )?;
            event(
                "decode_map",
                "begin",
                Duration::ZERO,
                serde_json::json!({"component_budget":"import_with_stats owns a separate standard ledger; capture remains live"}),
            )?;
            let before = Instant::now();
            let imported = photocraft_io::xcf_import::import_with_stats("Synthetic performance source", snapshot.bytes(), &Interrupt::NONE);
            let (result, stats) = match imported {
                Ok(value) => value,
                Err(error) => {
                    event("decode_map", "error", before.elapsed(), serde_json::json!({"error":error.to_string()}))?;
                    drop(snapshot);
                    return cancel_capture(source_path);
                }
            };
            let doc = result.document;
            event(
                "decode_map",
                "ok",
                before.elapsed(),
                serde_json::json!({"decode_ms":stats.decode.as_secs_f64()*1000.0,"map_ms":stats.mapping.as_secs_f64()*1000.0,"resident_charged_peak":stats.peak_charged_bytes,"width":doc.size.width,"height":doc.size.height,"depth":format!("{:?}",doc.depth),"warnings":result.warnings}),
            )?;
            drop(snapshot);
            (doc, source_hash)
        };
        if doc.xcf_state.as_ref().and_then(|s| s.provenance.source_hash.as_ref()) != Some(&source_hash) {
            return Err("source provenance hash mismatch".into());
        }
        let before = Instant::now();
        let current_hash = verify_declared(&doc)?;
        event(
            "native_sample_verify",
            "ok",
            before.elapsed(),
            serde_json::json!({"declared_native_exact":true,"current_rgba_native_blake3":current_hash,"retained_keys_default_payload_blake3":retained_surface_hash(&doc)?,"verification_scratch_max":64*64*16}),
        )?;
        event(
            "current_context",
            "ok",
            Duration::ZERO,
            serde_json::json!({"mode":doc.mode,"depth":doc.depth,"size":doc.size,"encoding":doc.sample_encoding,"working_interpretation":doc.working_profile_interpretation,"icc_blake3":doc.icc_profile.as_ref().map(|b|blake3::hash(b.as_slice()).to_hex().to_string()),"layer_import_policy":doc.layers.first().and_then(|l|l.import_compositing.as_ref()),"source_composition":doc.xcf_state.as_ref().and_then(|s|s.items.first()).and_then(|i|i.composition.as_ref())}),
        )?;
        if mode == "import-only" {
            drop(doc);
            return event(
                "import_only_complete",
                "ok",
                Duration::ZERO,
                serde_json::json!({"native_samples_context_source_verified":true,"no_archive_or_display_oracle":true}),
            );
        }
        event("compose", "begin", Duration::ZERO, serde_json::json!({"band_height":64,"metric":"checksum only, no independent display oracle"}))?;
        let before = Instant::now();
        let composed = (|| -> Result<f64, Box<dyn Error>> {
            let renderer = photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&doc, &Interrupt::NONE)?;
            let bounds = doc.bounds();
            let mut checksum = 0.0;
            let mut y = bounds.y0;
            while y < bounds.y1 {
                let end = y.saturating_add(64).min(bounds.y1);
                let band = renderer.render(photocraft_geom::Rect { x0: bounds.x0, y0: y, x1: bounds.x1, y1: end }, &mut |_| Ok(()))?;
                checksum += band.px.iter().flat_map(|p| p.iter()).sum::<f64>();
                y = end;
            }
            Ok(checksum)
        })();
        match composed {
            Ok(checksum) => event("compose", "ok", before.elapsed(), serde_json::json!({"checksum":checksum}))?,
            Err(e) => event("compose", "error", before.elapsed(), serde_json::json!({"error":e.to_string()}))?,
        }
        event("save", "begin", Duration::ZERO, serde_json::json!({"path":native_path,"limits":"default NativeSaveLimits"}))?;
        let before = Instant::now();
        match photocraft_io::save_native_path(&doc, native_path, Default::default(), &Interrupt::NONE) {
            Ok(saved) => {
                event(
                    "save",
                    "ok",
                    before.elapsed(),
                    serde_json::json!({"archive_bytes":native_path.metadata()?.len(),"retained_payload_bytes":saved.retained_payload_bytes,"warnings":saved.warnings}),
                )?;
                event("reopen", "begin", Duration::ZERO, serde_json::json!({"route":"public seekable ZIP64 stream; original document remains live"}))?;
                let before = Instant::now();
                match photocraft_format::load_from_stream(File::open(native_path)?, &Default::default(), Default::default(), &Interrupt::NONE) {
                    Ok(reopened) => {
                        if reopened != doc {
                            return Err("whole native document equality failed".into());
                        }
                        event("reopen", "ok", before.elapsed(), serde_json::json!({"whole_document_exact":true}))?;
                    }
                    Err(e) => event("reopen", "error", before.elapsed(), serde_json::json!({"error":e.to_string()}))?,
                }
            }
            Err(e) => event("save", "error", before.elapsed(), serde_json::json!({"error":e.to_string(),"destination_exists":native_path.exists()}))?,
        }
        drop(doc);
        if route == "staged" { cancel_staged_import(source_path, native_path) } else { cancel_capture(source_path) }
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    desktop::run()
}
#[cfg(target_arch = "wasm32")]
fn main() {}
