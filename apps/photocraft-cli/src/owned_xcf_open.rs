//! Explicit CLI policy; all request-shape checks precede source/root I/O.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Policy {
    Xcf,
    Native,
}
pub(super) fn policy(args: &Args, run: bool) -> Result<Option<Policy>, String> {
    let mut xcf = args.flags.iter().filter(|(key, _)| key == "--xcf-profile-policy");
    let mut native = args.flags.iter().filter(|(key, _)| key == "--native-profile-policy");
    let xcf_value = xcf.next();
    let native_value = native.next();
    if xcf_value.is_some() && native_value.is_some() {
        return Err("--xcf-profile-policy and --native-profile-policy are mutually exclusive".into());
    }
    let (policy, value, duplicate, option, kind, extension) = match (xcf_value, native_value) {
        (Some((_, value)), None) => (Policy::Xcf, value, xcf.next().is_some(), "--xcf-profile-policy", "XCF", "xcf"),
        (None, Some((_, value))) => (Policy::Native, value, native.next().is_some(), "--native-profile-policy", "native", "pcraft"),
        (None, None) => return Ok(None),
        _ => return Err("owned profile policies are mutually exclusive".into()),
    };
    if duplicate || value.as_deref() != Some("keepCurrent") {
        return Err(format!("{option} accepts exactly one keepCurrent value"));
    }
    if run && args.get("--new").is_some() {
        return Err(format!("explicit owned {kind} keepCurrent does not support --new; no input was opened"));
    }
    if run {
        if args.flags.iter().filter(|(key, _)| key == "--out").count() > 1 || args.flags.iter().filter(|(key, _)| key == "--format").count() > 1 {
            return Err(format!("explicit owned {kind} keepCurrent accepts at most one --out and --format"));
        }
        if let Some(output) = args.get("--out") {
            if output.is_empty()
                || output.len() > 4096
                || output.contains('\0')
                || !Path::new(output).extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("pcraft"))
            {
                return Err(format!("explicit owned {kind} keepCurrent output requires a bounded .pcraft destination before opening input"));
            }
        }
        if args.get("--format").is_some_and(|value| value.len() > 16 || !value.trim_start_matches('.').eq_ignore_ascii_case("pcraft")) {
            return Err(format!("explicit owned {kind} keepCurrent supports pcraft format only"));
        }
    }
    let [path] = args.positional.as_slice() else { return Err(format!("explicit owned {kind} keepCurrent requires one .{extension} input")) };
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\0')
        || !Path::new(path).extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case(extension))
    {
        return Err(format!("explicit owned {kind} keepCurrent requires a bounded .{extension} input"));
    }
    Ok(Some(policy))
}
pub(super) fn open(
    headless: &mut Headless,
    path: &str,
    policy: Policy,
) -> Result<photocraft_automation::headless::OwnedXcfOpenReply, photocraft_automation::AutomationError> {
    match policy {
        Policy::Xcf => headless.open_owned_xcf_keep_current_bounded(Path::new(path)),
        Policy::Native => headless.open_owned_native_keep_current_bounded(Path::new(path)),
    }
}
#[cfg(test)]
mod native_open_tests;
#[cfg(test)]
mod native_save_tests;
#[cfg(test)]
mod tests;
