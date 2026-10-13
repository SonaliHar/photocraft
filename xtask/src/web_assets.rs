//! Reproducible, pinned browser UI assets.
//! Payload-only fixed assets. Downloaded JSON never authorizes runtime ranges.
use crate::{sha256, text_canonical};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
const PACK_BYTES: u64 = 4_033_515;
const PACK_SHA: &str = "2da61a2cd8ad540f4ac2d7099a8c9e404201d3b7b611d7230700edc8ea5c735c";
const PACK_SRI: &str = "sha256-LaYaLNitVA9KwtcJmoyeQEIB07e2EdcjBwDtyOpcc1w=";
const FONT_PACKAGE_SHA: &str = "773fa9c96dd0dbef887e39d0ed6177f141cce4d68e6041df77570aa3702dfa13";
const INTER_SHA: &str = "40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82";
const RAW_CAP: u64 = 8 << 20;
#[derive(Clone, Copy)]
struct Input {
    id: &'static str,
    registry: bool,
    path: &'static str,
    length: u64,
    sha: &'static str,
}
#[derive(Clone, Copy)]
struct LicenseInput {
    registry: bool,
    path: &'static str,
    output: &'static str,
    length: u64,
    sha: &'static str,
}
#[path = "web_asset_pins.rs"]
mod pins;
#[derive(Debug, Clone)]
struct Record {
    id: &'static str,
    offset: u64,
    length: u64,
    sha: &'static str,
    lines: u32,
    line_bytes: u32,
    entries: u32,
}
fn check_relative(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 256 || !Path::new(path).components().all(|c| matches!(c, std::path::Component::Normal(_))) {
        return Err("asset path must be a bounded relative path".into());
    }
    Ok(())
}
fn source_path(base: &Path, relative: &str) -> Result<PathBuf, String> {
    check_relative(relative)?;
    let base = base.canonicalize().map_err(|e| format!("asset base: {e}"))?;
    let path = base.join(relative).canonicalize().map_err(|e| format!("asset {relative}: {e}"))?;
    if !path.starts_with(&base) {
        return Err(format!("asset {relative} escapes its input root"));
    }
    Ok(path)
}
fn bounded(path: &Path, expected: u64, cap: u64) -> Result<Vec<u8>, String> {
    if expected == 0 || expected > cap {
        return Err("asset input limit".into());
    }
    let length = usize::try_from(expected).map_err(|_| "asset input does not fit target".to_string())?;
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() != expected {
        return Err(format!("{}: asset length mismatch", path.display()));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| "asset input allocation".to_string())?;
    let mut block = [0u8; 65536];
    loop {
        let n = file.read(&mut block).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let end = bytes.len().checked_add(n).ok_or("asset read overflow")?;
        if end > length {
            return Err("asset grew while reading".into());
        }
        bytes.extend_from_slice(block.get(..n).ok_or("asset read span")?);
    }
    if bytes.len() != length {
        return Err("asset became short while reading".into());
    }
    Ok(bytes)
}
fn pinned(path: &Path, length: u64, cap: u64, hash: &str) -> Result<Vec<u8>, String> {
    let bytes = bounded(path, length, cap)?;
    if sha256::hex(&bytes) != hash {
        return Err(format!("{}: asset hash mismatch", path.display()));
    }
    Ok(bytes)
}
fn pinned_text(path: &Path, size: u64, cap: u64, hash: &str) -> Result<Vec<u8>, String> {
    let raw_cap = cap.checked_mul(2).ok_or("text raw cap overflow")?;
    let raw_size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    let raw = bounded(path, raw_size, raw_cap)?;
    let bytes = text_canonical::canonical_text(&raw, usize::try_from(raw_cap).map_err(|_| "text cap conversion")?)
        .map_err(|error| format!("text canonicalization: {error:?}"))?;
    if u64::try_from(bytes.len()).map_err(|_| "canonical length conversion")? != size || size > cap || sha256::hex(&bytes) != hash {
        return Err(format!("{}: canonical text length/hash mismatch", path.display()));
    }
    Ok(bytes)
}
fn verify_pack(path: &Path, records: &[Record]) -> Result<(), String> {
    validate_records(records)?;
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() != PACK_BYTES || sha256::reader_hex(&mut file, PACK_BYTES)? != PACK_SHA {
        return Err("whole asset pack length/hash mismatch".into());
    }
    for record in records {
        file.seek(std::io::SeekFrom::Start(record.offset)).map_err(|e| e.to_string())?;
        if sha256::reader_hex(Read::by_ref(&mut file).take(record.length), record.length)? != record.sha {
            return Err(format!("asset pack row hash mismatch {}", record.id));
        }
    }
    Ok(())
}
fn hex_array(hex: &str) -> Result<String, String> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("invalid pinned SHA256".into());
    }
    let mut out = String::from("[");
    for chunk in hex.as_bytes().as_chunks::<2>().0 {
        let s = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
        let _ = write!(out, "0x{s},");
    }
    out.push(']');
    Ok(out)
}
fn catalog_bounds(bytes: &[u8]) -> Result<(u32, u32, u32), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "asset catalog UTF8".to_string())?;
    let (mut lines, mut max, mut entries) = (0u32, 0u32, 0u32);
    for line in text.lines() {
        lines = lines.checked_add(1).ok_or("catalog line overflow")?;
        max = max.max(u32::try_from(line.len()).map_err(|_| "catalog line width")?);
        if lines > 4096 || max > 1024 {
            return Err("catalog grammar input limit".into());
        }
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        match (cols.next(), cols.next(), cols.next(), cols.next()) {
            (Some(_), Some(src), Some(tr), None) if !src.is_empty() && !tr.is_empty() => {}
            _ => return Err("invalid asset catalog three-column grammar".into()),
        }
        entries = entries.checked_add(1).ok_or("catalog entry overflow")?;
    }
    Ok((lines, max, entries))
}
fn font_role(id: &str) -> Result<(&'static str, u32), String> {
    match id {
        "font/Inter-Medium" => Ok(("InterMedium", 1f32.to_bits())),
        "font/Inter-SemiBold" => Ok(("InterSemiBold", 1f32.to_bits())),
        "font/JetBrainsMono-Regular" => Ok(("JetBrainsMono", 1f32.to_bits())),
        "font/Hack-Regular" => Ok(("Hack", 1f32.to_bits())),
        "font/Ubuntu-Light" => Ok(("UbuntuLight", 1f32.to_bits())),
        "font/NotoEmoji-Regular" => Ok(("NotoEmoji", 0.81f32.to_bits())),
        "font/emoji-icon-font" => Ok(("EmojiIcon", 0.90f32.to_bits())),
        _ => Err("unknown fixed font role".into()),
    }
}
fn validate_records(records: &[Record]) -> Result<(), String> {
    if records.len() != 21 {
        return Err("asset record count".into());
    }
    let mut ids = BTreeSet::new();
    let mut end = 0u64;
    for (record, pin) in records.iter().zip(pins::INPUTS) {
        if !ids.insert(record.id) || record.id != pin.id || record.offset != end || record.length != pin.length || record.sha != pin.sha {
            return Err("asset record identity/range mismatch".into());
        }
        let cap = if record.id.starts_with("font/") { 512 << 10 } else { 256 << 10 };
        if record.length == 0 || record.length > cap {
            return Err("asset record length cap".into());
        }
        end = end.checked_add(record.length).filter(|n| *n <= RAW_CAP).ok_or("asset range overflow")?;
    }
    if end != PACK_BYTES {
        return Err("asset total length mismatch".into());
    }
    Ok(())
}
fn descriptors(records: &[Record]) -> Result<String, String> {
    validate_records(records)?;
    let mut s = String::from(
        "// Generated trusted payload descriptors; audit JSON has no runtime authority.\n#[rustfmt::skip]\n#[derive(Debug,Clone,Copy,PartialEq,Eq)] pub enum AssetKind { Font, Catalog }\n#[rustfmt::skip]\n#[derive(Debug,Clone,Copy,PartialEq,Eq,PartialOrd,Ord)] pub enum FontRole { InterMedium, InterSemiBold, JetBrainsMono, Hack, UbuntuLight, NotoEmoji, EmojiIcon }\n#[rustfmt::skip]\n#[derive(Debug,Clone,Copy,PartialEq,Eq)] pub struct FontMeta { pub role:FontRole,pub index:u32,pub scale_bits:u32 }\n#[rustfmt::skip]\n#[derive(Debug,Clone,Copy,PartialEq,Eq)] pub struct CatalogMeta { pub code:&'static str,pub max_lines:u32,pub max_line_bytes:u32,pub max_entries:u32 }\n#[rustfmt::skip]\n#[derive(Debug,Clone,Copy,PartialEq,Eq)] pub struct Descriptor { pub id:&'static str,pub kind:AssetKind,pub offset:u64,pub length:u64,pub sha256:[u8;32],pub font:Option<FontMeta>,pub catalog:Option<CatalogMeta> }\n",
    );
    let _ = writeln!(
        s,
        "#[rustfmt::skip]\npub const SCHEMA_VERSION:u32=1;\n#[rustfmt::skip]\npub const PACK_BYTES:u64={PACK_BYTES};\n#[rustfmt::skip]\npub const PACK_SHA256:[u8;32]={};\n#[rustfmt::skip]\npub const PACK_FILE:&str=\"assetpack-v1-{PACK_SHA}.bin\";\n#[rustfmt::skip]\npub const PACK_SRI:&str=\"{PACK_SRI}\";",
        hex_array(PACK_SHA)?
    );
    s.push_str("#[rustfmt::skip]\npub const DESCRIPTORS:[Descriptor;21]=[\n");
    for r in records {
        let (kind, font, catalog) = if let Some(code) = r.id.strip_prefix("catalog/") {
            (
                "Catalog",
                "None".to_owned(),
                format!("Some(CatalogMeta{{code:{code:?},max_lines:{},max_line_bytes:{},max_entries:{}}})", r.lines, r.line_bytes, r.entries),
            )
        } else {
            let (role, scale) = font_role(r.id)?;
            ("Font", format!("Some(FontMeta{{role:FontRole::{role},index:0,scale_bits:{scale}}})"), "None".to_owned())
        };
        let _ = writeln!(
            s,
            "Descriptor{{id:{:?},kind:AssetKind::{kind},offset:{},length:{},sha256:{},font:{font},catalog:{catalog}}},",
            r.id,
            r.offset,
            r.length,
            hex_array(r.sha)?
        );
    }
    s.push_str("];\n");
    Ok(s)
}
fn check_font_package(root: &Path, archive: &Path) -> Result<(), String> {
    let lock_path = root.join("Cargo.lock");
    let lock_bytes = bounded(&lock_path, fs::metadata(&lock_path).map_err(|e| e.to_string())?.len(), 1 << 20)?;
    let lock = std::str::from_utf8(&lock_bytes).map_err(|e| e.to_string())?;
    if lock.len() > 1 << 20 {
        return Err("Cargo.lock input cap".into());
    }
    let candidates: Vec<_> = lock.split("[[package]]").filter(|s| s.lines().any(|l| l.trim() == "name = \"epaint_default_fonts\"")).collect();
    let [entry] = candidates.as_slice() else { return Err("locked default font package identity".into()) };
    for field in ["version = \"0.36.2\"", "source = \"registry+https://github.com/rust-lang/crates.io-index\"", &format!("checksum = \"{FONT_PACKAGE_SHA}\"")] {
        if !entry.lines().any(|l| l.trim() == field) {
            return Err("locked default font package version/source/checksum".into());
        }
    }
    let archive = fs::File::open(archive).map_err(|e| format!("locked font crate archive: {e}"))?;
    if sha256::reader_hex(archive, 8 << 20)? != FONT_PACKAGE_SHA {
        return Err("locked font crate archive SHA256 mismatch".into());
    }
    Ok(())
}
/// Committed, payload-free metadata is the compile authority in a clean checkout.
/// Reject drift before publishing generated assets; never rewrite that authority.
fn check_trusted_descriptors(root: &Path, generated: &[u8]) -> Result<(), String> {
    let path = source_path(root, "crates/ui-egui/src/web_ui_assets/trusted_descriptors.rs")?;
    let expected_len = u64::try_from(generated.len()).map_err(|_| "descriptor length overflow")?;
    let trusted = bounded(&path, expected_len, 64 << 10)?;
    if trusted != generated {
        return Err("web-assets generated descriptors differ from committed trusted metadata".into());
    }
    Ok(())
}
const FONT_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";

