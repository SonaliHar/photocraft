//! Release-pinned XCF evidence gates. Future story results stay pending at early gates.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

const STAGES: [&str; 5] = ["inventory", "us1", "us2", "us4", "us5"];
const JSON_LIMIT: u64 = 16 * 1024 * 1024;
const EVIDENCE_LIMIT: u64 = 16 * 1024 * 1024 * 1024;

fn object<'a>(v: &'a Value, key: &str) -> Result<&'a Value, String> {
    v.get(key).filter(|v| v.is_object()).ok_or_else(|| format!("missing object `{key}`"))
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing string `{key}`"))
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], String> {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).ok_or_else(|| format!("missing array `{key}`"))
}
fn strings<'a>(v: &'a Value, key: &str) -> Result<Vec<&'a str>, String> {
    array(v, key)?.iter().map(|v| v.as_str().filter(|s| !s.is_empty()).ok_or_else(|| format!("invalid string in `{key}`"))).collect()
}
fn hash(v: &Value, key: &str) -> Result<(), String> {
    let s = text(v, key)?;
    if s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { Ok(()) } else { Err(format!("invalid SHA256 `{key}`")) }
}
fn index<'a>(values: &'a [Value], key: &str) -> Result<BTreeMap<&'a str, &'a Value>, String> {
    let mut result = BTreeMap::new();
    for v in values {
        let id = text(v, key)?;
        if result.insert(id, v).is_some() {
            return Err(format!("duplicate `{key}`: {id}"));
        }
    }
    Ok(result)
}
fn references(v: &Value, key: &str, values: &BTreeMap<&str, &Value>, required: bool) -> Result<(), String> {
    let ids = strings(v, key)?;
    if required && ids.is_empty() {
        return Err(format!("empty required `{key}`"));
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(format!("duplicate reference `{id}` in `{key}`"));
        }
        if !values.contains_key(id) {
            return Err(format!("unknown `{key}` reference `{id}`"));
        }
    }
    Ok(())
}
fn artifacts(record: &Value) -> Result<BTreeSet<&str>, String> {
    let mut roles = BTreeSet::new();
    let items = array(record, "artifacts")?;
    if items.is_empty() {
        return Err("empty evidence artifact list".into());
    }
    for a in items {
        roles.insert(text(a, "role")?);
        safe_relative(text(a, "path")?)?;
        hash(a, "sha256")?;
        if !matches!(text(a, "license")?, "MIT" | "Apache-2.0" | "MIT OR Apache-2.0" | "CC0-1.0") {
            return Err("evidence has an unapproved/missing license".into());
        }
    }
    Ok(roles)
}
fn safe_relative(value: &str) -> Result<(), String> {
    // Check portable spelling as well as the current OS's path interpretation.
    if value.contains(['\\', ':']) || Path::new(value).components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("evidence path must be a portable relative path: {value}"));
    }
    if value.is_empty() {
        return Err("empty evidence path".into());
    }
    Ok(())
}

fn bind_availability(release: &Value, inventory: &Value, proofs: &BTreeMap<&str, &Value>) -> Result<(), String> {
    let release_id = text(release, "id")?;
    let captured = object(release, "availability")?;
    if text(captured, "status")? != "qualified" {
        return Err(format!("complete availability capture pending for `{release_id}`"));
    }
    references(captured, "proof_ids", proofs, true)?;
    for (surface, key) in [("filter", "available_entries"), ("graph", "graph_available_entries")] {
        let expected = object(captured, surface)?;
        hash(expected, "schema_digest")?;
        let count = expected.get("count").and_then(Value::as_u64).ok_or("availability count must be an integer")?;
        let mut names = BTreeSet::new();
        let mut lines = Vec::new();
        for entry in array(inventory, key)? {
            if text(entry, "release_id")? != release_id {
                continue;
            }
            let name = text(entry, "name")?;
            if name.contains(['\t', '\n', '\r']) || !names.insert(name) {
                return Err(format!("ambiguous/duplicate `{surface}` operation name in `{release_id}`"));
            }
            let schema = object(entry, "public_schema")?;
            hash(schema, "raw_schema_hash")?;
            lines.push(format!("{name}\t{}\n", text(schema, "raw_schema_hash")?));
        }
        lines.sort();
        let total = lines.iter().try_fold(0usize, |sum, line| sum.checked_add(line.len()).ok_or("availability digest size overflow"))?;
        if total as u64 > JSON_LIMIT {
            return Err("availability digest input exceeds evidence limit".into());
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(total).map_err(|e| format!("availability digest allocation: {e}"))?;
        for line in &lines {
            bytes.extend_from_slice(line.as_bytes());
        }
        if count != lines.len() as u64 || text(expected, "schema_digest")? != crate::sha256::hex(&bytes) {
            return Err(format!("omitted/extra/changed `{surface}` availability for `{release_id}`"));
        }
    }
    Ok(())
}

/// Stable measured domain, deliberately excluding implementation work and story results.
fn epoch_domain(release: &Value, inventory: &Value) -> Result<BTreeMap<String, Value>, String> {
    let id = text(release, "id")?;
    let mut domain = BTreeMap::new();
    for (member_id, member) in index(array(inventory, "members")?, "coverage_id")? {
        let mut measured = serde_json::Map::new();
        for key in ["coverage_id", "kind", "operation", "operation_version", "schema_epoch", "release_ids", "typed_schema", "dependency_coverage_ids", "fixture_ids"] {
            measured.insert(key.into(), member.get(key).cloned().ok_or_else(|| format!("epoch domain missing `{key}`"))?);
        }
        domain.insert(format!("member:{member_id}"), Value::Object(measured));
    }
    let mut availability = BTreeSet::new();
    for key in ["available_entries", "graph_available_entries"] {
        for entry in array(inventory, key)? {
            if text(entry, "release_id")? == id {
                let entry_id = text(entry, "availability_id")?;
                if !availability.insert(entry_id) { return Err(format!("duplicate epoch availability `{entry_id}`")); }
                // Public schema and observed serialization/nested classification are measured facts.
                domain.insert(format!("availability:{entry_id}"), entry.clone());
            }
        }
    }
    if domain.len() > 65536 { return Err("epoch subject count exceeds evidence limit".into()); }
    Ok(domain)
}
fn epoch_binding(release: &Value, inventory: &Value) -> Result<Value, String> {
    let id = text(release, "id")?;
    let domain = epoch_domain(release, inventory)?;
    let encoded = serde_json::to_vec(&domain).map_err(|e| format!("epoch domain encoding: {e}"))?;
    if encoded.len() as u64 > JSON_LIMIT { return Err("epoch domain exceeds JSON evidence limit".into()); }
    let runtime = object(release, "runtime")?;
    let mut runtime_binding = serde_json::Map::new();
    for key in ["gimp", "gegl", "babl", "platform", "executable_sha256", "stock_modules_digest"] {
        runtime_binding.insert(key.into(), Value::String(text(runtime, key)?.into()));
    }
    Ok(serde_json::json!({"release_id":id,"epoch_ids":strings(release,"epoch_ids")?,
        "package_sha256":text(object(release,"package")?,"sha256")?,"runtime":runtime_binding,
        "coverage_ids":domain.keys().filter_map(|id|id.strip_prefix("member:")).collect::<Vec<_>>(),
        "availability_ids":domain.keys().filter_map(|id|id.strip_prefix("availability:")).collect::<Vec<_>>(),"inventory_domain_sha256":crate::sha256::hex(&encoded)}))
}
fn epoch_proof<'a>(release: &Value, inventory: &Value, proofs: &'a BTreeMap<&str, &'a Value>) -> Result<&'a Value, String> {
    let id = text(release, "id")?;
    let reference = object(release, "epoch_equivalence_evidence").map_err(|_| format!("release `{id}` lacks distinct epoch evidence"))?;
    let proof_id = text(reference, "proof_id")?;
    if strings(release, "inventory_proof_ids")?.contains(&proof_id) || strings(object(release, "availability")?, "proof_ids")?.contains(&proof_id) {
        return Err(format!("release `{id}` substitutes metadata for epoch evidence"));
    }
    let proof = proofs.get(proof_id).copied().ok_or_else(|| format!("unknown epoch proof `{proof_id}`"))?;
    if text(proof, "kind")? != "full_inventory_epoch" || object(proof, "epoch_binding")? != &epoch_binding(release, inventory)? {
        return Err(format!("epoch proof `{proof_id}` release/runtime/full-domain binding mismatch"));
    }
    artifacts(proof)?;
    if array(proof, "artifacts")?.iter().filter(|a| a.get("role").and_then(Value::as_str) == Some("epoch_report")).count() != 1 {
        return Err(format!("epoch proof `{proof_id}` requires exactly one independent report"));
    }
    Ok(proof)
}

fn validate(releases: &Value, inventory: &Value, fixtures: &Value, stage: &str) -> Result<Vec<String>, String> {
    let upto = if stage == "final" { 4 } else { STAGES.iter().position(|s| *s == stage).ok_or_else(|| format!("unknown XCF gate `{stage}`"))? };
    for doc in [releases, inventory, fixtures] {
        if doc.get("schema_version").and_then(Value::as_u64) != Some(1) {
            return Err("unsupported/missing evidence schema_version".into());
        }
    }
    let releases = index(array(releases, "releases")?, "id")?;
    if releases.is_empty() {
        return Err("empty release inventory".into());
    }
    let availability_values: Vec<Value> =
        array(inventory, "available_entries")?.iter().chain(array(inventory, "graph_available_entries")?.iter()).cloned().collect();
    let available = index(&availability_values, "availability_id")?;
    if available.is_empty() {
        return Err("empty public availability inventory".into());
    }
    let mut epochs = BTreeSet::new();
    let mut latest = false;
    let proofs = index(array(fixtures, "proofs")?, "id")?;
    for release in releases.values() {
        let package = object(release, "package")?;
        if package.get("verified").and_then(Value::as_bool) != Some(true) {
            return Err("unverified official package".into());
        }
        hash(package, "sha256")?;
        for key in ["url", "official_checksum_url"] {
            if !text(package, key)?.starts_with("https://") {
                return Err(format!("release `{key}` must use HTTPS"));
            }
        }
        if package.get("bytes").and_then(Value::as_u64).filter(|n| *n > 0).is_none() {
            return Err("missing package length".into());
        }
        let runtime = object(release, "runtime")?;
        latest |= text(runtime, "gimp")? == "3.2.6";
        for key in ["gegl", "babl"] {
            text(runtime, key)?;
        }
        hash(runtime, "executable_sha256")?;
        hash(runtime, "stock_modules_digest")?;
        if text(runtime, "stock_tree_qualification")? != "passed" || text(release, "epoch_equivalence_status")? != "passed" {
            return Err("release stock provenance or epoch qualification remains pending".into());
        }
        references(release, "inventory_proof_ids", &proofs, true)?;
        bind_availability(release, inventory, &proofs)?;
        for epoch in strings(release, "epoch_ids")? {
            if ["2.8", "2.10", "3.0", "3.2"].contains(&epoch) && !text(runtime, "gimp")?.starts_with(&format!("{epoch}.")) {
                return Err(format!("release version does not qualify epoch `{epoch}`"));
            }
            epochs.insert(epoch);
        }
    }
    for epoch in ["2.8", "2.10", "3.0", "3.2"] {
        if !epochs.contains(epoch) {
            return Err(format!("missing target epoch `{epoch}`"));
        }
    }
    if !latest {
        return Err("missing target GIMP 3.2.6 runtime".into());
    }

    for proof in proofs.values() {
        text(proof, "description")?;
        artifacts(proof)?;
    }
    let fixtures = index(array(fixtures, "fixtures")?, "id")?;
    let members = index(array(inventory, "members")?, "coverage_id")?;
    if members.is_empty() {
        return Err("empty serializable stock union".into());
    }

    for fixture in fixtures.values() {
        references(fixture, "coverage_ids", &members, true)?;
        let author = object(fixture, "authoring")?;
        let release_id = text(author, "release_id")?;
        let runtime = object(releases.get(release_id).ok_or_else(|| format!("unknown fixture release `{release_id}`"))?, "runtime")?;
        for (a, r) in [("gimp_version", "gimp"), ("gegl_version", "gegl"), ("babl_version", "babl")] {
            if text(author, a)? != text(runtime, r)? {
                return Err(format!("fixture version `{a}` mismatches its release"));
            }
        }
        let procedure = object(fixture, "procedure")?;
        safe_relative(text(procedure, "generator")?)?;
        text(procedure, "case_id")?;
        text(procedure, "edit_description")?;
        if text(object(fixture, "qualification")?, "independent_authoring")? != "passed" {
            return Err("independent authoring is unqualified".into());
        }
        let roles = artifacts(fixture)?;
        for role in ["source", "render", "reference_edit", "reference_edit_render", "source_state", "reopened_state", "edited_state"] {
            if !roles.contains(role) {
                return Err(format!("positive fixture missing `{role}`"));
            }
        }
        array(fixture, "resources")?;
    }
    for entry in available.values() {
        let release = text(entry, "release_id")?;
        if !releases.contains_key(release) {
            return Err(format!("unknown availability release `{release}`"));
        }
        let name = text(entry, "name")?;
        let schema = object(entry, "public_schema")?;
        array(schema, "parameters")?;
        hash(schema, "raw_schema_hash")?;
        let serialization = object(entry, "serialization")?;
        references(serialization, "proof_ids", &proofs, true)?;
        match text(serialization, "status")? {
            "serialized" => {
                if strings(serialization, "owners")?.is_empty() {
                    return Err(format!("serialized operation `{name}` lacks owner types"));
                }
                let id = text(entry, "availability_id")?;
                let versions = strings(serialization, "operation_versions")?;
                if versions.is_empty() {
                    return Err(format!("serialized operation `{name}` lacks observed versions"));
                }
                let mut qualified_versions = BTreeSet::new();
                let mut found = false;
                for member in members.values() {
                    if strings(member, "availability_ids")?.contains(&id) {
                        if text(member, "operation")? != name || !strings(member, "release_ids")?.contains(&release) {
                            return Err(format!("misassigned availability `{id}`"));
                        }
                        found = true;
                        let version = text(member, "operation_version")?;
                        if !versions.contains(&version) {
                            return Err(format!("misassigned operation version for `{id}`"));
                        }
                        qualified_versions.insert(version);
                    }
                }
                if !found {
                    return Err(format!("omitted serialized stock member `{id}`"));
                }
                if versions.iter().any(|version| !qualified_versions.contains(version)) {
                    return Err(format!("omitted observed operation version for `{id}`"));
                }
            }
            "not_serialized" => {}
            status => return Err(format!("unqualified serialization status `{status}` for `{name}`")),
        }
        let nested = object(entry, "nested")?;
        if text(nested, "status")? != "closed" {
            return Err(format!("nested closure pending for `{name}`"));
        }
        references(nested, "dependency_coverage_ids", &members, false)?;
        references(nested, "proof_ids", &proofs, true)?;
    }
    let mut pending = Vec::new();
    for (id, member) in &members {
        let kind = text(member, "kind")?;
        if !matches!(kind, "operation" | "feature") {
            return Err(format!("unknown member kind `{kind}`"));
        }
        text(member, "operation")?;
        text(member, "operation_version")?;
        text(member, "schema_epoch")?;
        let typed_schema = object(member, "typed_schema")?;
        let serialized_schema = object(typed_schema, "serialization")?;
        if text(serialized_schema, "status")? != "qualified" {
            return Err(format!("serialized typed layout pending for `{id}`"));
        }
        references(serialized_schema, "proof_ids", &proofs, true)?;
        for key in ["stored_arguments", "configurations", "aux_inputs", "resources", "properties"] {
            array(serialized_schema, key)?;
        }
        if kind == "feature" && array(serialized_schema, "properties")?.is_empty() {
            return Err(format!("feature `{id}` has no measured stored fields"));
        }
        references(member, "release_ids", &releases, true)?;
        references(member, "availability_ids", &available, kind == "operation")?;
        for availability_id in strings(member, "availability_ids")? {
            let entry = available.get(availability_id).ok_or("missing availability")?;
            if text(object(entry, "serialization")?, "status")? != "serialized" {
                return Err(format!("member `{id}` assigned to nonserialized availability `{availability_id}`"));
            }
        }
        references(member, "serialization_proof_ids", &proofs, true)?;
        references(member, "dependency_coverage_ids", &members, false)?;
        references(member, "fixture_ids", &fixtures, true)?;
        for fixture_id in strings(member, "fixture_ids")? {
            let fixture = fixtures.get(fixture_id).ok_or("unknown fixture")?;
            if !strings(fixture, "coverage_ids")?.contains(id) {
                return Err(format!("fixture `{fixture_id}` does not cover `{id}`"));
            }
        }
        let work = object(member, "work")?;
        let parent = text(work, "parent_task")?;
        if kind == "operation" && !matches!(parent, "T056" | "T057" | "T058" | "T059" | "T060" | "T061" | "T062" | "T063" | "T064") {
            return Err(format!("operation `{id}` has invalid parent `{parent}`"));
        }
        for key in ["module", "test"] {
            safe_relative(text(work, key)?)?;
        }
        if strings(work, "acceptance_ids")?.is_empty() {
            return Err(format!("no acceptance IDs for `{id}`"));
        }
        let stages = object(member, "stages")?;
        for (n, name) in STAGES.iter().enumerate() {
            let result = object(stages, name)?;
            match text(result, "status")? {
                "passed" => {
                    if n > 0 {
                        references(result, "report_ids", &proofs, true)?;
                        if strings(result, "test_ids")?.is_empty() {
                            return Err(format!("stage `{name}` lacks tests for `{id}`"));
                        }
                        for report in strings(result, "report_ids")? {
                            let roles = artifacts(proofs.get(report).ok_or("missing test report")?)?;
                            if !roles.contains("test_report") {
                                return Err(format!("stage `{name}` uses a non-test report for `{id}`"));
                            }
                        }
                    }
                }
                "pending" | "failed" | "blocked" | "untested" => {
                    if n <= upto {
                        return Err(format!("{id}: required `{name}` stage is not passed"));
                    }
                    pending.push(format!("{id}: {name} {}", text(result, "status")?));
                }
                other => return Err(format!("invalid stage status `{other}`")),
            }
        }
    }
    if stage == "final" {
        let completion = object(inventory, "completion")?;
        let requirements = index(array(completion, "requirements")?, "id")?;
        let required_ids: Vec<String> = (1..=25).map(|n| format!("FR-{n:03}")).chain((1..=9).map(|n| format!("SC-{n:03}"))).collect();
        if requirements.len() != required_ids.len() {
            return Err("final gate requires exactly 25 FR and 9 SC records".into());
        }
        for id in required_ids {
            let result = requirements.get(id.as_str()).ok_or_else(|| format!("missing final requirement `{id}`"))?;
            if text(result, "status")? != "passed" {
                return Err(format!("final requirement `{id}` remains unqualified"));
            }
            references(result, "coverage_ids", &members, true)?;
            references(result, "report_ids", &proofs, true)?;
            if strings(result, "test_ids")?.is_empty() {
                return Err(format!("final requirement `{id}` lacks tests"));
            }
            for report in strings(result, "report_ids")? {
                if !artifacts(proofs.get(report).ok_or("missing final report")?)?.contains("test_report") {
                    return Err(format!("final requirement `{id}` has no executed report"));
                }
            }
        }
    }
    for release in releases.values() { epoch_proof(release, inventory, &proofs)?; }
    Ok(pending)
}

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&read_json_bytes(path)?).map_err(|e| format!("{}: {e}", path.display()))
}
fn read_json_bytes(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let capacity = usize::try_from(JSON_LIMIT + 1).map_err(|_| "JSON limit exceeds address space")?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity).map_err(|e| format!("JSON allocation: {e}"))?;
    file.take(JSON_LIMIT + 1).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > JSON_LIMIT {
        return Err(format!("{}: evidence JSON exceeds {JSON_LIMIT} bytes", path.display()));
    }
    Ok(bytes)
}

