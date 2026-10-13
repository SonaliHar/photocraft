#![cfg(not(target_arch = "wasm32"))]
mod common;
use photocraft_format::{FormatError, atomic::atomic_write_stream_guarded};
use std::{
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
};
#[test]
fn actual_target_gate_runs_before_create_and_before_replace() {
    let root = common::temp_dir("guarded-stream");
    let path = root.join("working.pcraft");
    std::fs::write(&path, b"last good").unwrap();
    let checks = AtomicUsize::new(0);
    let result: photocraft_format::Result<()> = atomic_write_stream_guarded(
        &path,
        |file| {
            file.write_all(b"replacement")?;
            Ok(())
        },
        |target| {
            assert_eq!(target, path.as_path());
            checks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    );
    result.unwrap();
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn target_gate_and_writer_failure_preserve_last_good_and_remove_only_temp() {
    let root = common::temp_dir("guarded-failure");
    let path = root.join("working.pcraft");
    std::fs::write(&path, b"last good").unwrap();
    let checks = AtomicUsize::new(0);
    let result: photocraft_format::Result<()> = atomic_write_stream_guarded(
        &path,
        |file| {
            file.write_all(b"replacement")?;
            Ok(())
        },
        |_| {
            if checks.fetch_add(1, Ordering::SeqCst) == 0 { Ok(()) } else { Err(FormatError::Cancelled) }
        },
    );
    assert!(matches!(result, Err(FormatError::Cancelled)));
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&path).unwrap(), b"last good");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    let result: photocraft_format::Result<()> = atomic_write_stream_guarded(
        &path,
        |file| {
            file.write_all(b"partial")?;
            Err(FormatError::Cancelled)
        },
        |_| Ok(()),
    );
    assert!(matches!(result, Err(FormatError::Cancelled)));
    assert_eq!(std::fs::read(&path).unwrap(), b"last good");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn retargeted_hardlink_is_rechecked_without_overwriting_protected_source() {
    let root = common::temp_dir("guarded-alias-race");
    let path = root.join("working.pcraft");
    let previous = root.join("previous.pcraft");
    let source = root.join("source.psd");
    std::fs::write(&path, b"last good").unwrap();
    std::fs::write(&source, b"gimp xcf v011\0protected").unwrap();
    let result: photocraft_format::Result<()> = atomic_write_stream_guarded(
        &path,
        |file| {
            file.write_all(b"completed replacement")?;
            std::fs::rename(&path, &previous)?;
            std::fs::hard_link(&source, &path)?;
            Ok(())
        },
        |target| {
            if std::fs::read(target)?.starts_with(b"gimp xcf ") { Err(FormatError::Unsupported("protected source alias".into())) } else { Ok(()) }
        },
    );
    assert!(matches!(result, Err(FormatError::Unsupported(_))));
    assert_eq!(std::fs::read(&source).unwrap(), b"gimp xcf v011\0protected");
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&source).unwrap());
    assert_eq!(std::fs::read(&previous).unwrap(), b"last good");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3);
    std::fs::remove_dir_all(root).unwrap();
}
