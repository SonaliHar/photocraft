//! Read-only original-point sampling and owned coverage for a closed backing domain.
//! This does not activate accounted document selection edits or source rendering.
#[cfg(test)]
#[path = "selection_backed_exact_tests.rs"]
mod exact_tests;
#[cfg(test)]
#[path = "selection_backed_sampling_tests.rs"]
mod tests;
use super::*;
use crate::channel_cmds::ChannelView;
use photocraft_doc::{Layer, LayerContent, SampleEncoding, WorkingProfileInterpretation};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
const WORK_LIMIT: u64 = 8_000_000_000;
// This plan borrows the immutable Transform; it never enters a self-referential
// query owner. The existing actual leases cover its bounded metadata/scratch.
fn exact_plan<'t>(
    transform: &'t photocraft_cms::Transform,
    owner: &Retention,
    extra_held: u64,
    control: &Interrupt<'_>,
    parent: Option<&ReadControl<'_>>,
) -> Result<(photocraft_cms::transform::PreparedRgbLabF32<'t>, photocraft_cms::control::Limits)> {
    let default = photocraft_cms::control::Limits::default();
    let held = owner._transform.bytes().checked_add(extra_held).ok_or_else(|| failure("exact CMS held envelope"))?;
    let resident_bytes = usize::try_from(held).map_err(|_| failure("exact CMS held bytes"))?.min(default.resident_bytes);
    let preparation = photocraft_cms::Transform::RGB_LAB_F32_PREPARATION_WORK;
    owner.work.consume(preparation, control, parent)?;
    let plan = transform
        .prepare_rgb_lab_f32_with_control(photocraft_cms::control::Limits { resident_bytes, work_units: preparation }, &|| control.cancelled())
        .map_err(cms_error)?;
    Ok((plan, photocraft_cms::control::Limits { resident_bytes, ..default }))
}
struct Work {
    used: AtomicU64,
}
impl Work {
    fn consume(&self, n: u64, control: &Interrupt<'_>, parent: Option<&ReadControl<'_>>) -> Result<()> {
        poll(control)?;
        self.used
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |old| old.checked_add(n).filter(|v| *v <= WORK_LIMIT))
            .map_err(|_| failure("backed preview cumulative work limit"))?;
        if let Some(parent) = parent {
            parent.check(n)?;
        }
        poll(control)
    }
}
pub(crate) struct Retention {
    source: DocumentSnapshot,
    revision: u64,
    active: Option<LayerId>,
    view: ChannelView,
    foreground: [f32; 4],
    quota: BackingQuota,
    work: Arc<Work>,
    _header: QuotaLease,
    _profile: QuotaLease,
    _transform: QuotaLease,
    _query: QuotaLease,
}
impl Retention {
    pub(crate) fn matches(&self, s: &Session) -> bool {
        s.active().is_some_and(|st| {
            st.revision == self.revision
                && st.active_layer == self.active
                && st.channel_view == self.view
                && self.source.downgrade().is_current(&st.doc)
                && self.foreground.map(f32::to_bits) == s.tools.foreground.map(f32::to_bits)
        })
    }
}
/// Move-owned coverage; source, prepared CMS, and physical output reservation stay live.
pub struct ColorRangeCoverage {
    mask: Vec<f32>,
    _source: Arc<Retention>,
    _lease: QuotaLease,
}
impl ColorRangeCoverage {
    pub fn values(&self) -> &[f32] {
        &self.mask
    }
}
/// Original-coordinate RGB samples with their exact source/preparation ownership.
pub struct BackedColorRangePixels {
    rect: Rect,
    pixels: Vec<[f32; 4]>,
    source: Arc<Retention>,
    _lease: QuotaLease,
}
impl BackedColorRangePixels {
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }
}
fn reserve_owner(q: &BackingQuota, n: u64) -> Result<QuotaLease> {
    q.reserve(QuotaKind::Resident, n).map_err(failure)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| failure("backed preview size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| failure("backed preview size overflow"))
}
fn active_layer<'a>(doc: &'a Document, id: LayerId, control: &Interrupt<'_>) -> Result<&'a Layer> {
    let mut frames: [Option<std::slice::Iter<'a, Layer>>; photocraft_doc::MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *frames.first_mut().ok_or_else(|| failure("backed owner hierarchy"))? = Some(doc.layers.iter());
    let (mut depth, mut count, mut found) = (0usize, 0usize, None);
    loop {
        poll(control)?;
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        };
        count = count.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| failure("backed owner count"))?;
        if layer.id == id {
            if found.is_some() {
                return Err(failure("backed owner identity is ambiguous"));
            }
            found = Some(layer);
        }
        if let Some(children) = layer.children() {
            depth = depth.checked_add(1).filter(|n| *n <= photocraft_doc::MAX_GROUP_DEPTH).ok_or_else(|| failure("backed owner hierarchy"))?;
            *frames.get_mut(depth).ok_or_else(|| failure("backed owner hierarchy"))? = Some(children.iter());
        }
    }
    found.ok_or_else(|| failure("backed preview missing active owner"))
}
fn selected<'a>(session: &'a Session, p: &Value, control: &Interrupt<'_>) -> Result<&'a Layer> {
    poll(control)?;
    envelope(p)?;
    target(session, p)?;
    if all_layers(p) || !matches!(preset(p)?, Preset::Sampled) {
        return Err(failure("backed preview requires explicit active-only Sampled Colors"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.accounted().is_none() {
        return Err(failure("backed preview requires the actual accounted source owner"));
    }
    let doc = &state.doc;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300_000 || doc.size.height > 300_000 {
        return Err(failure("backed preview canvas dimensions"));
    }
    if b(p, "detectFaces", false) {
        return Err(failure("backed preview face detection unsupported"));
    }
    let policy = crate::imported_backed_sampling::metadata(doc)?;
    if doc.mode != photocraft_doc::ColorMode::Rgb
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || (policy == crate::imported_backed_sampling::Policy::SourceFree && doc.working_profile_interpretation != WorkingProfileInterpretation::Icc)
        || doc.icc_profile.as_ref().is_none_or(|b| b.is_empty() || b.len() > 16 << 20)
    {
        return Err(failure("backed preview explicit RGB/nonlinear/current ICC context unsupported"));
    }
    let id = state.active_layer.ok_or_else(|| failure("backed preview active layer required"))?;
    let layer = active_layer(doc, id, control)?;
    let LayerContent::BackedRaster(backed) = &layer.content else {
        return Err(failure("backed preview active native backing required"));
    };
    let format = backed.format();
    if format.mode != doc.mode
        || format.sample != doc.depth
        || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || !layer.effects.items.is_empty()
        || layer.effects.psd_raw.is_some()
        || layer.indexed_source.is_some()
        || layer.video.is_some()
        || layer.fill_cache.is_some()
    {
        return Err(failure("backed preview native format or active owner semantics unsupported"));
    }
    if !backed.quota().shares_ledger(state.doc.quota().ok_or_else(|| failure("backed preview source quota missing"))?) {
        return Err(failure("backed preview source/backing quota identity"));
    }
    crate::imported_backed_sampling::layer(doc, layer)?;
    crate::selection_sample_policy::existing_selection(session, CMD, control)?;
    poll(control)?;
    Ok(layer)
}
pub(super) fn metadata(session: &Session, p: &Value, control: &Interrupt<'_>) -> Result<()> {
    selected(session, p, control).map(|_| ())
}
fn view_copy(view: &ChannelView, work: &Work, control: &Interrupt<'_>, parent: Option<&ReadControl<'_>>) -> Result<ChannelView> {
    fn flags(src: &[bool], work: &Work, control: &Interrupt<'_>, parent: Option<&ReadControl<'_>>) -> Result<Vec<bool>> {
        let mut out = reserve(src.len())?;
        for part in src.chunks(256) {
            work.consume(part.len() as u64, control, parent)?;
            out.extend_from_slice(part);
        }
        Ok(out)
    }
    Ok(ChannelView {
        target: view.target,
        color_hidden: flags(&view.color_hidden, work, control, parent)?,
        alpha_visible: flags(&view.alpha_visible, work, control, parent)?,
        quick_mask_hidden: view.quick_mask_hidden,
        layer_mask: view.layer_mask,
        editing_mask: view.editing_mask,
    })
}
/// Metadata-only, move-owned query capture. No CMS construction or backing read occurs here.
// Values precede their physical leases even on an early worker return.
// Keep this owner intact; destructuring values and guards into separate locals
// would reverse their destruction order.
struct CapturedQueryOwners {
    params: Value,
    view: ChannelView,
    _params: QuotaLease,
    _header: QuotaLease,
}
pub struct CapturedBackedColorRange {
    policy: crate::imported_backed_sampling::Policy,
    source: DocumentSnapshot,
    revision: u64,
    active: Option<LayerId>,
    foreground: [f32; 4],
    owners: CapturedQueryOwners,
    work: Arc<Work>,
}
fn param_envelope(value: &Value, depth: usize, work: &Work, control: &Interrupt<'_>, parent: Option<&ReadControl<'_>>) -> Result<u64> {
    if depth > 4 {
        return Err(failure("captured query depth"));
    }
    work.consume(64, control, parent)?;
    let header = std::mem::size_of::<Value>() as u64;
    let body = match value {
        Value::Array(values) => {
            let mut bytes = mul(values.capacity() as u64, header)?;
            bytes = add(bytes, mul(values.len() as u64, header * 2)?)?;
            for value in values {
                bytes = add(bytes, param_envelope(value, depth + 1, work, control, parent)?)?;
            }
            bytes
        }
        Value::Object(values) => {
            let mut bytes = mul(values.len() as u64, 3 * (header + std::mem::size_of::<String>() as u64 + 128))?;
            for (key, value) in values {
                work.consume(key.len() as u64, control, parent)?;
                bytes = add(bytes, add(key.capacity() as u64, mul(key.len() as u64, 2)?)?)?;
                bytes = add(bytes, param_envelope(value, depth + 1, work, control, parent)?)?;
            }
            bytes
        }
        Value::String(value) => {
            work.consume(value.len() as u64, control, parent)?;
            add(value.capacity() as u64, mul(value.len() as u64, 2)?)?
        }
        _ => 0,
    };
    add(header, body).and_then(|n| if n <= 8 << 20 { Ok(n) } else { Err(failure("captured parameter capacity limit")) })
}
fn param_copy(value: &Value, depth: usize, work: &Work, control: &Interrupt<'_>, parent: Option<&ReadControl<'_>>) -> Result<Value> {
    if depth > 4 {
        return Err(failure("captured query depth"));
    }
    work.consume(64, control, parent)?;
    Ok(match value {
        Value::Array(values) => {
            let mut out = reserve(values.len())?;
            for value in values {
                out.push(param_copy(value, depth + 1, work, control, parent)?);
            }
            Value::Array(out)
        }
        Value::Object(values) => {
            let mut out = serde_json::Map::new();
            for (key, value) in values {
                work.consume(key.len() as u64, control, parent)?;
                let mut copied = String::new();
                copied.try_reserve_exact(key.len()).map_err(|_| failure("captured key allocation"))?;
                if copied.capacity() > key.len().saturating_mul(2) {
                    return Err(failure("captured key capacity"));
                }
                copied.push_str(key);
                out.insert(copied, param_copy(value, depth + 1, work, control, parent)?);
            }
            Value::Object(out)
        }
        Value::String(value) => {
            work.consume(value.len() as u64, control, parent)?;
            let mut out = String::new();
            out.try_reserve_exact(value.len()).map_err(|_| failure("captured string allocation"))?;
            if out.capacity() > value.len().saturating_mul(2) {
                return Err(failure("captured string capacity"));
            }
            out.push_str(value);
            Value::String(out)
        }
        value => value.clone(),
    })
}
impl CapturedBackedColorRange {
    pub fn capture_with_read_control(session: &Session, p: &Value, control: &ReadControl<'_>) -> Result<Self> {
        capture(session, p, control.interrupt, Some(control))
    }
    pub fn prepare_with_read_control(self, control: &ReadControl<'_>) -> Result<crate::selection_cmds::BackedColorRangeSamples> {
        Ok(crate::selection_cmds::BackedColorRangeSamples { query: prepare_captured(self, control.interrupt, Some(control))? })
    }
}
fn capture(session: &Session, p: &Value, control: &Interrupt<'_>, parent: Option<&ReadControl<'_>>) -> Result<CapturedBackedColorRange> {
    let work = Work { used: AtomicU64::new(0) };
    work.consume(100_000, control, parent)?;
    selected(session, p, control)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let quota = state.doc.quota().ok_or_else(|| failure("backed preview quota missing"))?;
    let view = &state.channel_view;
    if view.color_hidden.len() > 100_000 || view.alpha_visible.len() > 100_000 {
        return Err(failure("backed preview view count"));
    }
    let header_bytes = add(
        (std::mem::size_of::<Retention>() + std::mem::size_of::<CapturedBackedColorRange>() + std::mem::size_of::<Work>() + 256) as u64,
        add(
            add(view.color_hidden.capacity() as u64, view.alpha_visible.capacity() as u64)?,
            add(view.color_hidden.len() as u64, view.alpha_visible.len() as u64)?,
        )?,
    )?;
    let header = reserve_owner(quota, header_bytes)?;
    // Actual held capacity plus prospective copied values/maps are admitted
    // before the first copy; the closed envelope limits logical shapes too.
    let param_bytes = param_envelope(p, 0, &work, control, parent)?;
    let params_lease = quota.reserve(QuotaKind::Index, param_bytes).map_err(failure)?;
    let view = view_copy(view, &work, control, parent)?;
    let params = param_copy(p, 0, &work, control, parent)?;
    Ok(CapturedBackedColorRange {
        policy: crate::imported_backed_sampling::metadata(&state.doc)?,
        source: state.doc.clone(),
        revision: state.revision,
        active: state.active_layer,
        foreground: session.tools.foreground,
        owners: CapturedQueryOwners { params, view, _params: params_lease, _header: header },
        work: Arc::new(work),
    })
}
pub(in crate::selection_cmds) fn prepare(session: &Session, p: &Value, control: &Interrupt<'_>, parent: Option<&ReadControl<'_>>) -> Result<ColorRangeSamples> {
    let result = prepare_captured(capture(session, p, control, parent)?, control, parent)?;
    if !result.backed.as_ref().is_some_and(|r| r.matches(session)) {
        return Err(failure("backed preview captured target changed"));
    }
    Ok(result)
}
fn prepare_captured(captured: CapturedBackedColorRange, control: &Interrupt<'_>, parent: Option<&ReadControl<'_>>) -> Result<ColorRangeSamples> {
    let CapturedBackedColorRange { policy, source, revision, active, foreground, owners, work } = captured;
    let p = &owners.params;
    let sample_source = source.clone();
    {
        let charge =
            |n| work.consume(n, control, parent).map_err(|e| if matches!(e, EngineError::Cancelled) { BackingError::Cancelled } else { BackingError::Work });
        crate::imported_backed_sampling::verify(&sample_source, policy, &ReadControl { interrupt: control, work: &charge })?;
    }
    work.consume(100_000, control, parent)?;
    let layer = active_layer(&sample_source, active.ok_or_else(|| failure("captured active owner missing"))?, control)?;
    let LayerContent::BackedRaster(backed) = &layer.content else {
        return Err(failure("captured backing missing"));
    };
    let quota = source.quota().ok_or_else(|| failure("captured source quota missing"))?.clone();
    let points = ["points", "subtractPoints"].iter().map(|k| p.get(k).and_then(Value::as_array).map_or(0, Vec::len)).sum::<usize>();
    if points > 4096 {
        return Err(failure("backed preview combined point limit"));
    }
    let fixed = p.get("colors").and_then(Value::as_array).map_or(0, Vec::len) + usize::from(p.get("color").is_some());
    // Existing reserve helper requires exact capacities. Covers every simultaneous
    // point/order/fixed-Lab/cluster vector in the unchanged query builder.
    let query_bytes = add(
        mul(points as u64, (std::mem::size_of::<sel::RangeSample>() + 1 + 8) as u64)?,
        add(
            mul(fixed as u64, 24)?,
            (std::mem::size_of::<ColorRangeSamples>() + std::mem::size_of::<QueryContext>() + 8 * std::mem::size_of::<Vec<u8>>()) as u64,
        )?,
    )?;
    let query_lease = reserve_owner(&quota, query_bytes)?;
    let cancel = || control.cancelled();
    let cms_limits = photocraft_cms::control::Limits::default();
    let icc = source.icc_profile.as_ref().ok_or_else(|| failure("backed preview ICC missing"))?;
    work.consume(64, control, parent)?;
    let parse_ceiling = photocraft_cms::Profile::parse_planning_work_ceiling(icc.len()).map_err(cms_error)?;
    let parse_limits = photocraft_cms::control::Limits { work_units: parse_ceiling, ..cms_limits };
    work.consume(parse_ceiling, control, parent)?;
    let profile_req = photocraft_cms::Profile::parse_requirements_with_control(icc, parse_limits, &cancel).map_err(cms_error)?;
    let profile_lease = reserve_owner(&quota, profile_req.peak_resident_bytes as u64)?;
    work.consume(profile_req.work_units, control, parent)?;
    let profile = photocraft_cms::Profile::parse_with_control(
        icc,
        photocraft_cms::control::Limits { resident_bytes: profile_req.peak_resident_bytes, work_units: profile_req.work_units },
        &cancel,
    )
    .map_err(cms_error)?;
    if profile.color_space != photocraft_cms::ColorSpace::Rgb
        || profile.class == photocraft_cms::ProfileClass::DeviceLink
        || profile.matrix.is_none()
        || profile.trc.is_none()
        || profile.a2b.iter().any(Option::is_some)
        || profile.b2a.iter().any(Option::is_some)
    {
        return Err(failure("backed preview requires the exact RGB matrix/TRC ICC domain"));
    }
    // Known Lab destination provider initialization remains an outer-polled
    // bounded provider boundary, identical to the ordinary query target.
    poll(control)?;
    let lab = photocraft_cms::Builtin::LabD50.profile();
    poll(control)?;
    work.consume(photocraft_cms::Transform::RGB_TO_BUILTIN_LAB_PLANNING_WORK, control, parent)?;
    let prospective = photocraft_cms::Transform::rgb_to_builtin_lab_planning_requirements_with_control(&profile, cms_limits, &cancel).map_err(cms_error)?;
    let transform_limits = photocraft_cms::control::Limits { work_units: prospective.work_units, ..cms_limits };
    work.consume(prospective.work_units, control, parent)?;
    let options = photocraft_cms::TransformOptions::default();
    let transform_req = photocraft_cms::Transform::requirements_with_control(&profile, lab, options, transform_limits, &cancel).map_err(cms_error)?;
    if transform_req.work_units > prospective.work_units
        || transform_req.peak_resident_bytes > prospective.peak_resident_bytes
        || transform_req.retained_bytes > prospective.retained_bytes
    {
        return Err(failure("actual transform requirements exceeded prospective envelope"));
    }
    let transform_lease = reserve_owner(&quota, transform_req.peak_resident_bytes as u64)?;
    work.consume(transform_req.work_units, control, parent)?;
    let to_lab = photocraft_cms::Transform::with_options_with_control(
        &profile,
        lab,
        options,
        photocraft_cms::control::Limits { resident_bytes: transform_req.peak_resident_bytes, work_units: transform_req.work_units },
        &cancel,
    )
    .map_err(cms_error)?;
    // Actual retained census charges stage/vector counts only; the already
    // admitted construction envelope (>=1Mi work) bounds this closed result.
    work.consume(transform_req.work_units, control, parent)?;
    let retention_limits = photocraft_cms::control::Limits { work_units: transform_req.work_units, ..cms_limits };
    if to_lab.retained_bytes_with_control(retention_limits, &cancel).map_err(cms_error)? > transform_req.retained_bytes {
        return Err(failure("backed preview actual transform exceeded prepared envelope"));
    }
    let retention = Arc::new(Retention {
        source,
        revision,
        active,
        view: owners.view,
        foreground,
        quota,
        work,
        _header: owners._header,
        _profile: profile_lease,
        _transform: transform_lease,
        _query: query_lease,
    });
    let context = QueryContext { to_lab: Arc::new(to_lab), grid: sample_source.depth == SampleType::U8, foreground, backed: Some(Arc::clone(&retention)) };
    let (exact, exact_limits) = exact_plan(&context.to_lab, &retention, retention._query.bytes(), control, parent)?;
    let point_work = exact.requirements_rgb(1, exact_limits, &|| control.cancelled()).map_err(cms_error)?.work_units;
    let conversion_bound = points.checked_add(fixed).and_then(|n| n.checked_add(1)).ok_or_else(|| failure("query conversion count"))?;
    retention.work.consume(mul(conversion_bound as u64, add(point_work, 512)?)?, control, parent)?;
    let format = backed.format();
    let charge = |n| {
        retention.work.consume(n, control, parent).map_err(|e| if matches!(e, EngineError::Cancelled) { BackingError::Cancelled } else { BackingError::Work })
    };
    let read = ReadControl { interrupt: control, work: &charge };
    let sample = |key: &str| -> Result<Vec<sel::RangeSample>> {
        let list = p.get(key).and_then(Value::as_array);
        let mut out = reserve(list.map_or(0, Vec::len))?;
        for point in list.into_iter().flatten() {
            retention.work.consume(1, control, parent)?;
            let pair = point.as_array().ok_or_else(|| failure("sample point shape"))?;
            let x = pair.first().and_then(Value::as_f64).ok_or_else(|| failure("sample x"))?.floor() as i32;
            let y = pair.get(1).and_then(Value::as_f64).ok_or_else(|| failure("sample y"))?.floor() as i32;
            if !sample_source.bounds().contains(x, y) || !backed.operative_bounds().contains(x, y) {
                return Err(failure("backed sample outside explicit canvas/native extent"));
            }
            if super::super::mode(p) == sel::SelectionMode::Replace
                && sample_source.selection.is_some()
                && previous(&sample_source, x, y, control)?.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            {
                return Err(failure("sample point outside active selection"));
            }
            let mut raw = [0u8; 40];
            let bytes = raw.get_mut(..format.bytes_per_pixel()).ok_or_else(|| failure("backed sample layout"))?;
            backed.checked_view().read_logical_pixel_native_into(x, y, bytes, &read).map_err(|e| error(control, e))?;
            let mut rgba = [0.0f32; 4];
            rgba[3] = 1.0;
            for (i, v) in rgba.get_mut(..format.channels()).ok_or_else(|| failure("backed sample channels"))?.iter_mut().enumerate() {
                *v = read_sample(bytes, format.sample, i);
            }
            if !rgba.iter().all(|v| v.is_finite()) {
                return Err(failure("finite backed sample required"));
            }
            out.push(sel::RangeSample {
                color: [rgba[0], rgba[1], rgba[2]],
                at: Some(((x - sample_source.bounds().x0) as f32, (y - sample_source.bounds().y0) as f32)),
            });
        }
        Ok(out)
    };
    let plus = sample("points")?;
    let minus = sample("subtractPoints")?;
    let result = with_context_conversion(&context, p, sample_source.bounds(), &plus, &minus, control, Some((&exact, exact_limits)))?;
    poll(control)?;
    Ok(result)
}
pub(in crate::selection_cmds) fn charge_preview(
    query: &ColorRangeSamples,
    units: u64,
    control: &Interrupt<'_>,
    parent: Option<&ReadControl<'_>>,
) -> Result<()> {
    query.backed.as_ref().ok_or_else(|| failure("preview work requires a backed prepared query"))?.work.consume(units, control, parent)
}
// Keep the explicit authority, borrowed payload and original-control proof inputs together.
#[allow(clippy::too_many_arguments)]
pub(in crate::selection_cmds) fn owned_coverage(
    query: &ColorRangeSamples,
    px: &[[f32; 4]],
    width: usize,
    scale: f32,
    origin: [f32; 2],
    limits: sel::ColorRangeLimits,
    control: &Interrupt<'_>,
    parent: Option<&ReadControl<'_>>,
) -> Result<ColorRangeCoverage> {
    let owner = query.backed.as_ref().ok_or_else(|| failure("owned coverage requires a backed prepared query"))?;
    poll(control)?;
    let bytes = add(mul(px.len() as u64, 36)?, add(mul(query.plus_at.capacity() as u64, 16)?, 2 * 512 * 16)?)?;
    if bytes > limits.resident_bytes {
        return Err(failure("backed coverage lower resident limit"));
    }
    let lease = reserve_owner(&owner.quota, bytes)?;
    let (exact, exact_limits) = exact_plan(&query.to_lab, owner, lease.bytes(), control, parent)?;
    let (units, _) = coverage_work(query, px.len(), Some((&exact, exact_limits)), control)?;
    if units > limits.work_units {
        return Err(failure("backed coverage lower work limit"));
    }
    owner.work.consume(units, control, parent)?;
    let mask = coverage_inner_conversion(query, px, width, scale, origin, limits, control, Some((&exact, exact_limits)))?;
    if mul(mask.capacity() as u64, 4)? > bytes {
        return Err(failure("backed actual output exceeded reservation"));
    }
    poll(control)?;
    Ok(ColorRangeCoverage { mask, _source: Arc::clone(owner), _lease: lease })
}
pub(in crate::selection_cmds) fn original_pixels(
    query: &ColorRangeSamples,
    rect: Rect,
    control: &Interrupt<'_>,
    parent: Option<&ReadControl<'_>>,
) -> Result<BackedColorRangePixels> {
    poll(control)?;
    let owner = query.backed.as_ref().ok_or_else(|| failure("original backed pixels require a backed query"))?;
    owner.work.consume(100_000, control, parent)?;
    let layer = active_layer(&owner.source, owner.active.ok_or_else(|| failure("original backing missing owner"))?, control)?;
    let LayerContent::BackedRaster(backed) = &layer.content else {
        return Err(failure("original backing missing"));
    };
    if rect.is_empty() || !owner.source.bounds().contains_rect(&rect) || !backed.operative_bounds().contains_rect(&rect) {
        return Err(failure("original pixel rectangle outside explicit native/canvas extent"));
    }
    let bytes = photocraft_raster::checked_region_byte_len(rect, PixelFormat::RGBA32F).map_err(|e| error(control, e))?;
    let envelope = add(mul(bytes as u64, 2)?, std::mem::size_of::<BackedColorRangePixels>() as u64)?;
    let lease = reserve_owner(&owner.quota, envelope)?;
    owner.work.consume(bytes as u64, control, parent)?;
    let mut pixels = reserve(bytes / 16)?;
    let count = bytes / 16;
    for start in (0..count).step_by(4096) {
        poll(control)?;
        pixels.resize(start.saturating_add(4096).min(count), [0.0; 4]);
    }
    let format = backed.format();
    let charge =
        |n| owner.work.consume(n, control, parent).map_err(|e| if matches!(e, EngineError::Cancelled) { BackingError::Cancelled } else { BackingError::Work });
    let read = ReadControl { interrupt: control, work: &charge };
    // One physical pin per tile. Never rebuild a full source map or repeatedly
    // decode a complete tile for each pixel row.
    let width = rect.width() as usize;
    for coord in rect.tiles() {
        let pin = backed.checked_view().pin_tile(coord, &read).map_err(|e| error(control, e))?;
        let x0 = coord.tx.checked_mul(TILE_SIZE).ok_or_else(|| failure("original tile origin"))?;
        let y0 = coord.ty.checked_mul(TILE_SIZE).ok_or_else(|| failure("original tile origin"))?;
        let tile = Rect::new(
            x0,
            y0,
            x0.checked_add(TILE_SIZE).ok_or_else(|| failure("original tile end"))?,
            y0.checked_add(TILE_SIZE).ok_or_else(|| failure("original tile end"))?,
        );
        let area = rect.intersect(&tile);
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let mut raw = [0u8; 40];
                let encoded = raw.get_mut(..format.bytes_per_pixel()).ok_or_else(|| failure("original pixel format"))?;
                pin.read_pixel_native_into(x, y, encoded, &read).map_err(|e| error(control, e))?;
                let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| failure("original row"))?;
                let col = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| failure("original column"))?;
                let index = row.checked_mul(width).and_then(|n| n.checked_add(col)).ok_or_else(|| failure("original pixel index"))?;
                let output = pixels.get_mut(index).ok_or_else(|| failure("original pixel length"))?;
                output[3] = 1.0;
                for (i, v) in output.get_mut(..format.channels()).ok_or_else(|| failure("original channels"))?.iter_mut().enumerate() {
                    *v = read_sample(encoded, format.sample, i);
                }
                if !output.iter().all(|v| v.is_finite()) {
                    return Err(failure("finite original sample required"));
                }
            }
        }
    }
    if mul(pixels.capacity() as u64, 16)? > envelope {
        return Err(failure("original output exceeded reservation"));
    }
    poll(control)?;
    Ok(BackedColorRangePixels { rect, pixels, source: Arc::clone(owner), _lease: lease })
}
pub(in crate::selection_cmds) fn paired_coverage(
    query: &ColorRangeSamples,
    pixels: &BackedColorRangePixels,
    limits: sel::ColorRangeLimits,
    control: &Interrupt<'_>,
    parent: Option<&ReadControl<'_>>,
) -> Result<ColorRangeCoverage> {
    if !query.backed.as_ref().is_some_and(|owner| Arc::ptr_eq(owner, &pixels.source)) {
        return Err(failure("backed pixel/query owner mismatch"));
    }
    owned_coverage(query, &pixels.pixels, pixels.rect.width() as usize, 1.0, [pixels.rect.x0 as f32, pixels.rect.y0 as f32], limits, control, parent)
}
