//! Explicit CLI policy; all request-shape checks precede source/root I/O.
use super::*;
pub(super) fn policy(args: &Args, run: bool) -> Result<bool, String> {
    let mut values = args.flags.iter().filter(|(key, _)| key == "--xcf-profile-policy");
    let Some((_, value)) = values.next() else { return Ok(false) };
    if values.next().is_some() || value.as_deref() != Some("keepCurrent") {
        return Err("--xcf-profile-policy accepts exactly one keepCurrent value".into());
    }
    if run && (args.get("--new").is_some() || args.get("--out").is_some()) {
        return Err("explicit owned XCF keepCurrent does not support --new or --out; no input was opened".into());
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
mod tests;
