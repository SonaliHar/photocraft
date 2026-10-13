//! Fixed, opt-in bootstrap contract diagnostics. Never linked in the ordinary app.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size, TextLayer};
use photocraft_engine::Session;
use photocraft_text::FontBootstrapError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PSD: &[u8] = include_bytes!("boot-contract-fixture/synthetic-type.psd");
const PSD_SHA256: &str = "2833cba878e8e001064e8543168481f8a2055577c503be8bc354b7479a6a1c11";
const MAX_REPORT_BYTES: usize = 16 * 1024;

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn fixture() -> Result<&'static [u8], String> {
    if PSD.len() > 64 * 1024 {
        return Err("contract fixture exceeds bound".into());
    }
    let hash = Sha256::digest(PSD).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    if hash != PSD_SHA256 {
        return Err("contract fixture hash mismatch".into());
    }
    Ok(PSD)
}

fn document() -> Document {
    let mut doc = Document::new("PhotoCraft boot contract", Size::new(128, 64), ColorMode::Rgb, SampleType::U8);
    let text = TextLayer { text: "PhotoCraft boot contract".into(), ..Default::default() };
    doc.layers.push(Layer::new("Editable diagnostic type", LayerContent::Text(text)));
    doc
}

fn session() -> Result<(Session, Document), String> {
    let mut session = Session::new();
    session.add_document(document(), None).map_err(error)?;
    session
        .edit("Contract redo witness", |doc, _| {
            doc.name = "Contract preserved redo".into();
            Ok(())
        })
        .map_err(error)?;
    let redo = (*session.active().ok_or("contract missing active document")?.doc).clone();
    session.execute("edit.undo", json!({})).map_err(error)?;
    Ok((session, redo))
}

fn expected_error(error: impl std::fmt::Display, expected: &str) -> Result<(), String> {
    let text = error.to_string();
    if text.contains(expected) { Ok(()) } else { Err(format!("expected {expected}; observed {text}")) }
}

fn commands(expect: Option<&str>) -> Result<Vec<Value>, String> {
    let mut checks = Vec::new();
    for (id, params) in [
        ("type.create", json!({"x":4,"y":30,"text":"Ready contract create"})),
        ("type.edit", json!({"text":"Ready contract edit"})),
        ("type.setStyle", json!({"size":18})),
        ("type.fonts", json!({})),
        ("type.info", json!({})),
        ("type.resolveMissingFonts", json!({})),
    ] {
        let (mut session, redo) = session()?;
        let before = session.active().ok_or("contract missing before document")?;
        let doc = before.doc.clone();
        let revision = before.revision;
        let saved_revision = before.saved_revision;
        let past = before.history.past_len();
        let labels = before.history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
        let redo_labels = before.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        let result = session.execute(id, params);
        if let Some(expected) = expect {
            expected_error(result.err().ok_or_else(|| format!("{id} incorrectly succeeded before readiness"))?, expected)?;
            let after = session.active().ok_or("contract lost active document")?;
            if after.doc != doc
                || after.revision != revision
                || after.saved_revision != saved_revision
                || after.history.past_len() != past
                || after.history.entry_labels().map(str::to_owned).collect::<Vec<_>>() != labels
                || after.history.redo_labels().map(str::to_owned).collect::<Vec<_>>() != redo_labels
            {
                return Err(format!("{id} changed document/revision/history/redo before readiness"));
            }
            session.execute("edit.redo", json!({})).map_err(error)?;
            if *session.active().ok_or("contract lost redo document")?.doc != redo {
                return Err(format!("{id} altered retained redo snapshot"));
            }
        } else {
            result.map_err(error)?;
        }
        checks.push(json!({"case":id,"passed":true,"error":expect,"atomicBeforeReady":expect.is_some()}));
    }
    Ok(checks)
}

fn adapters(expect: Option<&str>) -> Result<Vec<Value>, String> {
    let doc = document();
    let source = doc.layers.first().ok_or("contract missing text")?;
    let LayerContent::Text(source) = &source.content else { return Err("contract text fixture changed".into()) };
    let mut text = source.clone();
    let refreshed = photocraft_engine::type_cmds::try_refresh(&doc, &mut text);
    let missing = photocraft_engine::type_extra_cmds::try_missing_fonts(&doc);
    let imported = photocraft_io::import("synthetic-type.psd", fixture()?);
    if let Some(expected) = expect {
        expected_error(refreshed.err().ok_or("refresh incorrectly succeeded before readiness")?, expected)?;
        if text != *source {
            return Err("refresh mutated type before readiness".into());
        }
        expected_error(missing.err().ok_or("font query incorrectly succeeded before readiness")?, expected)?;
        let import_error = imported.err().ok_or("PSD import incorrectly succeeded before readiness")?;
        if !matches!(import_error, photocraft_io::IoError::FontBootstrap(_)) {
            return Err(format!("PSD import did not propagate typed bootstrap error: {import_error}"));
        }
        expected_error(import_error, expected)?;
    } else {
        refreshed.map_err(error)?;
        missing.map_err(error)?;
        if text.psd_raw.is_none() || text.cache.as_ref().is_none_or(|cache| cache.content_bounds().is_empty()) {
            return Err("ready refresh omitted editable PSD data or text ink".into());
        }
        let imported = imported.map_err(error)?;
        if !imported.document.walk().iter().any(
            |(_, _, layer)| matches!(&layer.content, LayerContent::Text(text) if text.cache.as_ref().is_some_and(|cache| !cache.content_bounds().is_empty())),
        ) {
            return Err("ready PSD import skipped text preparation".into());
        }
    }
    Ok(["try_refresh", "try_missing_fonts", "PSD import"].map(|case| json!({"case":case,"passed":true,"error":expect})).into())
}

fn report(phase: &str, bootstrap: Option<&str>) -> Result<String, String> {
    let mut checks = commands(bootstrap)?;
    checks.extend(adapters(bootstrap)?);
    let report = json!({"contract":"web-font-bootstrap-v1","phase":phase,"fixtureSha256":PSD_SHA256,"checks":checks}).to_string();
    if report.len() > MAX_REPORT_BYTES { Err("contract report exceeds bound".into()) } else { Ok(report) }
}

pub(crate) fn before_publication() -> Result<String, String> {
    let bootstrap = photocraft_text::try_shared().err().ok_or("text engine incorrectly ready before bank publication")?;
    if !matches!(bootstrap, FontBootstrapError::ProviderMissing | FontBootstrapError::NotReady) {
        return Err(format!("unexpected pre-publication bootstrap error: {bootstrap}"));
    }
    report("before-publication", Some(bootstrap.code()))
}

pub(crate) fn after_publication() -> Result<String, String> {
    photocraft_text::try_shared().map_err(error)?;
    report("after-publication", None)
}