/// Keep assignment edits separate from the release/probe author's inventory. Bind every
/// measured identity before inserting work; never overwrite probe results or stage claims.
fn merge_work(inventory: &mut Value, assignments: &Value) -> Result<(), String> {
    if assignments.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err("unsupported/missing work schema_version".into());
    }
    let work = index(array(assignments, "assignments")?, "coverage_id")?;
    let members = index(array(inventory, "members")?, "coverage_id")?;
    for id in work.keys() {
        if !members.contains_key(id) {
            return Err(format!("work assigned to unknown member `{id}`"));
        }
    }
    for (id, member) in &members {
        let assignment = work.get(id).ok_or_else(|| format!("missing work assignment for `{id}`"))?;
        for key in ["operation", "operation_version", "schema_epoch", "typed_schema", "dependency_coverage_ids", "fixture_ids"] {
            let measured = member.get(key).ok_or_else(|| format!("member `{id}` missing `{key}`"))?;
            if assignment.get(key) != Some(measured) {
                return Err(format!("stale/misassigned work `{key}` for `{id}`"));
            }
        }
        let assigned = object(assignment, "work")?;
        if member.get("work").is_some_and(|prior| prior != assigned) {
            return Err(format!("conflicting inline work assignment for `{id}`"));
        }
    }
    let members = inventory.get_mut("members").and_then(Value::as_array_mut).ok_or("missing members array")?;
    for member in members {
        let id = text(member, "coverage_id")?;
        let assignment = work.get(id).ok_or_else(|| format!("missing work assignment for `{id}`"))?;
        let assigned = object(assignment, "work")?.clone();
        member.as_object_mut().ok_or("invalid member object")?.insert("work".into(), assigned);
    }
    Ok(())
}

fn artifact_path(root: &Path, relative: &str) -> Result<std::path::PathBuf, String> {
    safe_relative(relative)?;
    let root = root.canonicalize().map_err(|e| format!("evidence root {}: {e}", root.display()))?;
    let path = root.join(relative).canonicalize().map_err(|e| format!("missing evidence `{relative}`: {e}"))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(format!("evidence escapes root or is not a file: {relative}"));
    }
    Ok(path)
}

fn verify_files(corpus_root: &Path, fixture_doc: &Value) -> Result<(), String> {
    let mut verified = BTreeMap::new();
    for record in array(fixture_doc, "fixtures")?.iter().chain(array(fixture_doc, "proofs")?.iter()) {
        for artifact in array(record, "artifacts")? {
            let relative = text(artifact, "path")?;
            let expected = text(artifact, "sha256")?;
            if let Some(prior) = verified.get(relative) {
                if *prior != expected {
                    return Err(format!("conflicting hashes for `{relative}`"));
                }
                continue;
            }
            let path = artifact_path(corpus_root, relative)?;
            let file = std::fs::File::open(&path).map_err(|e| format!("{relative}: {e}"))?;
            let actual = crate::sha256::reader_hex(file, EVIDENCE_LIMIT)?;
            if actual != expected {
                return Err(format!("forged/stale evidence hash for `{relative}`: expected {expected}, got {actual}"));
            }
            verified.insert(relative, expected);
        }
    }
    Ok(())
}

fn enrolled_artifact<'a>(proofs: &BTreeMap<&str, &'a Value>, proof_id: &str, path: &str) -> Result<&'a Value, String> {
    let proof = proofs.get(proof_id).ok_or_else(|| format!("unknown capture proof `{proof_id}`"))?;
    let mut found = None;
    for artifact in array(proof, "artifacts")? {
        if text(artifact, "path")? == path && found.replace(artifact).is_some() {
            return Err(format!("ambiguous enrolled artifact `{path}`"));
        }
    }
    found.ok_or_else(|| format!("capture path `{path}` is not enrolled by `{proof_id}`"))
}

fn captured_version(capture: &Value, key: &str) -> Result<String, String> {
    if let Some(value) = capture.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()) {
        return Ok(value.to_owned());
    }
    let parts = array(capture, key)?;
    if parts.len() != 3 {
        return Err(format!("invalid captured runtime version `{key}`"));
    }
    let parts: Result<Vec<String>, String> =
        parts.iter().map(|v| v.as_u64().map(|n| n.to_string()).ok_or_else(|| format!("non-integer captured `{key}`"))).collect();
    Ok(parts?.join("."))
}

/// Bind completeness to independently captured bytes rather than a digest that can
/// be recomputed after deleting a row from the final inventory. Canonical captures
/// retain enrolled original observations; reflection is not serialization proof.
fn verify_availability_captures(root: &Path, releases: &Value, inventory: &Value, fixtures: &Value) -> Result<(), String> {
    let proofs = index(array(fixtures, "proofs")?, "id")?;
    for release in array(releases, "releases")? {
        let release_id = text(release, "id")?;
        let availability = object(release, "availability")?;
        let captures = index(array(availability, "captures")?, "surface")?;
        if captures.len() != 2 || !captures.contains_key("filter") || !captures.contains_key("graph") {
            return Err(format!("release `{release_id}` requires complete filter and graph captures"));
        }
        let runtime = object(release, "runtime")?;
        for (surface, spec) in captures {
            let proof_id = text(spec, "proof_id")?;
            if !strings(availability, "proof_ids")?.contains(&proof_id) {
                return Err(format!("capture `{proof_id}` is not bound by release `{release_id}`"));
            }
            let relative = text(spec, "path")?;
            let artifact = enrolled_artifact(&proofs, proof_id, relative)?;
            if !matches!(text(artifact, "role")?, "availability_capture" | "public_api_inventory") {
                return Err(format!("capture `{relative}` has the wrong evidence role"));
            }
            let capture = read_json(&artifact_path(root, relative)?)?;
            if capture.get("schema_version").and_then(Value::as_u64) != Some(1) || text(&capture, "release_id")? != release_id {
                return Err(format!("capture `{relative}` has wrong schema/release identity"));
            }
            for (key, expected) in [
                ("package_sha256", text(object(release, "package")?, "sha256")?),
                ("executable_sha256", text(runtime, "executable_sha256")?),
                ("stock_modules_digest", text(runtime, "stock_modules_digest")?),
            ] {
                hash(&capture, key)?;
                if text(&capture, key)? != expected {
                    return Err(format!("capture `{relative}` mismatches `{key}`"));
                }
            }
            for key in ["gimp", "gegl", "babl"] {
                if captured_version(&capture, key)? != text(runtime, key)? {
                    return Err(format!("capture `{relative}` mismatches `{key}` runtime"));
                }
            }
            let sources = array(&capture, "source_artifacts")?;
            if sources.is_empty() {
                return Err(format!("capture `{relative}` has no enrolled original observations"));
            }
            for source in sources {
                hash(source, "sha256")?;
                let original = enrolled_artifact(&proofs, text(source, "proof_id")?, text(source, "path")?)?;
                if text(source, "path")? == relative || text(source, "sha256")? != text(original, "sha256")? {
                    return Err(format!("capture `{relative}` has circular/stale original observation"));
                }
            }
            let key = if surface == "filter" { "available_entries" } else { "graph_available_entries" };
            let entries: Vec<Value> =
                array(inventory, key)?.iter().filter(|entry| entry.get("release_id").and_then(Value::as_str) == Some(release_id)).cloned().collect();
            let expected = index(&entries, "name")?;
            let format = text(spec, "format")?;
            if format == "legacy_filter_absent" {
                let version = text(runtime, "gimp")?;
                if surface != "filter"
                    || !(version.starts_with("2.8.") || version.starts_with("2.10."))
                    || !expected.is_empty()
                    || text(&capture, "qualification")? != "passed"
                    || text(&capture, "classification")? != "no stored drawable-filter grammar in the stock pre-3.0 writer/reader"
                {
                    return Err(format!("unqualified historical filter grammar capture `{relative}`"));
                }
                references(&capture, "witness_proof_ids", &proofs, true)?;
                continue;
            }
            let metadata_key = match (surface, format) {
                ("filter", "gimp_filter") => "details",
                ("graph", "gegl_graph") => "metadata",
                _ => return Err(format!("incompatible capture format `{format}` for `{surface}`")),
            };
            let observed = index(array(&capture, "operations")?, "name")?;
            if observed.len() != expected.len() || observed.keys().any(|name| !expected.contains_key(name)) {
                return Err(format!("original capture `{relative}` disagrees with complete `{surface}` availability"));
            }
            for (name, entry) in expected {
                let raw = observed.get(name).ok_or("missing captured operation")?;
                let schema = object(entry, "public_schema")?;
                array(raw, "parameters")?;
                if raw.get("parameters") != schema.get("parameters") || raw.get(metadata_key).is_none() || raw.get(metadata_key) != schema.get(metadata_key) {
                    return Err(format!("original capture `{relative}` disagrees with typed `{name}` schema"));
                }
            }
        }
    }
    Ok(())
}