fn metadata_font_paths(metadata: &Value) -> Result<(PathBuf, PathBuf), String> {
    let packages = metadata.get("packages").and_then(Value::as_array).ok_or("Cargo metadata lacks packages")?;
    let mut matches = packages.iter().filter(|package| package.get("name").and_then(Value::as_str) == Some("epaint_default_fonts"));
    let package = matches.next().ok_or("locked Cargo metadata lacks epaint_default_fonts 0.36.2; resolve the native workspace defaults")?;
    if matches.next().is_some() {
        return Err("ambiguous epaint_default_fonts packages in locked Cargo metadata; keep the single pinned 0.36.2 input".into());
    }
    if package.get("version").and_then(Value::as_str) != Some("0.36.2") || package.get("source").and_then(Value::as_str) != Some(FONT_SOURCE) {
        return Err("untrusted font package version/source; expected locked crates.io epaint_default_fonts 0.36.2".into());
    }
    let manifest = Path::new(package.get("manifest_path").and_then(Value::as_str).ok_or("font metadata lacks manifest_path")?);
    if !manifest.is_absolute()
        || manifest.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml")
        || manifest.components().any(|component| matches!(component, std::path::Component::ParentDir | std::path::Component::CurDir))
    {
        return Err("font manifest must be an absolute, confined Cargo source path".into());
    }
    let source = manifest.parent().ok_or("font source directory")?;
    let bucket = source.parent().ok_or("font registry source bucket")?;
    let src = bucket.parent().ok_or("font registry src directory")?;
    let registry = src.parent().ok_or("font registry directory")?;
    if source.file_name().and_then(|name| name.to_str()) != Some("epaint_default_fonts-0.36.2")
        || src.file_name().and_then(|name| name.to_str()) != Some("src")
        || registry.file_name().and_then(|name| name.to_str()) != Some("registry")
    {
        return Err(
            "resolved font source is not a Cargo registry/src cache; use explicit --font-crate/--font-archive for authenticated nonstandard sources".into()
        );
    }
    let bucket_name = bucket.file_name().ok_or("font registry bucket name")?;
    let archive = registry.join("cache").join(bucket_name).join("epaint_default_fonts-0.36.2.crate");
    Ok((source.to_path_buf(), archive))
}

