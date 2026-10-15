//! Explicit CLI policy; all request-shape checks precede source/root I/O.
use super::*;
pub(super) fn policy(args: &Args, run: bool) -> Result<bool, String> {
    let mut values = args.flags.iter().filter(|(key, _)| key == "--xcf-profile-policy");
    let Some((_, value)) = values.next() else { return Ok(false) };
    if values.next().is_some() || value.as_deref() != Some("keepCurrent") {
        return Err("--xcf-profile-policy accepts exactly one keepCurrent value".into());
    }
    if run && args.get("--new").is_some() {
        return Err("explicit owned XCF keepCurrent does not support --new; no input was opened".into());
    }
    if run {
        if args.flags.iter().filter(|(key, _)| key == "--out").count() > 1 || args.flags.iter().filter(|(key, _)| key == "--format").count() > 1 {
            return Err("explicit owned XCF keepCurrent accepts at most one --out and --format".into());
        }
        if let Some(output) = args.get("--out") {
            if output.is_empty()
                || output.len() > 4096
                || output.contains('\0')
                || !Path::new(output).extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("pcraft"))
            {
                return Err("explicit owned XCF keepCurrent output requires a bounded .pcraft destination before opening input".into());
            }
        }
        if args.get("--format").is_some_and(|value| value.len() > 16 || !value.trim_start_matches('.').eq_ignore_ascii_case("pcraft")) {
            return Err("explicit owned XCF keepCurrent supports pcraft format only".into());
        }
    }
    let [path] = args.positional.as_slice() else { return Err("explicit owned XCF keepCurrent requires one .xcf input".into()) };
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\0')
        || !Path::new(path).extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("xcf"))
    {
        return Err("explicit owned XCF keepCurrent requires a bounded .xcf input".into());
    }
    Ok(true)
}
pub(super) fn open(headless: &mut Headless, path: &str) -> Result<photocraft_automation::headless::OwnedXcfOpenReply, photocraft_automation::AutomationError> {
    headless.open_owned_xcf_keep_current_bounded(Path::new(path))
}
#[cfg(test)]
mod native_save_tests;
#[cfg(test)]
mod tests;