fn epoch_subjects(release: &Value, inventory: &Value) -> Result<BTreeMap<String, &'static str>, String> {
    let id = text(release, "id")?;
    let entries: Vec<&Value> = array(inventory, "available_entries")?.iter().chain(array(inventory, "graph_available_entries")?).filter(|e| e.get("release_id").and_then(Value::as_str) == Some(id)).collect();
    let mut subjects = BTreeMap::new();
    for member in array(inventory, "members")? {
        let classification = if strings(member, "release_ids")?.contains(&id) { "present" }
            else if text(member, "kind")? == "feature" { "historical" }
            else {
                let matches: Vec<_> = entries.iter().filter(|e| e.get("name") == member.get("operation")).collect();
                if matches.is_empty() { "unavailable" }
                else if matches.iter().all(|e| e.get("serialization").and_then(|s| s.get("status")).and_then(Value::as_str) == Some("not_serialized")) { "not_serialized" }
                else { "historical" }
            };
        subjects.insert(format!("member:{}", text(member, "coverage_id")?), classification);
    }
    for entry in entries {
        let classification = match text(object(entry, "serialization")?, "status")? {
            "serialized" => "present", "not_serialized" => "not_serialized",
            _ => return Err("epoch availability serialization classification remains pending".into()),
        };
        subjects.insert(format!("availability:{}", text(entry, "availability_id")?), classification);
    }
    Ok(subjects)
}
/// A passed metadata string cannot replace independently executed, role-connected bytes.
fn verify_one_epoch_report(root: &Path, release: &Value, inventory: &Value, proofs: &BTreeMap<&str, &Value>) -> Result<(), String> {
        let proof = epoch_proof(release, inventory, proofs)?;
        let evidence = index(array(proof, "artifacts")?, "path")?;
        let mut declared_observation_bytes = 0u64;
        for artifact in evidence.values() {
            let role = text(artifact,"role")?;
            if matches!(role,"epoch_report"|"epoch_author_trace"|"epoch_reopen_trace"|"epoch_observation") {
                let bytes = artifact_path(root,text(artifact,"path")?)?.metadata().map_err(|e|format!("epoch JSON metadata: {e}"))?.len();
                if bytes > JSON_LIMIT { return Err("epoch JSON artifact exceeds evidence limit".into()); }
                if role=="epoch_observation" {
                    declared_observation_bytes=declared_observation_bytes.checked_add(bytes).ok_or("epoch observation size overflow")?;
                    if declared_observation_bytes>JSON_LIMIT { return Err("total epoch observations exceed evidence limit".into()); }
                }
            }
        }
        // Hash every connected artifact even when called independently of the general file gate.
        for artifact in evidence.values() {
            let path = artifact_path(root, text(artifact, "path")?)?;
            if path.metadata().map_err(|e| format!("epoch artifact metadata: {e}"))?.len() == 0 { return Err("empty epoch artifact".into()); }
            let file = std::fs::File::open(&path).map_err(|e| format!("epoch artifact: {e}"))?;
            if crate::sha256::reader_hex(file, EVIDENCE_LIMIT)? != text(artifact, "sha256")? { return Err("forged/stale epoch artifact hash".into()); }
        }
        let report_artifact = evidence.values().find(|a| a.get("role").and_then(Value::as_str) == Some("epoch_report")).ok_or("missing epoch report")?;
        let raw = read_json_bytes(&artifact_path(root, text(report_artifact, "path")?)?)?;
        if crate::sha256::hex(&raw) != text(report_artifact, "sha256")? { return Err("changed epoch report bytes".into()); }
        let report: Value = serde_json::from_slice(&raw).map_err(|e| format!("epoch report JSON: {e}"))?;
        if report.get("schema_version").and_then(Value::as_u64) != Some(1) || text(&report, "proof_id")? != text(proof, "id")?
            || object(&report, "epoch_binding")? != object(proof, "epoch_binding")? { return Err("epoch report binding mismatch".into()); }
        let artifact_role = |path: &str, role: &str| -> Result<(), String> {
            let a = evidence.get(path).ok_or_else(|| format!("disconnected epoch artifact `{path}`"))?;
            if text(a, "role")? != role { return Err(format!("wrong epoch artifact role `{path}`")); }
            Ok(())
        };
        let runs = index(array(&report, "runs")?, "id")?;
        if runs.len() != 2 { return Err("epoch report requires independent author and reopen/edit runs".into()); }
        let mut roles = BTreeSet::new();
        let mut processes = BTreeSet::new();
        let mut profiles = BTreeSet::new();
        let mut profile_paths = BTreeSet::new();
        for run in runs.values() {
            let role = text(run, "role")?;
            if !matches!(role, "author" | "reopen_edit") || !roles.insert(role) || run.get("executed").and_then(Value::as_bool) != Some(true)
                || strings(run, "command")?.is_empty() || run.get("process_id").and_then(Value::as_u64).filter(|p| *p > 0).is_none()
                || text(run, "package_sha256")? != text(object(proof, "epoch_binding")?, "package_sha256")?
                || object(run, "runtime")? != object(object(proof, "epoch_binding")?, "runtime")? { return Err("epoch run provenance mismatch/unexecuted".into()); }
            if !processes.insert((text(run, "machine")?, run.get("process_id").and_then(Value::as_u64))) { return Err("epoch author/reopen process is not independent".into()); }
            if !profiles.insert(text(run, "profile_id")?) || !profile_paths.insert(text(run, "profile_artifact")?) { return Err("epoch author/reopen profiles are not isolated".into()); }
            artifact_role(text(run, "profile_artifact")?, "epoch_profile")?;
            artifact_role(text(run, "generator_artifact")?, "epoch_generator")?;
            artifact_role(text(run, "trace_artifact")?, if role == "author" { "epoch_author_trace" } else { "epoch_reopen_trace" })?;
            let trace_artifact = evidence.get(text(run, "trace_artifact")?).ok_or("missing epoch trace")?;
            let trace_bytes = read_json_bytes(&artifact_path(root, text(trace_artifact, "path")?)?)?;
            if crate::sha256::hex(&trace_bytes) != text(trace_artifact, "sha256")? { return Err("changed epoch trace bytes".into()); }
            let trace: Value = serde_json::from_slice(&trace_bytes).map_err(|e| format!("epoch trace JSON: {e}"))?;
            if trace.get("schema_version").and_then(Value::as_u64) != Some(1) || text(&trace, "proof_id")? != text(proof,"id")?
                || object(&trace,"epoch_binding")? != object(proof,"epoch_binding")? || object(&trace,"run")? != *run { return Err("epoch trace/run identity mismatch".into()); }
        }
        let expected = epoch_subjects(release, inventory)?;
        let domain = epoch_domain(release, inventory)?;
        let mut observations = BTreeMap::new();
        let mut observation_bytes = 0u64;
        let mut observation_count = 0usize;
        for artifact in evidence.values().filter(|a| a.get("role").and_then(Value::as_str) == Some("epoch_observation")) {
            let path = text(artifact,"path")?;
            let raw = read_json_bytes(&artifact_path(root,path)?)?;
            observation_bytes = observation_bytes.checked_add(raw.len() as u64).ok_or("epoch observation size overflow")?;
            if observation_bytes > JSON_LIMIT { return Err("total epoch observations exceed evidence limit".into()); }
            if crate::sha256::hex(&raw) != text(artifact,"sha256")? { return Err("changed epoch observation bytes".into()); }
            let observation: Value = serde_json::from_slice(&raw).map_err(|e|format!("epoch observation JSON: {e}"))?;
            if observation.get("schema_version").and_then(Value::as_u64) != Some(1) || text(&observation,"proof_id")? != text(proof,"id")?
                || object(&observation,"epoch_binding")? != object(proof,"epoch_binding")? { return Err("epoch observation release/domain mismatch".into()); }
            let records = index(array(&observation,"records")?,"subject_id")?;
            observation_count = observation_count.checked_add(records.len()).ok_or("epoch observation count overflow")?;
            if observation_count > expected.len().saturating_mul(2) { return Err("epoch observation count exceeds domain bound".into()); }
            let mut owned_records = BTreeMap::new();
            for (id,record) in records {
                if domain.get(id) != record.get("identity") || expected.get(id).copied() != Some(text(record,"classification")?) { return Err("epoch observed subject identity/classification mismatch".into()); }
                owned_records.insert(id.to_owned(),record.clone());
            }
            observations.insert(path,owned_records);
        }
        let checks = index(array(&report, "checks")?, "subject_id")?;
        if expected.len() != checks.len() || expected.keys().any(|id| !checks.contains_key(id.as_str())) { return Err("epoch report omits/extends full inventory domain".into()); }
        for (id, classification) in expected {
            let check = checks.get(id.as_str()).ok_or("missing epoch subject")?;
            if text(check, "classification")? != classification { return Err(format!("epoch subject `{id}` classification mismatch")); }
            for kind in if classification == "present" { &["serialization", "behavior"][..] } else { &["absence"][..] } {
                let result = object(check, kind)?;
                if text(result, "status")? != if *kind == "absence" { "observed_absent" } else { "passed" } { return Err(format!("epoch `{id}` {kind} not independently observed")); }
                let run_ids = strings(result, "run_ids")?;
                let used_runs: BTreeSet<_> = run_ids.iter().copied().collect();
                if run_ids.len() != 2 || used_runs.len() != 2 || used_runs.iter().any(|run| !runs.contains_key(run)) { return Err("disconnected epoch result run".into()); }
                let paths = strings(result, "artifact_paths")?;
                let observation_path = text(result,"observation_artifact")?;
                artifact_role(observation_path,"epoch_observation")?;
                let record = observations.get(observation_path).and_then(|records|records.get(&id)).ok_or("epoch subject absent from independent observation")?;
                let observed = object(object(record,"results")?,kind)?;
                if observed.get("status") != result.get("status") || observed.get("run_ids") != result.get("run_ids")
                    || (*kind == "behavior" && observed.get("meaningful_change") != result.get("meaningful_change")) { return Err("epoch result contradicts observed subject".into()); }
                let observed_artifacts = index(array(observed,"artifacts")?,"path")?;
                if observed_artifacts.len() != paths.len() { return Err("epoch subject artifact count mismatch".into()); }
                for path in &paths {
                    let actual = evidence.get(path).ok_or("disconnected epoch subject artifact")?;
                    let bound = observed_artifacts.get(path).ok_or("borrowed unrelated epoch subject artifact")?;
                    if text(bound,"role")? != text(actual,"role")? || text(bound,"sha256")? != text(actual,"sha256")? { return Err("epoch observed subject artifact role/hash mismatch".into()); }
                }
                let observed_roles: Result<BTreeSet<_>, String> = paths.iter().map(|path| evidence.get(path).ok_or("disconnected epoch result artifact".into()).and_then(|a| text(a, "role"))).collect();
                let observed_roles = observed_roles?;
                let required = match *kind {
                    "serialization" => &["epoch_source", "epoch_source_state", "epoch_reopened_state"][..],
                    "behavior" => &["epoch_reference_edit", "epoch_render", "epoch_reference_edit_render", "epoch_edited_state"][..],
                    _ => &["epoch_author_trace", "epoch_reopen_trace"][..],
                };
                if required.iter().any(|role| !observed_roles.contains(role)) || paths.len() != required.len() { return Err(format!("epoch `{id}` missing/ambiguous {kind} roles")); }
                if *kind == "behavior" && result.get("meaningful_change").and_then(Value::as_bool) != Some(true) { return Err("epoch behavior has no independently observed meaningful edit".into()); }
            }
        }
    Ok(())
}
fn verify_epoch_reports(root: &Path, releases: &Value, inventory: &Value, fixtures: &Value) -> Result<(), String> {
    let proofs = index(array(fixtures, "proofs")?, "id")?;
    for release in array(releases, "releases")? {
        if text(release, "epoch_equivalence_status")? != "passed" { return Err("release epoch qualification remains pending".into()); }
        verify_one_epoch_report(root, release, inventory, &proofs)?;
    }
    Ok(())
}

/// Scoped evidence never qualifies the global inventory or a story stage.
fn verify_scoped_epoch_report(root: &Path, releases: &Value, inventory: &Value, canonical: &Value, fixtures: &Value, release_id: &str) -> Result<Value, String> {
    if release_id.is_empty() { return Err("--epoch-release requires an exact release ID".into()); }
    let release_index = index(array(releases, "releases")?, "id")?;
    let release = release_index.get(release_id).copied().ok_or_else(|| format!("unknown epoch release `{release_id}`"))?;
    let members = index(array(inventory, "members")?, "coverage_id")?;
    let canonical_members = index(array(canonical, "members")?, "coverage_id")?;
    if members.keys().ne(canonical_members.keys()) { return Err("scoped evidence changes canonical global member union".into()); }
    let public_union = |input: &Value| -> Result<BTreeMap<String, Value>, String> {
        let mut result = BTreeMap::new();
        for key in ["available_entries", "graph_available_entries"] {
            for entry in array(input, key)? {
                if text(entry, "release_id")? == release_id {
                    let id = text(entry, "availability_id")?;
                    let identity = serde_json::json!({"surface":key,"name":text(entry,"name")?,"public_schema":object(entry,"public_schema")?});
                    if result.insert(id.to_owned(), identity).is_some() { return Err("duplicate canonical availability ID".into()); }
                }
            }
        }
        Ok(result)
    };
    if public_union(inventory)? != public_union(canonical)? { return Err("scoped evidence changes canonical selected public availability".into()); }
    let proofs = index(array(fixtures, "proofs")?, "id")?;
    bind_availability(release, inventory, &proofs)?;
    verify_one_epoch_report(root, release, inventory, &proofs)?;
    let binding = epoch_binding(release, inventory)?;
    let proof = epoch_proof(release, inventory, &proofs)?;
    Ok(serde_json::json!({"scope":"release_epoch_evidence","release_id":release_id,
        "subject_count":epoch_domain(release,inventory)?.len(),"inventory_domain_sha256":text(&binding,"inventory_domain_sha256")?,
        "proof_id":text(proof,"id")?,"selected_evidence":"validated","global_qualification":"pending"}))
}