fn read_metadata(reader: &mut impl Read) -> Result<Vec<u8>, String> {
    const CAP: usize = 16 << 20;
    let mut bytes = Vec::new();
    let mut block = [0u8; 65536];
    loop {
        let count = reader.read(&mut block).map_err(|error| format!("Cargo metadata output: {error}"))?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().checked_add(count).is_none_or(|end| end > CAP) {
            return Err("Cargo metadata exceeds the 16 MiB input limit".into());
        }
        bytes.try_reserve(count).map_err(|_| "Cargo metadata allocation")?;
        bytes.extend_from_slice(block.get(..count).ok_or("Cargo metadata read span")?);
    }
}

fn prepare(root: &Path, args: &[&str]) -> Result<(), String> {
    let (mut out, mut check) = (root.join("generated/web-ui-assets"), false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--out" => out = PathBuf::from(args.next().ok_or("--prepare --out PATH")?),
            "--check" => check = true,
            _ => return Err(format!("unknown --prepare argument {arg}; use [--out PATH] [--check]")),
        }
    }
    // Cargo supplies the selected registry/cache location, including custom CARGO_HOME.
    // Inherit offline policy; never upgrade the lockfile or guess a newest package.
    let mut child = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("cannot resolve locked font inputs through Cargo metadata: {error}"))?;
    let result = child.stdout.take().ok_or_else(|| "Cargo metadata stdout unavailable".to_owned()).and_then(|mut stdout| read_metadata(&mut stdout));
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|error| format!("Cargo metadata completion: {error}"))?;
    let bytes = result?;
    if !status.success() {
        return Err("locked Cargo metadata failed; fetch the pinned inputs with the same CARGO_HOME or retry online without changing Cargo.lock".into());
    }
    let metadata: Value = serde_json::from_slice(&bytes).map_err(|error| format!("Cargo metadata JSON: {error}"))?;
    let (fonts, archive) = metadata_font_paths(&metadata)?;
    source_path(&fonts, "Cargo.toml")?;
    if !archive.is_file() {
        return Err(format!("pinned font crate cache archive missing at {}; run cargo fetch --locked with the same CARGO_HOME", archive.display()));
    }
    let mut explicit = vec![
        "--out",
        out.to_str().ok_or("asset output path is not UTF8")?,
        "--font-crate",
        fonts.to_str().ok_or("font source path is not UTF8")?,
        "--font-archive",
        archive.to_str().ok_or("font archive path is not UTF8")?,
    ];
    if check {
        explicit.push("--check");
    }
    run(root, &explicit)
}
/// Explicit locked source root is issued by native packaging using Cargo metadata.
/// It is not runtime fetch authority, nor a wildcard/upgrade lookup.
pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    if args.first() == Some(&"--prepare") {
        return prepare(root, args.get(1..).ok_or("prepare argument span")?);
    }
    if let ["--verify-package", path] = args {
        return verify_distribution(Path::new(path));
    }
    let (mut out, mut font_root, mut font_archive, mut check) = (None, None, None, false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--out" => out = Some(PathBuf::from(args.next().ok_or("--out PATH")?)),
            "--font-crate" => font_root = Some(PathBuf::from(args.next().ok_or("--font-crate PATH from locked Cargo metadata")?)),
            "--font-archive" => font_archive = Some(PathBuf::from(args.next().ok_or("--font-archive locked cached .crate PATH")?)),
            "--check" => check = true,
            _ => return Err(format!("unknown web-assets argument {arg}")),
        }
    }
    let out = out.ok_or("web-assets requires --out PATH")?;
    let fonts = font_root.ok_or("web-assets requires locked Cargo metadata --font-crate PATH")?;
    let archive = font_archive.ok_or("web-assets requires --font-archive locked cached .crate PATH")?;
    check_font_package(root, &archive)?;
    let read_only = check || out.exists();
    let stage = if read_only {
        None
    } else {
        let parent = out.parent().filter(|p| !p.as_os_str().is_empty()).ok_or("asset output needs a parent directory")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let leaf = out.file_name().and_then(|s| s.to_str()).filter(|s| !s.is_empty() && s.len() <= 128).ok_or("asset output name")?;
        let path = parent.join(format!(".{leaf}.staging-{}", std::process::id()));
        fs::create_dir(&path).map_err(|e| format!("asset staging must be fresh: {e}"))?;
        let file = fs::OpenOptions::new().write(true).create_new(true).open(path.join(format!("assetpack-v1-{PACK_SHA}.bin"))).map_err(|e| e.to_string())?;
        Some((path, file))
    };
    let mut stage = stage;
    let (mut offset, mut records) = (0u64, Vec::new());
    for input in pins::INPUTS {
        let base = if input.registry { &fonts } else { root };
        let path = source_path(base, input.path)?;
        let cap = if input.id.starts_with("font/") { 512 << 10 } else { 256 << 10 };
        let bytes =
            if input.id.starts_with("catalog/") { pinned_text(&path, input.length, cap, input.sha)? } else { pinned(&path, input.length, cap, input.sha)? };
        let end = offset.checked_add(input.length).filter(|n| *n <= PACK_BYTES).ok_or("pack append range")?;
        let (lines, line_bytes, entries) = if input.id.starts_with("catalog/") { catalog_bounds(&bytes)? } else { (0, 0, 0) };
        records.push(Record { id: input.id, offset, length: input.length, sha: input.sha, lines, line_bytes, entries });
        if let Some((_, file)) = &mut stage {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        offset = end;
    }
    validate_records(&records)?;
    let stage_path = if let Some((path, mut file)) = stage {
        file.flush().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        verify_pack(&path.join(format!("assetpack-v1-{PACK_SHA}.bin")), &records)?;
        Some(path)
    } else {
        verify_pack(&source_path(&out, &format!("assetpack-v1-{PACK_SHA}.bin"))?, &records)?;
        None
    };
    if offset != PACK_BYTES {
        return Err("pack append invariant".into());
    }
    let generated = descriptors(&records)?;
    check_trusted_descriptors(root, generated.as_bytes())?;
    let inter = pinned(&source_path(root, "assets/fonts/Inter-Regular.ttf")?, 411640, 512 << 10, INTER_SHA)?;
    drop(inter);
    let mut objects: Vec<(String, Vec<u8>)> = Vec::new();
    objects.push(("web_asset_descriptors.rs".into(), generated.into_bytes()));
    let mut licenses = Vec::new();
    for l in pins::LICENSES {
        let bytes = pinned_text(&source_path(if l.registry { &fonts } else { root }, l.path)?, l.length, 64 << 10, l.sha)?;
        check_relative(l.output)?;
        objects.push((l.output.into(), bytes));
        licenses.push(json!({"file":l.output,"bytes":l.length,"sha256":l.sha}));
    }
    let rows:Vec<_>=records.iter().map(|r|json!({"id":r.id,"offset":r.offset,"length":r.length,"sha256":r.sha,"catalog_max_lines":r.lines,"catalog_max_line_bytes":r.line_bytes,"catalog_max_entries":r.entries})).collect();
    let manifest = json!({"schema_version":1,"payload_only":true,"pack":{"file":format!("assetpack-v1-{PACK_SHA}.bin"),"bytes":PACK_BYTES,"sha256":PACK_SHA,"sri":PACK_SRI},"assets":rows,"licenses":licenses,"retained_static":{"id":"font/Inter-Regular","bytes":411640,"sha256":INTER_SHA},"font_source":{"crate":"epaint_default_fonts","version":"0.36.2","package_checksum":FONT_PACKAGE_SHA},"runtime_authority":"compiled descriptors only; this JSON is audit metadata","text_policy":"CRLF pairs to LF only; Unicode and standalone CR preserved; source files unchanged"});
    objects.push(("asset-manifest.json".into(), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?));
    if read_only {
        for (name, expected) in objects {
            let actual = bounded(&source_path(&out, &name)?, expected.len() as u64, RAW_CAP)?;
            if actual != expected {
                return Err(format!("web-assets stale/corrupt {name}"));
            }
        }
        println!("web-assets verified21ranges/9licenses/{PACK_BYTES}B/{PACK_SHA}");
        return Ok(());
    }
    // Complete staging before publication. Existing destinations are never removed.
    let stage = stage_path.ok_or("missing asset staging owner")?;
    for (name, bytes) in objects {
        check_relative(&name)?;
        let path = stage.join(&name);
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?
        }
        fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        let reread = bounded(&path, bytes.len() as u64, RAW_CAP)?;
        if reread != bytes {
            return Err("asset staging roundtrip mismatch".into());
        }
    }
    fs::rename(&stage, &out).map_err(|e| format!("complete asset staging publication: {e}"))?;
    println!("web-assets wrote21ranges/9licenses/{PACK_BYTES}B/{PACK_SHA}");
    Ok(())
}
/// Distribution verification uses only compiled identities, never downloaded metadata.
fn verify_distribution(out: &Path) -> Result<(), String> {
    let mut offset = 0u64;
    let mut records = Vec::new();
    for input in pins::INPUTS {
        records.push(Record { id: input.id, offset, length: input.length, sha: input.sha, lines: 0, line_bytes: 0, entries: 0 });
        offset = offset.checked_add(input.length).ok_or("pack range overflow")?;
    }
    verify_pack(&source_path(out, &format!("assetpack-v1-{PACK_SHA}.bin"))?, &records)?;
    for license in pins::LICENSES {
        drop(pinned(&source_path(out, license.output)?, license.length, 64 << 10, license.sha)?);
    }
    for (name, expected) in [
        ("web_asset_descriptors.rs", "c40072986e6fda64e905ac85f4cbd2cec31f9e7b414f3c045682113f4e05fb60"),
        ("asset-manifest.json", "762d17452f9d5bf113533410be85805ebdaa46b36fb4064b31a47c9e125c1c1c"),
    ] {
        if sha256::reader_hex(fs::File::open(source_path(out, name)?).map_err(|e| e.to_string())?, 64 << 10)? != expected {
            return Err(format!("web-assets package stale/corrupt {name}"));
        }
    }
    println!("web-assets package verified 21 ranges and nine licenses: {PACK_SHA}");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn font_metadata() -> Value {
        let manifest = std::env::temp_dir().join("custom-cargo-home/registry/src/custom-index-bucket/epaint_default_fonts-0.36.2/Cargo.toml");
        json!({"packages":[{"name":"epaint_default_fonts","version":"0.36.2","source":FONT_SOURCE,"manifest_path":manifest}]})
    }
    #[test]
    fn prepare_respects_resolved_custom_cargo_home_and_registry_bucket() {
        let metadata = font_metadata();
        let (source, archive) = metadata_font_paths(&metadata).unwrap();
        let home = std::env::temp_dir().join("custom-cargo-home");
        assert_eq!(source, home.join("registry/src/custom-index-bucket/epaint_default_fonts-0.36.2"));
        assert_eq!(archive, home.join("registry/cache/custom-index-bucket/epaint_default_fonts-0.36.2.crate"));
    }
    #[test]
    fn prepare_refuses_absent_ambiguous_untrusted_and_noncache_metadata() {
        assert!(metadata_font_paths(&json!({})).is_err());
        assert!(metadata_font_paths(&json!({"packages":[]})).is_err());
        for case in 0..7 {
            let mut metadata = font_metadata();
            match case {
                0 => {
                    let extra = metadata["packages"][0].clone();
                    metadata["packages"].as_array_mut().unwrap().push(extra);
                }
                1 => metadata["packages"][0]["version"] = json!("0.36.3"),
                2 => metadata["packages"][0]["source"] = json!("git+https://example.invalid/fonts"),
                3 => metadata["packages"][0]["manifest_path"] = json!("relative/Cargo.toml"),
                4 => metadata["packages"][0]["manifest_path"] = json!(std::env::temp_dir().join("vendor/epaint_default_fonts-0.36.2/Cargo.toml")),
                5 => {
                    metadata["packages"][0]["manifest_path"] =
                        json!(std::env::temp_dir().join("custom-cargo-home/registry/src/custom-index-bucket/epaint_default_fonts-0.36.2/../Cargo.toml"))
                }
                _ => metadata["packages"][0]["source"] = Value::Null,
            }
            assert!(metadata_font_paths(&metadata).is_err(), "case{case}");
        }
        assert!(run(Path::new("unused"), &["--prepare", "--font-crate", "untrusted"]).is_err());
    }
    #[test]
    fn cargo_metadata_reads_are_bounded_and_reader_errors_propagate() {
        assert_eq!(read_metadata(&mut b"{}".as_slice()).unwrap(), b"{}");
        assert!(read_metadata(&mut std::io::repeat(0).take((16 << 20) + 1)).unwrap_err().contains("16 MiB"));
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("metadata read failure"))
            }
        }
        assert!(read_metadata(&mut Broken).unwrap_err().contains("metadata read failure"));
    }
    #[test]
    fn missing_generator_arguments_fail_before_any_source_or_output_io() {
        assert!(run(Path::new("not-a-workspace"), &[]).is_err());
    }
    fn records() -> Vec<Record> {
        let mut offset = 0;
        let mut records = Vec::new();
        for p in pins::INPUTS {
            records.push(Record { id: p.id, offset, length: p.length, sha: p.sha, lines: 0, line_bytes: 0, entries: 0 });
            offset += p.length
        }
        records
    }
    #[test]
    fn ranges_fail_closed_on_missing_duplicate_wrong_hash_overlap_and_overflow() {
        let good = records();
        assert!(validate_records(&good).is_ok());
        for i in 0..5 {
            let mut bad = good.clone();
            match i {
                0 => {
                    bad.pop();
                }
                1 => bad[1].id = bad[0].id,
                2 => bad[0].sha = "0",
                3 => bad[1].offset -= 1,
                _ => bad[0].length = u64::MAX,
            };
            assert!(validate_records(&bad).is_err())
        }
    }
    #[test]
    fn malformed_catalog_utf8_columns_and_bounds_are_errors() {
        assert_eq!(catalog_bounds(b"#header\n\tFile\tFichier\n").unwrap(), (2, 13, 1));
        assert!(catalog_bounds(&[255]).is_err());
        assert!(catalog_bounds(b"a\tb\tc\td").is_err());
        assert!(catalog_bounds("x".repeat(1025).as_bytes()).is_err());
    }
    #[test]
    fn paths_and_digest_do_not_create_authority() {
        for p in ["../x", "/x", "x/../y", ""] {
            assert!(check_relative(p).is_err())
        }
        assert!(hex_array(&"0".repeat(64)).is_ok());
        assert!(hex_array(&"g".repeat(64)).is_err());
    }

    #[test]
    fn committed_metadata_is_required_and_one_byte_drift_is_rejected() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("photocraft-web-descriptor-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let target = root.join("crates/ui-egui/src/web_ui_assets/trusted_descriptors.rs");
        let generated = descriptors(&records()).unwrap();
        assert!(check_trusted_descriptors(&root, generated.as_bytes()).is_err());
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, &generated).unwrap();
        assert!(check_trusted_descriptors(&root, generated.as_bytes()).is_ok());
        let mut drifted = generated.as_bytes().to_vec();
        drifted[0] ^= 1;
        fs::write(&target, &drifted).unwrap();
        assert!(check_trusted_descriptors(&root, generated.as_bytes()).unwrap_err().contains("differ from committed"));
        fs::write(&target, &generated).unwrap();
        assert!(check_trusted_descriptors(&root, &drifted).is_err());
        fs::write(&target, generated.as_bytes().get(..generated.len() - 1).unwrap()).unwrap();
        assert!(check_trusted_descriptors(&root, generated.as_bytes()).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
