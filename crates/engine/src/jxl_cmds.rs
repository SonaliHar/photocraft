//! Lossless JPEG ↔ JPEG XL: File › Scripts › Recompress JPEG as JPEG XL… stores each JPEG's own
//! DCT coefficients as a JPEG XL file about 20% smaller, with the pixels unchanged; Restore
//! Original JPEG from JPEG XL… rebuilds the original JPEG file byte for byte from such a file.
//! Neither opens a document: they work on files, like the other scripts, and the UI, CLI, MCP and
//! control channel all reach them here.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::file_cmds::{read_file, write_file};
use crate::{EngineError, Result, Session};

const TO_JXL: &str = "file.scripts.jpegToJpegXl";
const TO_JPEG: &str = "file.scripts.jpegXlToJpeg";

/// Most files one call converts: each is read whole into memory, one at a time.
const MAX_FILES: usize = 10_000;

/// The paths a call converts: `paths` (an array) or `path` (one).
fn paths(p: &Value, cmd: &str) -> Result<Vec<String>> {
    let bad = |msg: String| EngineError::BadParams { cmd: cmd.into(), msg };
    let list: Vec<String> = match (p.get("paths"), p.get("path")) {
        (Some(Value::Array(a)), _) => {
            a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad("every entry of `paths` is a file path".into()))).collect::<Result<_>>()?
        }
        (Some(_), _) => return Err(bad("`paths` is an array of file paths".into())),
        (None, Some(Value::String(s))) => vec![s.clone()],
        (None, Some(_)) => return Err(bad("`path` is a file path".into())),
        (None, None) => return Err(bad("need `paths` or `path`".into())),
    };
    if list.is_empty() || list.iter().any(String::is_empty) {
        return Err(bad("no file to convert".into()));
    }
    if list.len() > MAX_FILES {
        return Err(bad(format!("{} files are more than the {MAX_FILES} one call converts", list.len())));
    }
    Ok(list)
}

/// `path` with its extension replaced by `ext`.
fn with_extension(path: &str, ext: &str) -> String {
    std::path::Path::new(path).with_extension(ext).to_string_lossy().into_owned()
}

/// Converts every file with `convert`, writing the result next to it as `ext`. An existing output
/// is left alone unless `overwrite`. Stops at the first failure, reporting the files done before.
fn convert_all(p: &Value, cmd: &str, ext: &str, convert: impl Fn(&[u8]) -> Result<Vec<u8>>) -> Result<Value> {
    let overwrite = p.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
    let mut done = vec![];
    let (mut before, mut after) = (0u64, 0u64);
    for path in paths(p, cmd)? {
        let out = with_extension(&path, ext);
        if out == path {
            return Err(EngineError::Other(format!("{path}: the output would replace the file itself")));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !overwrite && std::path::Path::new(&out).exists() {
            return Err(EngineError::Other(format!("{out} exists already (pass \"overwrite\": true to replace it); converted before it: {}", done.len())));
        }
        let bytes = read_file(&path)?;
        let converted = convert(&bytes).map_err(|e| EngineError::Other(format!("{path}: {e}; converted before it: {}", done.len())))?;
        write_file(&out, &converted)?;
        before = before.saturating_add(bytes.len() as u64);
        after = after.saturating_add(converted.len() as u64);
        done.push(json!({"path": path, "out": out, "bytes": bytes.len(), "outBytes": converted.len()}));
    }
    // Read only where there are files to find (the web build has none).
    let _ = overwrite;
    Ok(json!({"files": done, "bytes": before, "outBytes": after}))
}

fn jpeg_to_jxl(_s: &mut Session, p: &Value) -> Result<Value> {
    convert_all(p, TO_JXL, "jxl", |bytes| {
        photocraft_codecs::jpeg_to_jxl(bytes, &photocraft_codecs::Limits::default()).map_err(|e| EngineError::Other(e.to_string()))
    })
}

fn jxl_to_jpeg(_s: &mut Session, p: &Value) -> Result<Value> {
    convert_all(p, TO_JPEG, "jpg", |bytes| {
        photocraft_codecs::jxl_to_jpeg(bytes, &photocraft_codecs::Limits::default())
            .map_err(|e| EngineError::Other(e.to_string()))?
            .ok_or_else(|| EngineError::Other("this JPEG XL file was not made from a JPEG, so there is no original JPEG to restore".into()))
    })
}

pub fn specs() -> Vec<CommandSpec> {
    const PARAMS: &str = r#"{"paths":[file path…] | "path":file path, "overwrite":bool=false (replace an existing output file)} → {files:[{path, out, bytes, outBytes}], bytes, outBytes} (each output is written next to its file, with the new extension; stops at the first failure)"#;
    vec![
        CommandSpec {
            id: TO_JXL,
            label: "Recompress JPEG as JPEG XL…",
            menu: &["File", "Scripts"],
            shortcut: None,
            params: PARAMS,
            enabled: always,
            journal: true,
            run: jpeg_to_jxl,
        },
        CommandSpec {
            id: TO_JPEG,
            label: "Restore Original JPEG from JPEG XL…",
            menu: &["File", "Scripts"],
            shortcut: None,
            params: PARAMS,
            enabled: always,
            journal: true,
            run: jxl_to_jpeg,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_params_are_errors() {
        let mut s = Session::new();
        for p in [json!({}), json!({"paths": []}), json!({"paths": "a.jpg"}), json!({"paths": [1]}), json!({"path": 3}), json!({"path": ""})] {
            assert!(s.execute(TO_JXL, p.clone()).is_err(), "{p}");
            assert!(s.execute(TO_JPEG, p.clone()).is_err(), "{p}");
        }
        assert!(s.execute(TO_JXL, json!({"path": "/no/such/file.jpg"})).is_err());
    }

    #[test]
    fn outputs_go_next_to_the_inputs() {
        assert_eq!(with_extension("/a/b/photo.JPG", "jxl"), "/a/b/photo.jxl");
        assert_eq!(with_extension("noext", "jpg"), "noext.jpg");
    }
}