/// Independent canonical workspace inputs anchor staged metadata. These are
/// trusted repository inputs, never subject IDs/digests supplied by the report.
fn verify_scoped_metadata(root: &Path, releases: &Value, inventory: &Value, canonical_releases: &Value, canonical_inventory: &Value, fixtures: &Value, release_id: &str) -> Result<(), String> {
    for document in [releases,inventory,canonical_releases,canonical_inventory,fixtures] {
        if document.get("schema_version").and_then(Value::as_u64)!=Some(1) { return Err("unsupported/missing scoped input schema_version".into()); }
    }
    let fixture_index=index(array(fixtures,"fixtures")?,"id")?;
    let proofs=index(array(fixtures,"proofs")?,"id")?;
    let releases_index=index(array(releases,"releases")?,"id")?;
    let canonical_index=index(array(canonical_releases,"releases")?,"id")?;
    if releases_index.keys().ne(canonical_index.keys()) { return Err("changed canonical target release union".into()); }
    for (id,release) in &releases_index {
        let original=canonical_index.get(id).ok_or("missing canonical release")?;
        let mut measured=(*release).clone(); let mut original=(**original).clone();
        if *id==release_id {
            measured.as_object_mut().ok_or("release is not object")?.remove("epoch_equivalence_evidence");
            original.as_object_mut().ok_or("canonical release is not object")?.remove("epoch_equivalence_evidence");
        }
        if measured!=original { return Err(format!("changed immutable canonical release `{id}`")); }
    }
    let selected=releases_index.get(release_id).copied().ok_or("unknown epoch release selector")?;
    let original=canonical_index.get(release_id).copied().ok_or("unknown canonical epoch release")?;
    let selected_binding=epoch_binding(selected,inventory)?;
    let canonical_binding=epoch_binding(original,canonical_inventory)?;
    for key in ["release_id","epoch_ids","package_sha256","runtime"] {
        if selected_binding.get(key)!=canonical_binding.get(key) { return Err(format!("changed canonical selected release `{key}`")); }
    }
    if selected.get("package")!=original.get("package") || selected.get("availability")!=original.get("availability") {
        return Err("changed canonical package or availability capture binding".into());
    }
    let members=index(array(inventory,"members")?,"coverage_id")?;
    let originals=index(array(canonical_inventory,"members")?,"coverage_id")?;
    if members.keys().ne(originals.keys()) { return Err("changed canonical global member union".into()); }
    for (id, member) in members {
        for (record,key) in [(member,"fixture_ids"),(member,"serialization_proof_ids"),(object(object(member,"typed_schema")?,"serialization")?,"proof_ids")] {
            let ids=strings(record,key)?;
            if ids.iter().copied().collect::<BTreeSet<_>>().len()!=ids.len() { return Err(format!("duplicate scoped evidence reference `{key}`")); }
        }
        references(member,"fixture_ids",&fixture_index,false)?;
        references(member,"serialization_proof_ids",&proofs,false)?;
        references(object(object(member,"typed_schema")?,"serialization")?,"proof_ids",&proofs,false)?;
        let original=originals.get(id).ok_or("missing canonical member")?;
        for key in ["kind","operation","operation_version","schema_epoch","release_ids","dependency_coverage_ids"] {
            if member.get(key)!=original.get(key) { return Err(format!("changed canonical member identity `{id}` `{key}`")); }
        }
        let mut measured=member.clone(); let mut original=(**original).clone();
        // Only serialization facts and additive evidence references may be staged.
        // Membership, dependencies, work and all story/status markers stay exact.
        for key in ["fixture_ids","serialization_proof_ids"] {
            let staged=strings(member,key)?; let baseline=strings(&original,key)?;
            if baseline.iter().any(|id|!staged.contains(id)) { return Err(format!("removed canonical member evidence `{key}`")); }
            measured.as_object_mut().ok_or("member is not object")?.remove(key);
            original.as_object_mut().ok_or("canonical member is not object")?.remove(key);
        }
        measured.get_mut("typed_schema").and_then(Value::as_object_mut).ok_or("missing typed schema")?.remove("serialization");
        original.get_mut("typed_schema").and_then(Value::as_object_mut).ok_or("missing canonical typed schema")?.remove("serialization");
        if measured!=original { return Err(format!("changed immutable canonical member `{id}`")); }
    }
    for key in ["available_entries","graph_available_entries"] {
        let measured=index(array(inventory,key)?,"availability_id")?;
        let baseline=index(array(canonical_inventory,key)?,"availability_id")?;
        if measured.keys().ne(baseline.keys()) { return Err("changed canonical global availability union".into()); }
        for (id,entry) in measured {
            let mut staged=entry.clone();let mut original=(*baseline.get(id).ok_or("missing canonical availability")?).clone();
            if text(entry,"release_id")?==release_id {
                for key in ["serialization","nested"] { staged.as_object_mut().ok_or("availability is not object")?.remove(key); original.as_object_mut().ok_or("canonical availability is not object")?.remove(key); }
            }
            if staged!=original { return Err(format!("changed immutable canonical availability `{id}`")); }
        }
    }
    let capture_paths=array(object(selected,"availability")?,"captures")?.iter().map(|capture|text(capture,"path")).collect::<Result<BTreeSet<_>,String>>()?;
    let mut pending=strings(selected,"inventory_proof_ids")?.into_iter().chain(strings(object(selected,"availability")?,"proof_ids")?).map(str::to_owned).collect::<Vec<_>>();
    let mut visited=BTreeSet::new();
    let mut connected=Vec::new();
    while let Some(id)=pending.pop() {
        if !visited.insert(id.clone()) { continue; }
        if visited.len()>proofs.len() { return Err("metadata proof closure exceeds domain".into()); }
        let proof=proofs.get(id.as_str()).copied().ok_or("unknown selected metadata proof")?;
        artifacts(proof)?;
        for artifact in array(proof,"artifacts")? {
            if capture_paths.contains(text(artifact,"path")?) {
                let capture=read_json(&artifact_path(root,text(artifact,"path")?)?)?;
                for source in array(&capture,"source_artifacts")? { pending.push(text(source,"proof_id")?.to_owned()); }
            }
        }
        connected.push(proof.clone());
    }
    verify_files(root,&serde_json::json!({"fixtures":[],"proofs":connected}))?;
    verify_availability_captures(root,&serde_json::json!({"releases":[selected]}),inventory,fixtures)
}

fn verify_run_reports(corpus_root: &Path, inventory: &Value, fixture_doc: &Value, final_gate: bool) -> Result<(), String> {
    let proofs = index(array(fixture_doc, "proofs")?, "id")?;
    let mut reports = BTreeMap::new();
    for (id, proof) in &proofs {
        for artifact in array(proof, "artifacts")? {
            if text(artifact, "role")? == "test_report" {
                let report = read_json(&artifact_path(corpus_root, text(artifact, "path")?)?)?;
                if report.get("schema_version").and_then(Value::as_u64) != Some(1) {
                    return Err(format!("unknown run report schema `{id}`"));
                }
                let build = object(&report, "build")?;
                for key in ["commit", "target", "profile"] {
                    text(build, key)?;
                }
                text(&report, "machine")?;
                if strings(&report, "command")?.is_empty() {
                    return Err(format!("run report `{id}` has no command"));
                }
                if reports.insert(*id, report).is_some() {
                    return Err(format!("ambiguous run report `{id}`"));
                }
            }
        }
    }
    for member in array(inventory, "members")? {
        let id = text(member, "coverage_id")?;
        let stages = object(member, "stages")?;
        for name in STAGES.iter().skip(1) {
            let stage = object(stages, name)?;
            if text(stage, "status")? != "passed" {
                continue;
            }
            let mut checks = BTreeSet::new();
            let mut fixture_checks: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
            for test_id in strings(stage, "test_ids")? {
                let mut exercised = false;
                for report_id in strings(stage, "report_ids")? {
                    let report = reports.get(report_id).ok_or_else(|| format!("missing run report `{report_id}`"))?;
                    for result in array(report, "test_results")? {
                        if text(result, "coverage_id")? == id && text(result, "stage")? == *name && text(result, "test_id")? == test_id {
                            if text(result, "status")? != "passed" {
                                return Err(format!("run report contains failing `{test_id}`"));
                            }
                            if text(result, "fidelity")? != "full" {
                                return Err(format!("required stock positive `{test_id}` is reduced/dependency-limited"));
                            }
                            let exercised_fixtures = strings(result, "fixture_ids")?;
                            if exercised_fixtures.is_empty()
                                || exercised_fixtures.iter().any(|f| !strings(member, "fixture_ids").is_ok_and(|ids| ids.contains(f)))
                            {
                                return Err(format!("run report fixture mismatch for `{id}`"));
                            }
                            let result_checks = strings(result, "checks")?;
                            if result_checks.contains(&"render") {
                                let observed = result
                                    .get("display_max_error_255")
                                    .and_then(Value::as_f64)
                                    .filter(|v| v.is_finite() && *v >= 0.0)
                                    .ok_or("render report lacks finite display error")?;
                                let tolerance = result
                                    .get("display_tolerance_255")
                                    .and_then(Value::as_f64)
                                    .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 2.0)
                                    .ok_or("render report relaxes/misses display tolerance")?;
                                if observed > tolerance {
                                    return Err(format!("display mismatch for `{id}`"));
                                }
                            }
                            if result_checks.contains(&"import_state") && result.get("exact_state_passed").and_then(Value::as_bool) != Some(true) {
                                return Err(format!("exact imported state not proven for `{id}`"));
                            }
                            for fixture in exercised_fixtures {
                                fixture_checks.entry(fixture).or_default().extend(result_checks.iter().map(|s| String::from(*s)));
                            }
                            checks.extend(result_checks.into_iter().map(String::from));
                            exercised = true;
                        }
                    }
                }
                if !exercised {
                    return Err(format!("no executed `{name}` test `{test_id}` for `{id}`"));
                }
            }
            let required: &[&str] = match *name {
                "us1" => &["import_state", "render", "activation"],
                "us2" => &["edit", "undo", "redo"],
                "us4" => &["native_reopen", "continued_edit", "migration"],
                "us5" => &["native", "web", "cli", "control", "mcp", "gui"],
                _ => &[],
            };
            for check in required {
                if !checks.contains(*check) {
                    return Err(format!("{id}: `{name}` missing exercised `{check}` evidence"));
                }
            }
            for fixture in strings(member, "fixture_ids")? {
                let exercised = fixture_checks.get(fixture).ok_or_else(|| format!("{id}: `{name}` fixture `{fixture}` unexercised"))?;
                if required.iter().any(|check| !exercised.contains(*check)) {
                    return Err(format!("{id}: `{name}` fixture `{fixture}` lacks required checks"));
                }
            }
        }
    }
    if final_gate {
        for requirement in array(object(inventory, "completion")?, "requirements")? {
            let id = text(requirement, "id")?;
            for test_id in strings(requirement, "test_ids")? {
                let mut exercised = false;
                for report_id in strings(requirement, "report_ids")? {
                    let report = reports.get(report_id).ok_or("missing final run report")?;
                    for result in array(report, "requirement_results")? {
                        if text(result, "id")? == id && text(result, "test_id")? == test_id {
                            if text(result, "status")? != "passed" {
                                return Err(format!("final run failed `{id}`/`{test_id}`"));
                            }
                            let coverage = strings(result, "coverage_ids")?;
                            let expected = strings(requirement, "coverage_ids")?;
                            if coverage.is_empty() || expected.iter().any(|id| !coverage.contains(id)) {
                                return Err(format!("final run coverage mismatch for `{id}`"));
                            }
                            exercised = true;
                        }
                    }
                }
                if !exercised {
                    return Err(format!("no executed final test `{id}`/`{test_id}`"));
                }
            }
        }
    }
    Ok(())
}

pub fn run(workspace: &Path, args: &[&str]) -> Result<(), String> {
    let mut stage = "inventory";
    let mut feature = workspace.join("specs/002-full-xcf-import");
    let mut corpus =
        std::env::var_os("PHOTOCRAFT_CORPUS_REPO").map(std::path::PathBuf::from).unwrap_or_else(|| workspace.join("../photocraft-corpus")).join("xcf");
    let mut args = args.iter();
    let mut epoch_release = None;
    let mut artifacts_only = false;
    let mut explicit_stage = false;
    while let Some(arg) = args.next() {
        match *arg {
            "--epoch-release" => {
                if epoch_release.is_some() { return Err("duplicate --epoch-release".into()); }
                epoch_release = Some(args.next().copied().filter(|s| !s.is_empty() && !s.starts_with("--")).ok_or("--epoch-release requires an exact release ID")?);
            }
            "--stage" => {
                stage = args.next().copied().ok_or("--stage requires a value")?;
                explicit_stage = true;
            }
            "--feature" => feature = std::path::PathBuf::from(args.next().ok_or("--feature requires a path")?),
            "--corpus" => corpus = std::path::PathBuf::from(args.next().ok_or("--corpus requires the xcf root")?),
            "--artifacts-only" => artifacts_only = true,
            other => return Err(format!("unknown xcf-inventory argument `{other}`")),
        }
    }
    if epoch_release.is_some() && (artifacts_only || explicit_stage) { return Err("--epoch-release is evidence validation and cannot be combined with --stage or --artifacts-only".into()); }
    if artifacts_only && explicit_stage {
        return Err("--artifacts-only cannot qualify --stage; run these checks separately".into());
    }
    if !artifacts_only && stage != "final" && !STAGES.contains(&stage) {
        return Err(format!("unknown XCF gate `{stage}`"));
    }
    let fixtures = read_json(&corpus.join("full/coverage-fixtures.json"))?;
    if artifacts_only {
        for record in array(&fixtures, "fixtures")?.iter().chain(array(&fixtures, "proofs")?.iter()) {
            artifacts(record)?;
        }
        verify_files(&corpus, &fixtures)?;
        println!(
            "Verified artifact hashes/license declarations for {} fixture records. No inventory or implementation stage qualified.",
            array(&fixtures, "fixtures")?.len()
        );
        return Ok(());
    }
    let releases = read_json(&feature.join("target-releases.json"))?;
    let mut inventory = read_json(&feature.join("stock-inventory.json"))?;
    let assignments = read_json(&feature.join("operation-work.json"))?;
    merge_work(&mut inventory, &assignments)?;
    if let Some(release_id) = epoch_release {
        let canonical_inventory_bytes = read_json_bytes(&workspace.join("specs/002-full-xcf-import/stock-inventory.json"))?;
        let canonical_release_bytes = read_json_bytes(&workspace.join("specs/002-full-xcf-import/target-releases.json"))?;
        let canonical_work_bytes = read_json_bytes(&workspace.join("specs/002-full-xcf-import/operation-work.json"))?;
        let mut canonical: Value = serde_json::from_slice(&canonical_inventory_bytes).map_err(|e|format!("canonical inventory JSON: {e}"))?;
        let canonical_releases: Value = serde_json::from_slice(&canonical_release_bytes).map_err(|e|format!("canonical releases JSON: {e}"))?;
        let canonical_work: Value = serde_json::from_slice(&canonical_work_bytes).map_err(|e|format!("canonical work JSON: {e}"))?;
        merge_work(&mut canonical,&canonical_work)?;
        verify_scoped_metadata(&corpus, &releases, &inventory, &canonical_releases, &canonical, &fixtures, release_id)?;
        let mut result = verify_scoped_epoch_report(&corpus, &releases, &inventory, &canonical, &fixtures, release_id)?;
        let result_object = result.as_object_mut().ok_or("invalid scoped result shape")?;
        result_object.insert("canonical_inputs".into(), serde_json::json!({"stock_inventory_sha256":crate::sha256::hex(&canonical_inventory_bytes),"target_releases_sha256":crate::sha256::hex(&canonical_release_bytes),"operation_work_sha256":crate::sha256::hex(&canonical_work_bytes)}));
        println!("{}", serde_json::to_string(&result).map_err(|e|format!("scoped result encoding: {e}"))?);
        return Ok(());
    }
    let pending = validate(&releases, &inventory, &fixtures, stage)?;
    verify_files(&corpus, &fixtures)?;
    verify_availability_captures(&corpus, &releases, &inventory, &fixtures)?;
    verify_epoch_reports(&corpus, &releases, &inventory, &fixtures)?;
    verify_run_reports(&corpus, &inventory, &fixtures, stage == "final")?;
    println!(
        "XCF `{stage}` gate passed: {} members, {} fixtures; {} later results pending.",
        array(&inventory, "members")?.len(),
        array(&fixtures, "fixtures")?.len(),
        pending.len()
    );
    for line in pending {
        println!("pending: {line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn documents() -> (Value, Value, Value) {
        let mut release_rows = Vec::new();
        for (id, version, epoch) in
            [("legacy28", "2.8.22", "2.8"), ("legacy210", "2.10.38", "2.10"), ("modern30", "3.0.8", "3.0"), ("gimp-current", "3.2.6", "3.2")]
        {
            release_rows.push(json!({"id":id,"package":{"verified":true,"sha256":"a".repeat(64),"url":"https://download.gimp.org/package.exe","official_checksum_url":"https://download.gimp.org/SHA256SUMS","bytes":123},"runtime":{"gimp":version,"gegl":"0.4.72","babl":"0.1.128","platform":"test-platform","executable_sha256":"b".repeat(64),"stock_modules_digest":"e".repeat(64),"stock_tree_qualification":"passed"},"epoch_ids":[epoch],"epoch_equivalence_status":"passed","inventory_proof_ids":["serialize"]}));
        }
        let releases = json!({"schema_version":1,"releases":release_rows});
        let mut releases = releases;
        for release in releases["releases"].as_array_mut().unwrap() {
            let current = release["id"] == "gimp-current";
            release["availability"] = json!({"status":"qualified","proof_ids":["serialize"],
                "filter":{"count":usize::from(current),"schema_digest":if current {"f44108f787ab135ae38f8f5c14b7e9be6cdf34374e08a515417c9cea2ea7f976"}else{"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"}},
                "graph":{"count":0,"schema_digest":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"}});
        }
        let inventory = json!({"schema_version":1,"available_entries":[{
            "availability_id":"gimp-current::gegl:test","name":"gegl:test","release_id":"gimp-current",
            "public_schema":{"parameters":[],"raw_schema_hash":"c".repeat(64)},
            "serialization":{"status":"serialized","proof_ids":["serialize"],"owners":["layer"],"operation_versions":["1"]},
            "nested":{"status":"closed","dependency_coverage_ids":[],"proof_ids":["serialize"]}
        }],"graph_available_entries":[],"members":[{
            "coverage_id":"gegl:test@1/epoch1","kind":"operation","operation":"gegl:test","operation_version":"1","schema_epoch":"epoch1",
            "release_ids":["gimp-current"],"availability_ids":["gimp-current::gegl:test"],"typed_schema":{"parameters":[],"serialization":{"status":"qualified","proof_ids":["serialize"],"stored_arguments":[],"configurations":[],"aux_inputs":[],"resources":[],"properties":[]}},
            "serialization_proof_ids":["serialize"],"dependency_coverage_ids":[],"fixture_ids":["positive"],
            "work":{"parent_task":"T056","module":"crates/algo/src/xcf_point.rs","test":"crates/compose/tests/xcf_filters_point.rs","acceptance_ids":["test-positive"]},
            "stages":{"inventory":{"status":"passed"},"us1":{"status":"pending"},"us2":{"status":"pending"},"us4":{"status":"pending"},"us5":{"status":"pending"}}
        }]});
        let artifact = |role: &str| json!({"role":role,"path":format!("full/{role}.bin"),"sha256":"d".repeat(64),"license":"MIT OR Apache-2.0"});
        let mut fixtures = json!({"schema_version":1,"proofs":[{"id":"serialize","description":"Authored public API save/reopen","artifacts":[artifact("proof")]}],"fixtures":[{
            "id":"positive","coverage_ids":["gegl:test@1/epoch1"],"authoring":{"release_id":"gimp-current","gimp_version":"3.2.6","gegl_version":"0.4.72","babl_version":"0.1.128"},
            "procedure":{"generator":"tools/gimp-oracles/generate.py","case_id":"positive","edit_description":"change argument"},
            "resources":[],"artifacts":[artifact("source"),artifact("render"),artifact("reference_edit"),artifact("reference_edit_render"),artifact("source_state"),artifact("reopened_state"),artifact("edited_state")],
            "qualification":{"independent_authoring":"passed","import":"pending","edit":"pending","migration":"pending"}
        }]});
        for release in releases["releases"].as_array_mut().unwrap() {
            let id = format!("epoch-{}", release["id"].as_str().unwrap());
            release["epoch_equivalence_evidence"] = json!({"proof_id":id});
            fixtures["proofs"].as_array_mut().unwrap().push(json!({"id":id,"kind":"full_inventory_epoch","description":"Synthetic complete independent epoch evidence", "epoch_binding":epoch_binding(release,&inventory).unwrap(),"artifacts":[{"role":"epoch_report","path":format!("full/{id}/report.json"),"sha256":"d".repeat(64),"license":"MIT"}]}));
        }
        (releases, inventory, fixtures)
    }

    fn refresh_epoch_bindings(releases: &Value, inventory: &Value, fixtures: &mut Value) {
        for release in releases["releases"].as_array().unwrap() {
            let id = release["epoch_equivalence_evidence"]["proof_id"].as_str().unwrap();
            let proof = fixtures["proofs"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == id).unwrap();
            proof["epoch_binding"] = epoch_binding(release, inventory).unwrap();
        }
    }
    fn write_epoch_reports(root: &Path, releases: &Value, inventory: &Value, fixtures: &mut Value) {
        for release in releases["releases"].as_array().unwrap() {
            let id = release["epoch_equivalence_evidence"]["proof_id"].as_str().unwrap();
            let proof = fixtures["proofs"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == id).unwrap();
            let mut paths = BTreeMap::new();
            let mut artifacts = Vec::new();
            for role in ["epoch_report", "epoch_profile", "epoch_generator", "epoch_author_trace", "epoch_reopen_trace", "epoch_observation",
                "epoch_source", "epoch_source_state", "epoch_reopened_state", "epoch_reference_edit", "epoch_render", "epoch_reference_edit_render", "epoch_edited_state"] {
                let relative = format!("full/{id}/{role}.json");
                let path = root.join(&relative); std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let bytes = format!("Synthetic independent {id} {role}").into_bytes();
                std::fs::write(&path, &bytes).unwrap(); paths.insert(role, relative.clone());
                artifacts.push(json!({"role":role,"path":relative,"sha256":crate::sha256::hex(&bytes),"license":"MIT"}));
            }
            let second_profile = format!("full/{id}/reopen-profile.json");
            let profile_bytes=std::fs::read(root.join(&paths["epoch_profile"])).unwrap(); std::fs::write(root.join(&second_profile),&profile_bytes).unwrap();
            artifacts.push(json!({"role":"epoch_profile","path":second_profile,"sha256":crate::sha256::hex(&profile_bytes),"license":"MIT"}));
            let runs: Vec<_> = ["author", "reopen_edit"].iter().enumerate().map(|(i, role)| json!({"id":role,"role":role,"executed":true,
                "machine":"synthetic-machine","process_id":i+1,"command":["original-public-authoring"],"package_sha256":proof["epoch_binding"]["package_sha256"],
                "runtime":proof["epoch_binding"]["runtime"],"profile_id":format!("isolated-{role}"),"profile_artifact":if i==0 {paths["epoch_profile"].clone()}else{second_profile.clone()},"generator_artifact":paths["epoch_generator"],
                "trace_artifact":paths[if *role=="author" {"epoch_author_trace"}else{"epoch_reopen_trace"}]})).collect();
            for run in &runs {
                let bytes=serde_json::to_vec(&json!({"schema_version":1,"proof_id":id,"epoch_binding":proof["epoch_binding"],"run":run})).unwrap();
                let path=run["trace_artifact"].as_str().unwrap(); std::fs::write(root.join(path),&bytes).unwrap();
                artifacts.iter_mut().find(|a|a["path"]==path).unwrap()["sha256"]=json!(crate::sha256::hex(&bytes));
            }
            let checks: Vec<_> = epoch_subjects(release, inventory).unwrap().into_iter().map(|(subject, classification)| {
                let result = |roles: &[&str], status: &str| json!({"status":status,"run_ids":["author","reopen_edit"],"observation_artifact":paths["epoch_observation"],"artifact_paths":roles.iter().map(|r|paths[*r].clone()).collect::<Vec<_>>()});
                if classification == "present" {
                    let mut behavior = result(&["epoch_reference_edit","epoch_render","epoch_reference_edit_render","epoch_edited_state"], "passed"); behavior["meaningful_change"]=json!(true);
                    json!({"subject_id":subject,"classification":classification,"serialization":result(&["epoch_source","epoch_source_state","epoch_reopened_state"],"passed"),"behavior":behavior})
                } else {json!({"subject_id":subject,"classification":classification,"absence":result(&["epoch_author_trace","epoch_reopen_trace"],"observed_absent")})}
            }).collect();
            let domain=epoch_domain(release,inventory).unwrap();
            let records:Vec<_>=checks.iter().map(|check| {
                let mut results=serde_json::Map::new();
                for kind in ["serialization","behavior","absence"] {
                    if let Some(result)=check.get(kind) {
                        let mut observed=result.clone(); let observed_object=observed.as_object_mut().unwrap();
                        let observed_artifacts:Vec<_>=result["artifact_paths"].as_array().unwrap().iter().map(|path|artifacts.iter().find(|a|a["path"]==*path).unwrap().clone()).collect();
                        observed_object.remove("artifact_paths");observed_object.remove("observation_artifact");observed_object.insert("artifacts".into(),json!(observed_artifacts));results.insert(kind.into(),observed);
                    }
                }
                json!({"subject_id":check["subject_id"],"identity":domain[check["subject_id"].as_str().unwrap()],"classification":check["classification"],"results":results})
            }).collect();
            let observation_bytes=serde_json::to_vec(&json!({"schema_version":1,"proof_id":id,"epoch_binding":proof["epoch_binding"],"records":records})).unwrap();
            std::fs::write(root.join(&paths["epoch_observation"]),&observation_bytes).unwrap();
            artifacts.iter_mut().find(|a|a["role"]=="epoch_observation").unwrap()["sha256"]=json!(crate::sha256::hex(&observation_bytes));
            let report = json!({"schema_version":1,"proof_id":id,"epoch_binding":proof["epoch_binding"],"runs":runs,"checks":checks});
            let bytes = serde_json::to_vec(&report).unwrap(); std::fs::write(root.join(&paths["epoch_report"]), &bytes).unwrap();
            artifacts[0]["sha256"] = json!(crate::sha256::hex(&bytes)); proof["artifacts"] = json!(artifacts);
        }
    }
    fn mutate_epoch_report(root: &Path, fixtures: &mut Value, mutate: impl FnOnce(&mut Value)) {
        let proof = fixtures["proofs"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == "epoch-gimp-current").unwrap();
        let artifact = proof["artifacts"].as_array_mut().unwrap().iter_mut().find(|a| a["role"] == "epoch_report").unwrap();
        let path = root.join(artifact["path"].as_str().unwrap()); let mut report = read_json(&path).unwrap(); mutate(&mut report);
        let bytes = serde_json::to_vec(&report).unwrap(); std::fs::write(path, &bytes).unwrap(); artifact["sha256"] = json!(crate::sha256::hex(&bytes));
    }

    #[test]
    fn passed_epoch_string_without_distinct_bound_proof_is_rejected() {
        let (mut r, i, f) = documents();
        for release in r["releases"].as_array_mut().unwrap() { release.as_object_mut().unwrap().remove("epoch_equivalence_evidence"); }
        let error = validate(&r, &i, &f, "inventory").expect_err("passed string alone must not qualify an epoch");
        assert!(error.contains("epoch"), "{error}");
    }

    #[test]
    fn full_epoch_report_requires_exact_release_runtime_domain_and_distinct_proof() {
        let (r, i, f) = documents();
        for mutation in 0..7 {
            let (mut releases, mut fixtures) = (r.clone(), f.clone());
            let proof = &mut fixtures["proofs"][4];
            match mutation {
                0 => releases["releases"][3]["epoch_equivalence_evidence"]["proof_id"] = json!("unknown"),
                1 => releases["releases"][3]["epoch_equivalence_evidence"]["proof_id"] = json!("serialize"),
                2 => proof["epoch_binding"]["release_id"] = json!("wrong"),
                3 => proof["epoch_binding"]["runtime"]["executable_sha256"] = json!("f".repeat(64)),
                4 => proof["epoch_binding"]["coverage_ids"] = json!([]),
                5 => proof["kind"] = json!("scoped_gaussian"),
                _ => proof["artifacts"][0]["role"] = json!("inventory_capture"),
            }
            assert!(validate(&releases,&i,&fixtures,"inventory").is_err(),"mutation{mutation}");
        }
        let root=Scratch::new(); let mut fixtures=f; write_epoch_reports(&root.0,&r,&i,&mut fixtures);
        validate(&r,&i,&fixtures,"inventory").unwrap(); verify_epoch_reports(&root.0,&r,&i,&fixtures).unwrap();
        let mut disconnected=fixtures.clone(); disconnected["proofs"][4]["artifacts"][0]["sha256"]=json!("0".repeat(64));
        assert!(verify_epoch_reports(&root.0,&r,&i,&disconnected).is_err());
            assert!(verify_scoped_epoch_report(&root.0,&r,&i,&i,&disconnected,"gimp-current").is_err());
    }
    #[test]
    fn epoch_reports_refuse_rehashed_subset_unexecuted_forged_disconnected_and_noop_checks() {
        let (r,i,f)=documents();
        for mutation in 0..10 {
            let root=Scratch::new(); let mut fixtures=f.clone(); write_epoch_reports(&root.0,&r,&i,&mut fixtures);
            mutate_epoch_report(&root.0,&mut fixtures,|report| match mutation {
                0 => {report["checks"].as_array_mut().unwrap().pop();},
                1 => report["runs"][0]["executed"]=json!(false),
                2 => report["runs"][1]["process_id"]=json!(1),
                3 => report["runs"][0]["runtime"]["executable_sha256"]=json!("f".repeat(64)),
                4 => report["checks"][0]["behavior"]["meaningful_change"]=json!(false),
                5 => report["checks"][0]["serialization"]["artifact_paths"][0]=json!("full/disconnected.xcf"),
                6 => report["checks"][0]["serialization"]["run_ids"]=json!(["author","missing"]),
                7 => report["checks"][0]["classification"]=json!("unavailable"),
                8 => report["runs"][1]["profile_artifact"]=report["runs"][0]["profile_artifact"].clone(),
                _ => report["runs"][1]["profile_id"]=report["runs"][0]["profile_id"].clone(),
            });
            assert!(verify_epoch_reports(&root.0,&r,&i,&fixtures).is_err(),"mutation{mutation}");
            assert!(verify_scoped_epoch_report(&root.0,&r,&i,&i,&fixtures,"gimp-current").is_err(),"scoped mutation{mutation}");
        }
    }
    #[test]
    fn epoch_observations_and_traces_reject_borrowed_artifacts_and_changed_subjects() {
        let (r,i,f)=documents();
        for mutation in 0..4 {
            let root=Scratch::new();let mut fixtures=f.clone();write_epoch_reports(&root.0,&r,&i,&mut fixtures);
            if mutation==0 {
                let path="full/unrelated-gaussian-source.bin";let bytes=b"Unrelated own Gaussian control";std::fs::write(root.0.join(path),bytes).unwrap();
                fixtures["proofs"][4]["artifacts"].as_array_mut().unwrap().push(json!({"role":"epoch_source","path":path,"sha256":crate::sha256::hex(bytes),"license":"MIT"}));
                mutate_epoch_report(&root.0,&mut fixtures,|report|report["checks"][0]["serialization"]["artifact_paths"][0]=json!(path));
            } else {
                let role=if mutation==3 {"epoch_author_trace"}else{"epoch_observation"};
                let artifact=fixtures["proofs"][4]["artifacts"].as_array_mut().unwrap().iter_mut().find(|a|a["role"]==role).unwrap();
                let path=root.0.join(artifact["path"].as_str().unwrap());let mut record=read_json(&path).unwrap();
                match mutation {
                    1 => record["records"][0]["identity"]["name"]=json!("gegl:borrowed"),
                    2 => {record["records"].as_array_mut().unwrap().pop();},
                    _ => record["run"]["process_id"]=json!(999),
                }
                let bytes=serde_json::to_vec(&record).unwrap();std::fs::write(path,&bytes).unwrap();artifact["sha256"]=json!(crate::sha256::hex(&bytes));
            }
            assert!(verify_epoch_reports(&root.0,&r,&i,&fixtures).is_err(),"mutation{mutation}");
            assert!(verify_scoped_epoch_report(&root.0,&r,&i,&i,&fixtures,"gimp-current").is_err(),"scoped mutation{mutation}");
        }
    }
    #[test]
    fn epoch_isolation_rejects_shared_profiles_even_with_rebound_traces() {
        let (r,i,f)=documents();
        for field in ["profile_id","profile_artifact"] {
            let root=Scratch::new();let mut fixtures=f.clone();write_epoch_reports(&root.0,&r,&i,&mut fixtures);
            mutate_epoch_report(&root.0,&mut fixtures,|report|report["runs"][1][field]=report["runs"][0][field].clone());
            let proof=&mut fixtures["proofs"][4];
            let report=read_json(&root.0.join(proof["artifacts"][0]["path"].as_str().unwrap())).unwrap();
            let artifact=proof["artifacts"].as_array_mut().unwrap().iter_mut().find(|a|a["role"]=="epoch_reopen_trace").unwrap();
            let path=root.0.join(artifact["path"].as_str().unwrap());let mut trace=read_json(&path).unwrap();trace["run"]=report["runs"][1].clone();
            let bytes=serde_json::to_vec(&trace).unwrap();std::fs::write(path,&bytes).unwrap();artifact["sha256"]=json!(crate::sha256::hex(&bytes));
            assert!(verify_epoch_reports(&root.0,&r,&i,&fixtures).unwrap_err().contains("profiles are not isolated"));
            assert!(verify_scoped_epoch_report(&root.0,&r,&i,&i,&fixtures,"gimp-current").unwrap_err().contains("profiles are not isolated"));
        }
    }
    #[test]
    fn epoch_observation_files_and_aggregate_are_bounded() {
        let (r,i,f)=documents();
        for multiple in [false,true] {
            let root=Scratch::new();let mut fixtures=f.clone();write_epoch_reports(&root.0,&r,&i,&mut fixtures);
            let proof=&mut fixtures["proofs"][4];
            let original=proof["artifacts"].as_array().unwrap().iter().find(|a|a["role"]=="epoch_observation").unwrap().clone();
            let mut observation=read_json(&root.0.join(original["path"].as_str().unwrap())).unwrap();
            observation["padding"]=json!("x".repeat(if multiple {JSON_LIMIT as usize/2+1024}else{JSON_LIMIT as usize}));
            let bytes=serde_json::to_vec(&observation).unwrap();std::fs::write(root.0.join(original["path"].as_str().unwrap()),&bytes).unwrap();
            let artifacts=proof["artifacts"].as_array_mut().unwrap();artifacts.iter_mut().find(|a|a["role"]=="epoch_observation").unwrap()["sha256"]=json!(crate::sha256::hex(&bytes));
            if multiple {
                let mut extra=original;extra["path"]=json!("full/additional-observation.json");extra["sha256"]=json!(crate::sha256::hex(&bytes));
                std::fs::write(root.0.join(extra["path"].as_str().unwrap()),&bytes).unwrap();artifacts.push(extra);
            }
            assert!(verify_epoch_reports(&root.0,&r,&i,&fixtures).unwrap_err().contains("exceed"));
            assert!(verify_scoped_epoch_report(&root.0,&r,&i,&i,&fixtures,"gimp-current").unwrap_err().contains("exceed"));
        }
    }

    #[test]
    fn early_gate_allows_pending_later_stages() {
        let (r, i, f) = documents();
        let pending = validate(&r, &i, &f, "inventory").expect("inventory passes");
        assert_eq!(pending.len(), 4);
        assert!(validate(&r, &i, &f, "us1").is_err());
        assert!(validate(&r, &i, &f, "final").is_err());
    }

    #[test]
    fn release_capture_binding_rejects_omitted_extra_changed_or_unqualified_availability() {
        let (mut releases, mut inventory, mut fixtures) = documents();
        let mut unused = inventory["available_entries"][0].clone();
        unused["availability_id"] = json!("gimp-current::gegl:unused");
        unused["name"] = json!("gegl:unused");
        unused["serialization"]["status"] = json!("not_serialized");
        unused["serialization"]["operation_versions"] = json!([]);
        inventory["available_entries"].as_array_mut().unwrap().push(unused.clone());
        releases["releases"][3]["availability"]["filter"] =
            json!({"count":2,"schema_digest":"765e979e5c569e0d0dfd55246c4175e5e35c42cdcdaf4d2a4ba469c56b4df324"});
        refresh_epoch_bindings(&releases, &inventory, &mut fixtures);
        assert!(validate(&releases, &inventory, &fixtures, "inventory").is_ok());
        let mut omitted = inventory.clone();
        omitted["available_entries"].as_array_mut().unwrap().pop();
        assert!(validate(&releases, &omitted, &fixtures, "inventory").is_err(), "omitted nonserialized operation");
        let mut extra = inventory.clone();
        unused["name"] = json!("gegl:extra");
        unused["availability_id"] = json!("gimp-current::gegl:extra");
        extra["available_entries"].as_array_mut().unwrap().push(unused);
        assert!(validate(&releases, &extra, &fixtures, "inventory").is_err());
        let mut changed = inventory;
        changed["available_entries"][1]["public_schema"]["raw_schema_hash"] = json!("e".repeat(64));
        assert!(validate(&releases, &changed, &fixtures, "inventory").is_err());
        releases["releases"][3]["availability"]["status"] = json!("pending");
        assert!(validate(&releases, &omitted, &fixtures, "inventory").is_err());
    }

    #[test]
    fn public_pspecs_do_not_qualify_pending_serialized_configs_or_feature_fields() {
        let (releases, inventory, mut fixtures) = documents();
        let mut pending = inventory.clone();
        pending["members"][0]["typed_schema"]["serialization"]["status"] = json!("pending");
        assert!(validate(&releases, &pending, &fixtures, "inventory").is_err());
        let mut missing = inventory.clone();
        missing["members"][0]["typed_schema"]["serialization"]["proof_ids"] = json!([]);
        assert!(validate(&releases, &missing, &fixtures, "inventory").is_err());
        missing["members"][0]["typed_schema"]["serialization"]["proof_ids"] = json!(["missing-layout"]);
        assert!(validate(&releases, &missing, &fixtures, "inventory").is_err());
        for key in ["stored_arguments", "configurations", "aux_inputs", "resources", "properties"] {
            let mut missing = inventory.clone();
            missing["members"][0]["typed_schema"]["serialization"].as_object_mut().unwrap().remove(key);
            assert!(validate(&releases, &missing, &fixtures, "inventory").is_err(), "{key}");
        }
        let mut feature = inventory;
        feature["members"][0]["kind"] = json!("feature");
        assert!(validate(&releases, &feature, &fixtures, "inventory").is_err());
        feature["members"][0]["typed_schema"]["serialization"]["properties"] = json!([{"name":"precision","type":"u32"}]);
        refresh_epoch_bindings(&releases, &feature, &mut fixtures);
        assert!(validate(&releases, &feature, &fixtures, "inventory").is_ok());
    }

    fn assignment_document(inventory: &Value) -> Value {
        let member = &inventory["members"][0];
        json!({"schema_version":1,"assignments":[{
            "coverage_id":member["coverage_id"],"operation":member["operation"],
            "operation_version":member["operation_version"],"schema_epoch":member["schema_epoch"],
            "typed_schema":member["typed_schema"],"dependency_coverage_ids":member["dependency_coverage_ids"],
            "fixture_ids":member["fixture_ids"],"work":member["work"]
        }]})
    }

    #[test]
    fn work_overlay_binds_exact_member_schema_dependencies_and_fixtures() {
        let (_, inventory, _) = documents();
        let work = assignment_document(&inventory);
        let mut merged = inventory.clone();
        merged["members"][0].as_object_mut().unwrap().remove("work");
        merge_work(&mut merged, &work).expect("matching work");
        assert_eq!(merged["members"][0]["work"], inventory["members"][0]["work"]);
        for key in ["operation", "operation_version", "schema_epoch", "typed_schema", "dependency_coverage_ids", "fixture_ids"] {
            let mut altered = work.clone();
            altered["assignments"][0][key] = json!("wrong binding");
            assert!(merge_work(&mut inventory.clone(), &altered).is_err(), "{key}");
        }
    }

    #[test]
    fn work_overlay_rejects_missing_unknown_duplicate_and_conflicting_assignments() {
        let (_, inventory, _) = documents();
        let work = assignment_document(&inventory);
        let mut missing = work.clone();
        missing["assignments"] = json!([]);
        assert!(merge_work(&mut inventory.clone(), &missing).is_err());
        let mut unknown = work.clone();
        unknown["assignments"][0]["coverage_id"] = json!("unknown-operation");
        assert!(merge_work(&mut inventory.clone(), &unknown).is_err());
        let mut duplicate = work.clone();
        duplicate["assignments"].as_array_mut().unwrap().push(work["assignments"][0].clone());
        assert!(merge_work(&mut inventory.clone(), &duplicate).is_err());
        let mut conflict = work;
        conflict["assignments"][0]["work"]["parent_task"] = json!("T064");
        assert!(merge_work(&mut inventory.clone(), &conflict).is_err());
    }

    #[test]
    fn rejects_missing_stock_member_and_unclassified_availability() {
        let (r, mut i, f) = documents();
        i["members"] = json!([]);
        assert!(validate(&r, &i, &f, "inventory").is_err());
        let (_, mut i, _) = documents();
        i["available_entries"][0]["serialization"]["status"] = json!("pending");
        assert!(validate(&r, &i, &f, "inventory").is_err());
    }

    #[test]
    fn rejects_misassigned_positive_and_missing_edit_reference() {
        let (r, i, mut f) = documents();
        f["fixtures"][0]["coverage_ids"] = json!(["another-operation"]);
        assert!(validate(&r, &i, &f, "inventory").is_err());
        let (_, _, mut f) = documents();
        f["fixtures"][0]["artifacts"].as_array_mut().unwrap().retain(|a| a["role"] != "reference_edit");
        assert!(validate(&r, &i, &f, "inventory").is_err());
    }

    #[test]
    fn rejects_unknown_nested_dependency_and_premature_pass() {
        let (r, mut i, f) = documents();
        i["members"][0]["dependency_coverage_ids"] = json!(["omitted-op"]);
        assert!(validate(&r, &i, &f, "inventory").is_err());
        let (_, mut i, _) = documents();
        i["members"][0]["stages"]["us1"] = json!({"status":"passed"});
        assert!(validate(&r, &i, &f, "us1").is_err());
    }

    #[test]
    fn rejects_omitted_version_stock_provenance_and_duplicate_identity() {
        let (r, mut i, f) = documents();
        i["available_entries"][0]["serialization"]["operation_versions"] = json!(["1", "2"]);
        assert!(validate(&r, &i, &f, "inventory").unwrap_err().contains("omitted observed"));
        let (mut r, i, f) = documents();
        r["releases"][0]["runtime"]["stock_tree_qualification"] = json!("pending");
        assert!(validate(&r, &i, &f, "inventory").is_err());
        let (r, mut i, f) = documents();
        let duplicate = i["available_entries"][0].clone();
        i["graph_available_entries"] = json!([duplicate]);
        assert!(validate(&r, &i, &f, "inventory").unwrap_err().contains("duplicate"));
    }

    #[test]
    fn invalid_cli_cannot_bypass_final_gate_with_hash_only() {
        let nowhere = Path::new("nonexistent-evidence-root");
        assert!(run(nowhere, &["--stage", "final", "--artifacts-only"]).unwrap_err().contains("cannot qualify"));
        assert!(run(nowhere, &["--stage", "typo"]).unwrap_err().contains("unknown XCF gate"));
        assert!(run(nowhere, &["--unexpected"]).unwrap_err().contains("unknown xcf-inventory argument"));
        assert!(run(nowhere, &["--stage"]).is_err());
    }

    #[test]
    fn final_requires_every_requirement_with_executed_evidence() {
        let (r, mut i, mut f) = documents();
        f["proofs"].as_array_mut().unwrap().push(json!({"id":"run","description":"executed tests","artifacts":[{"role":"test_report","path":"full/run.json","sha256":"e".repeat(64),"license":"MIT"}]}));
        for stage in STAGES.iter().skip(1) {
            i["members"][0]["stages"][*stage] = json!({"status":"passed","test_ids":["positive-test"],"report_ids":["run"]});
        }
        assert!(validate(&r, &i, &f, "final").unwrap_err().contains("completion"));
        let ids = (1..=25).map(|n| format!("FR-{n:03}")).chain((1..=9).map(|n| format!("SC-{n:03}")));
        let requirements: Vec<Value> =
            ids.map(|id| json!({"id":id,"status":"passed","coverage_ids":["gegl:test@1/epoch1"],"test_ids":["positive-test"],"report_ids":["run"]})).collect();
        i["completion"] = json!({"requirements":requirements});
        assert!(validate(&r, &i, &f, "final").unwrap().is_empty());
        i["completion"]["requirements"][0]["status"] = json!("pending");
        assert!(validate(&r, &i, &f, "final").is_err());
        i["completion"]["requirements"].as_array_mut().unwrap().remove(0);
        assert!(validate(&r, &i, &f, "final").is_err());
    }

    #[test]
    fn rejects_unproven_nonserialized_and_missing_release_epoch() {
        let (r, mut i, f) = documents();
        i["available_entries"][0]["serialization"] = json!({"status":"not_serialized","proof_ids":[]});
        assert!(validate(&r, &i, &f, "inventory").is_err());
        let (mut r, i, f) = documents();
        r["releases"][0]["epoch_ids"] = json!([]);
        assert!(validate(&r, &i, &f, "inventory").is_err());
    }

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("photocraft-xcf-ledger-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            assert!(self.0.starts_with(std::env::temp_dir()));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_artifacts(root: &Path, doc: &mut Value) {
        for key in ["fixtures", "proofs"] {
            for record in doc[key].as_array_mut().unwrap() {
                for artifact in record["artifacts"].as_array_mut().unwrap() {
                    let path = root.join(artifact["path"].as_str().unwrap());
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, b"independently authored example").unwrap();
                    artifact["sha256"] = json!(crate::sha256::hex(b"independently authored example"));
                }
            }
        }
    }

    #[test]
    fn actual_bytes_reject_forged_hashes_conflicts_missing_and_escape() {
        let root = Scratch::new();
        let (_, _, mut f) = documents();
        write_artifacts(&root.0, &mut f);
        verify_files(&root.0, &f).unwrap();
        std::fs::write(root.0.join("full/source.bin"), b"modified").unwrap();
        assert!(verify_files(&root.0, &f).unwrap_err().contains("forged/stale"));
        assert!(artifact_path(&root.0, "../elsewhere.bin").is_err());
        assert!(artifact_path(&root.0, "C:/secret.bin").is_err());
        assert!(artifact_path(&root.0, "full/missing.bin").is_err());
        write_artifacts(&root.0, &mut f);
        let duplicate = json!({"role":"duplicate","path":"full/source.bin","sha256":"a".repeat(64),"license":"MIT"});
        f["fixtures"][0]["artifacts"].as_array_mut().unwrap().push(duplicate);
        assert!(verify_files(&root.0, &f).unwrap_err().contains("conflicting"));
    }

    #[test]
    fn original_captures_reject_recomputed_omission_wrong_schema_runtime_and_unbound_paths() {
        let root = Scratch::new();
        let (mut releases, mut inventory, mut fixtures) = documents();
        releases["releases"] = json!([releases["releases"][3].clone()]);
        inventory["available_entries"][0]["public_schema"]["details"] = json!([true, [], []]);
        std::fs::create_dir(root.0.join("full")).unwrap();
        let identity = json!({"schema_version":1,"release_id":"gimp-current","package_sha256":"a".repeat(64),"executable_sha256":"b".repeat(64),"stock_modules_digest":"e".repeat(64),
            "gimp":"3.2.6","gegl":[0,4,72],"babl":[0,1,128],"source_artifacts":[{"proof_id":"serialize","path":"full/proof.bin","sha256":"d".repeat(64)}]});
        let mut source = identity.clone();
        source["operations"] = json!([
            {"name":"gegl:test","parameters":[],"details":[true,[],[]]},
            {"name":"gegl:unused","parameters":[],"details":[true,[],[]]}
        ]);
        let path = root.0.join("full/capture.json");
        std::fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
        fixtures["proofs"].as_array_mut().unwrap().push(json!({"id":"capture","description":"Original complete public API observation",
            "artifacts":[{"role":"public_api_inventory","path":"full/capture.json","sha256":crate::sha256::hex(&std::fs::read(&path).unwrap()),"license":"MIT"}]}));
        releases["releases"][0]["availability"]["proof_ids"] = json!(["capture"]);
        releases["releases"][0]["availability"]["captures"] = json!([
            {"surface":"filter","proof_id":"capture","path":"full/capture.json","format":"gimp_filter"},
            {"surface":"graph","proof_id":"capture","path":"full/capture.json","format":"gegl_graph"}
        ]);
        assert!(
            verify_availability_captures(&root.0, &releases, &inventory, &fixtures).is_err(),
            "omitted row with recomputed target digest must fail against original capture"
        );
        let mut unused = inventory["available_entries"][0].clone();
        unused["name"] = json!("gegl:unused");
        unused["availability_id"] = json!("gimp-current::gegl:unused");
        inventory["available_entries"].as_array_mut().unwrap().push(unused);
        let mut graph = identity;
        graph["operations"] = json!([]);
        let graph_path = root.0.join("full/graph.json");
        std::fs::write(&graph_path, serde_json::to_vec(&graph).unwrap()).unwrap();
        fixtures["proofs"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == "capture").unwrap()["artifacts"].as_array_mut().unwrap().push(
            json!({"role":"public_api_inventory","path":"full/graph.json","sha256":crate::sha256::hex(&std::fs::read(&graph_path).unwrap()),"license":"MIT"}),
        );
        releases["releases"][0]["availability"]["captures"][1]["path"] = json!("full/graph.json");
        verify_availability_captures(&root.0, &releases, &inventory, &fixtures).unwrap();
        let mut changed = inventory.clone();
        changed["available_entries"][0]["public_schema"]["parameters"] = json!([{"name":"invented"}]);
        assert!(verify_availability_captures(&root.0, &releases, &changed, &fixtures).is_err());
        let mut changed = releases.clone();
        changed["releases"][0]["runtime"]["gegl"] = json!("0.4.999");
        assert!(verify_availability_captures(&root.0, &changed, &inventory, &fixtures).is_err());
        let mut changed = releases.clone();
        changed["releases"][0]["availability"]["captures"][0]["path"] = json!("full/unbound.json");
        std::fs::copy(&path, root.0.join("full/unbound.json")).unwrap();
        assert!(verify_availability_captures(&root.0, &changed, &inventory, &fixtures).is_err());
        let mut changed = releases.clone();
        changed["releases"][0]["availability"]["captures"].as_array_mut().unwrap().pop();
        assert!(verify_availability_captures(&root.0, &changed, &inventory, &fixtures).is_err());
    }

    #[test]
    #[ignore = "requires the locally authored full XCF corpus and release captures"]
    fn local_qualified_availability_captures_match_original_bytes() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let corpus =
            std::env::var_os("PHOTOCRAFT_CORPUS_REPO").map(std::path::PathBuf::from).unwrap_or_else(|| workspace.join("../photocraft-corpus")).join("xcf");
        let feature = workspace.join("specs/002-full-xcf-import");
        let mut releases = read_json(&feature.join("target-releases.json")).unwrap();
        let inventory = read_json(&feature.join("stock-inventory.json")).unwrap();
        let fixtures = read_json(&corpus.join("full/coverage-fixtures.json")).unwrap();
        let all_releases = array(&releases, "releases").unwrap();
        let selected: Vec<Value> =
            all_releases.iter().filter(|release| release.pointer("/availability/status").and_then(Value::as_str) == Some("qualified")).cloned().collect();
        assert!(!selected.is_empty(), "no qualified captures to exercise");
        let pending = all_releases.len() - selected.len();
        releases["releases"] = Value::Array(selected);
        let proofs = index(array(&fixtures, "proofs").unwrap(), "id").unwrap();
        for record in array(&fixtures, "fixtures").unwrap().iter().chain(array(&fixtures, "proofs").unwrap().iter()) {
            artifacts(record).unwrap();
        }
        verify_files(&corpus, &fixtures).unwrap();
        for release in array(&releases, "releases").unwrap() {
            bind_availability(release, &inventory, &proofs).unwrap();
        }
        verify_availability_captures(&corpus, &releases, &inventory, &fixtures).unwrap();
        println!(
            "Verified original bytes for {} qualified release availability captures; {pending} releases pending. No inventory or implementation stage qualified.",
            array(&releases, "releases").unwrap().len()
        );
    }

    #[test]
    fn run_reports_require_executed_matching_positive_checks_and_tolerance() {
        let root = Scratch::new();
        let (r, mut i, mut f) = documents();
        write_artifacts(&root.0, &mut f);
        let report_path = root.0.join("full/run.json");
        let mut report = json!({"schema_version":1,"build":{"commit":"test-build","target":"test-target","profile":"test"},"machine":"test-machine","command":["cargo","test"],"test_results":[{
            "coverage_id":"gegl:test@1/epoch1","stage":"us1","test_id":"test-positive","status":"passed","fidelity":"full","fixture_ids":["positive"],"checks":["import_state","render","activation"],"display_max_error_255":1.0,"display_tolerance_255":2.0,"exact_state_passed":true
        }]});
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        f["proofs"].as_array_mut().unwrap().push(json!({"id":"run","description":"actual test run","artifacts":[{"role":"test_report","path":"full/run.json","sha256":crate::sha256::hex(&std::fs::read(&report_path).unwrap()),"license":"MIT"}]}));
        i["members"][0]["stages"]["us1"] = json!({"status":"passed","test_ids":["test-positive"],"report_ids":["run"]});
        validate(&r, &i, &f, "us1").unwrap();
        verify_files(&root.0, &f).unwrap();
        verify_run_reports(&root.0, &i, &f, false).unwrap();
        report["test_results"][0]["coverage_id"] = json!("other-member");
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        assert!(verify_run_reports(&root.0, &i, &f, false).is_err());
        report["test_results"][0]["coverage_id"] = json!("gegl:test@1/epoch1");
        report["test_results"][0]["display_tolerance_255"] = json!(3.0);
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        assert!(verify_run_reports(&root.0, &i, &f, false).is_err());
        report["test_results"][0]["display_tolerance_255"] = json!(2.0);
        report["test_results"][0]["fidelity"] = json!("reduced");
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        assert!(verify_run_reports(&root.0, &i, &f, false).is_err());
    }
    #[test]
    fn scoped_epoch_accepts_complete_selected_evidence_without_promoting_other_pending_releases() {
        let (mut releases, inventory, mut fixtures) = documents();
        let root=Scratch::new(); write_epoch_reports(&root.0,&releases,&inventory,&mut fixtures);
        for release in releases["releases"].as_array_mut().unwrap() { release["epoch_equivalence_status"]=json!("pending_independent_serialization_qualification"); }
        let before=serde_json::to_vec(&releases).unwrap();
        let result=verify_scoped_epoch_report(&root.0,&releases,&inventory,&inventory,&fixtures,"gimp-current").unwrap();
        assert_eq!(result["subject_count"],2); assert_eq!(result["global_qualification"],"pending");
        assert_eq!(result["selected_evidence"],"validated");
        assert_eq!(serde_json::to_vec(&releases).unwrap(),before);
        assert!(verify_epoch_reports(&root.0,&releases,&inventory,&fixtures).is_err());
        assert!(validate(&releases,&inventory,&fixtures,"inventory").is_err());
        for selector in ["", "3.2", "3.2.6", "wrong"] { assert!(verify_scoped_epoch_report(&root.0,&releases,&inventory,&inventory,&fixtures,selector).is_err()); }
    }
    #[test]
    fn scoped_epoch_refuses_rehashed_smaller_canonical_union() {
        let (r, canonical, mut f)=documents(); let mut smaller=canonical.clone();
        smaller["members"]=json!([]); refresh_epoch_bindings(&r,&smaller,&mut f);
        let root=Scratch::new(); write_epoch_reports(&root.0,&r,&smaller,&mut f);
        assert!(verify_scoped_epoch_report(&root.0,&r,&smaller,&canonical,&f,"gimp-current").unwrap_err().contains("canonical global member union"));
        let mut smaller=canonical.clone(); smaller["available_entries"]=json!([]);
        assert!(verify_scoped_epoch_report(&root.0,&r,&smaller,&canonical,&f,"gimp-current").unwrap_err().contains("canonical selected public availability"));
    }
    #[test]
    fn scoped_epoch_reuses_global_hostile_report_rejections() {
        let (r,i,f)=documents();
        for mutation in 0..10 {
            let root=Scratch::new();let mut fixtures=f.clone();write_epoch_reports(&root.0,&r,&i,&mut fixtures);
            mutate_epoch_report(&root.0,&mut fixtures,|report| match mutation {
                0=>{report["checks"].as_array_mut().unwrap().pop();},
                1=>report["runs"][0]["executed"]=json!(false),
                2=>report["runs"][1]["process_id"]=json!(1),
                3=>report["runs"][0]["runtime"]["executable_sha256"]=json!("f".repeat(64)),
                4=>report["checks"][0]["behavior"]["meaningful_change"]=json!(false),
                5=>report["checks"][0]["serialization"]["artifact_paths"][0]=json!("full/disconnected.xcf"),
                6=>report["checks"][0]["serialization"]["run_ids"]=json!(["author","missing"]),
                7=>report["checks"][0]["classification"]=json!("unavailable"),
                8=>report["runs"][1]["profile_artifact"]=report["runs"][0]["profile_artifact"].clone(),
                _=>report["runs"][1]["profile_id"]=report["runs"][0]["profile_id"].clone(),
            });
            assert!(verify_epoch_reports(&root.0,&r,&i,&fixtures).is_err(),"global{mutation}");
            assert!(verify_scoped_epoch_report(&root.0,&r,&i,&i,&fixtures,"gimp-current").is_err(),"scoped{mutation}");
        }
    }

    #[test]
    fn scoped_cli_refuses_conflicts_duplicate_and_missing_selector_before_loading_files() {
        let root=Scratch::new();
        for args in [vec!["--epoch-release"],vec!["--epoch-release",""],vec!["--epoch-release","--stage"],
            vec!["--epoch-release","gimp-current","--epoch-release","gimp-current"],
            vec!["--epoch-release","gimp-current","--stage","inventory"],
            vec!["--epoch-release","gimp-current","--artifacts-only"]] {
            let error=run(&root.0,&args).unwrap_err(); assert!(error.contains("epoch-release"),"{error}");
        }
    }
    #[test]
    fn scoped_metadata_refuses_self_consistent_staged_cross_release_identity() {
        let (r,i,f)=documents();let root=Scratch::new();
        for mutation in 0..5 {
            let mut staged=r.clone();
            match mutation {
                0=>staged["releases"][3]["runtime"]["executable_sha256"]=json!("f".repeat(64)),
                1=>staged["releases"][3]["runtime"]["stock_modules_digest"]=json!("f".repeat(64)),
                2=>staged["releases"][3]["package"]["sha256"]=json!("f".repeat(64)),
                3=>staged["releases"][3]["epoch_ids"]=json!(["2.8"]),
                _=>{staged["releases"].as_array_mut().unwrap().pop();},
            }
            let error=verify_scoped_metadata(&root.0,&staged,&i,&r,&i,&f,"gimp-current").unwrap_err();
            assert!(error.contains("canonical"),"{error}");
        }
    }

    fn scoped_cli_fixture(root:&Path) -> (Value,Value,Value) {
        let (mut r,mut i,mut f)=documents();
        i["available_entries"][0]["public_schema"]["details"]=json!([true,[],[]]);
        refresh_epoch_bindings(&r,&i,&mut f);
        write_artifacts(root,&mut f);
        let source=f["proofs"][0]["artifacts"][0].clone();
        let mut capture_artifacts=Vec::new(); let mut capture_specs=Vec::new();
        for (surface,format,operations) in [("filter","gimp_filter",json!([{"name":"gegl:test","parameters":[],"details":[true,[],[]]}])),("graph","gegl_graph",json!([]))] {
            let path=format!("full/scoped-{surface}.json");
            let bytes=serde_json::to_vec(&json!({"schema_version":1,"release_id":"gimp-current","package_sha256":"a".repeat(64),
                "executable_sha256":"b".repeat(64),"stock_modules_digest":"e".repeat(64),"gimp":"3.2.6","gegl":[0,4,72],"babl":[0,1,128],
                "source_artifacts":[{"proof_id":"serialize","path":source["path"],"sha256":source["sha256"]}],"operations":operations})).unwrap();
            std::fs::write(root.join(&path),&bytes).unwrap();
            capture_artifacts.push(json!({"role":"public_api_inventory","path":path,"sha256":crate::sha256::hex(&bytes),"license":"MIT"}));
            capture_specs.push(json!({"surface":surface,"format":format,"proof_id":"capture","path":path}));
        }
        f["proofs"].as_array_mut().unwrap().push(json!({"id":"capture","description":"Synthetic complete selected public capture","artifacts":capture_artifacts}));
        r["releases"][3]["availability"]["proof_ids"]=json!(["capture"]);
        r["releases"][3]["availability"]["captures"]=json!(capture_specs);
        write_epoch_reports(root,&r,&i,&mut f);
        for release in r["releases"].as_array_mut().unwrap() { release["epoch_equivalence_status"]=json!("pending_independent_serialization_qualification"); }
        (r,i,f)
    }
    #[test]
    fn scoped_cli_metadata_positive_does_not_touch_inputs_or_read_other_epoch_reports() {
        let root=Scratch::new(); let (r,i,f)=scoped_cli_fixture(&root.0);
        let feature=root.0.join("specs/002-full-xcf-import");std::fs::create_dir_all(&feature).unwrap();
        let work=json!({"schema_version":1,"assignments":i["members"].as_array().unwrap().iter().map(|member| {
            let mut assigned=json!({"work":member["work"],"coverage_id":member["coverage_id"]});
            for key in ["operation","operation_version","schema_epoch","typed_schema","dependency_coverage_ids","fixture_ids"] {assigned[key]=member[key].clone();} assigned
        }).collect::<Vec<_>>()});
        // Match the real canonical inventory: work lives in operation-work.json,
        // not inline in the raw stock inventory.
        let mut raw_inventory=i.clone();for member in raw_inventory["members"].as_array_mut().unwrap() {member.as_object_mut().unwrap().remove("work");}
        for (path,value) in [(feature.join("target-releases.json"),&r),(feature.join("stock-inventory.json"),&raw_inventory),(feature.join("operation-work.json"),&work),(root.0.join("full/coverage-fixtures.json"),&f)] {
            std::fs::write(path,serde_json::to_vec(value).unwrap()).unwrap();
        }
        let input_before=[feature.join("target-releases.json"),feature.join("stock-inventory.json"),feature.join("operation-work.json"),root.0.join("full/coverage-fixtures.json")].into_iter().map(|path| {let bytes=std::fs::read(&path).unwrap();(path,bytes)}).collect::<Vec<_>>();
        // Unrelated report bytes are missing: scoped mode must not load them.
        for proof in f["proofs"].as_array().unwrap().iter().filter(|proof|proof["kind"]=="full_inventory_epoch" && proof["id"]!="epoch-gimp-current") {
            let artifact=proof["artifacts"].as_array().unwrap().iter().find(|a|a["role"]=="epoch_report").unwrap();
            std::fs::remove_file(root.0.join(artifact["path"].as_str().unwrap())).unwrap();
        }
        let before=std::fs::read(feature.join("target-releases.json")).unwrap();
        run(&root.0,&["--epoch-release","gimp-current","--corpus",root.0.to_str().unwrap()]).unwrap();
        assert_eq!(std::fs::read(feature.join("target-releases.json")).unwrap(),before);
        assert!(run(&root.0,&["--corpus",root.0.to_str().unwrap()]).is_err());
        for selector in ["3.2.6","missing"] {assert!(run(&root.0,&["--epoch-release",selector,"--corpus",root.0.to_str().unwrap()]).is_err());}
        std::fs::write(root.0.join("full/proof.bin"),b"mutated original metadata").unwrap();
        assert!(run(&root.0,&["--epoch-release","gimp-current","--corpus",root.0.to_str().unwrap()]).is_err());
        for (path,bytes) in input_before {assert_eq!(std::fs::read(path).unwrap(),bytes,"scoped success/failure must not write inputs");}
    }

    #[test]
    fn scoped_canonical_anchor_refuses_rehashed_membership_and_dependency_removal() {
        for field in ["release_ids","dependency_coverage_ids"] {
            let root=Scratch::new();let (r,mut canonical,mut f)=scoped_cli_fixture(&root.0);
            if field=="dependency_coverage_ids" {canonical["members"][0][field]=json!(["gegl:test@1/epoch1"]);}
            let mut staged=canonical.clone();staged["members"][0][field]=json!([]);
            refresh_epoch_bindings(&r,&staged,&mut f);write_epoch_reports(&root.0,&r,&staged,&mut f);
            // This is a completely rehashed, internally connected staged report,
            // not a stale hash/trace failure. Canonical facts must reject it.
            verify_scoped_epoch_report(&root.0,&r,&staged,&staged,&f,"gimp-current").unwrap();
            let error=verify_scoped_metadata(&root.0,&r,&staged,&r,&canonical,&f,"gimp-current").unwrap_err();
            assert!(error.contains(field),"{field}: {error}");
        }
    }
    #[test]
    fn scoped_staging_allowlist_preserves_markers_and_public_schema() {
        let root=Scratch::new();let(r,canonical,mut f)=scoped_cli_fixture(&root.0);
        for mutation in 0..6 {
            let mut staged=canonical.clone();
            match mutation {
                0=>staged["members"][0]["stages"]["us1"]["status"]=json!("passed"),
                1=>staged["members"][0]["typed_schema"]["parameters"]=json!([{"name":"invented"}]),
                2=>staged["members"][0]["fixture_ids"]=json!([]),
                3=>staged["members"][0]["serialization_proof_ids"]=json!([]),
                4=>staged["available_entries"][0]["extra_immutable_metadata"]=json!(true),
                _=>staged["members"][0]["fixture_ids"].as_array_mut().unwrap().push(json!("positive")),
            }
            assert!(verify_scoped_metadata(&root.0,&r,&staged,&r,&canonical,&f,"gimp-current").is_err(),"{mutation}");
        }
        let mut staged=canonical.clone();
        staged["members"][0]["fixture_ids"].as_array_mut().unwrap().push(json!("new-independent-case"));
        staged["members"][0]["serialization_proof_ids"].as_array_mut().unwrap().push(json!("new-independent-proof"));
        staged["members"][0]["typed_schema"]["serialization"]["stored_arguments"]=json!([{"observed_tag":45}]);
        assert!(verify_scoped_metadata(&root.0,&r,&staged,&r,&canonical,&f,"gimp-current").is_err(),"unregistered added evidence must fail");
        f["fixtures"].as_array_mut().unwrap().push(json!({"id":"new-independent-case"}));
        f["proofs"].as_array_mut().unwrap().push(json!({"id":"new-independent-proof"}));
        verify_scoped_metadata(&root.0,&r,&staged,&r,&canonical,&f,"gimp-current").unwrap();
        // This metadata approval alone cannot validate a stale report; the
        // full report must bind the added exact staged fields and artifacts.
        assert!(verify_scoped_epoch_report(&root.0,&r,&staged,&canonical,&f,"gimp-current").is_err());
    }

    #[test]
    #[ignore = "read-only canonical workspace input domain calculation"]
    fn real_windows326_domain_keeps_all_global_members_and_selected_availability() {
        let path=std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_EPOCH_CANONICAL_FEATURE").unwrap());
        let releases=read_json(&path.join("target-releases.json")).unwrap();let inventory=read_json(&path.join("stock-inventory.json")).unwrap();
        let release=array(&releases,"releases").unwrap().iter().find(|r|r["id"]=="gimp-3.2.6-windows-x86_64-setup").unwrap();
        let domain=epoch_domain(release,&inventory).unwrap();let binding=epoch_binding(release,&inventory).unwrap();
        let members=domain.keys().filter(|id|id.starts_with("member:")).count();let availability=domain.len()-members;
        assert_eq!(members,array(&inventory,"members").unwrap().len());assert_eq!(members,709);assert_eq!(availability,440);assert_eq!(domain.len(),1149);
        println!("real_read_only_domain={}",serde_json::to_string(&serde_json::json!({"members":members,"availability":availability,"subjects":domain.len(),"binding":binding})).unwrap());
    }

}
