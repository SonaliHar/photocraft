use crate::*;
use std::collections::HashSet;
use std::ops::Deref;
pub(crate) enum Input<'a> {
    Bytes(&'a [u8]),
    Source(&'a mut dyn Source),
}
impl Input<'_> {
    fn len(&self) -> u64 {
        match self {
            Self::Bytes(b) => b.len() as u64,
            Self::Source(s) => s.len(),
        }
    }
}
/// Owned temporary bytes precede their lease so allocation dies before refund.
pub(crate) struct ReadOwned<'b, 'a> {
    bytes: Vec<u8>,
    _lease: Reservation<'b, 'a>,
}
pub(crate) enum ReadBytes<'s, 'b, 'a> {
    Borrowed(&'s [u8]),
    Inline([u8; 16], usize),
    Owned(ReadOwned<'b, 'a>),
}
impl Deref for ReadBytes<'_, '_, '_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Borrowed(b) => b,
            Self::Inline(b, n) => b.get(..*n).unwrap_or(&[]),
            Self::Owned(b) => &b.bytes,
        }
    }
}
impl AsRef<[u8]> for ReadBytes<'_, '_, '_> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

pub(crate) struct Reader<'a, 'o> {
    input: Input<'a>,
    pub pos: u64,
    pub version: u32,
    metadata_only: bool,
    pub options: &'o DecodeOptions<'o>,
    pub limits: Limits,
    pub charged: usize,
    resident: Option<Reservation<'o, 'o>>,
    resident_at_entry: u64,
    pub metadata: usize,
    pub visits: usize,
    pub props: usize,
    pub drawable_count: usize,
    pub pixels: u64,
    pub budget: Option<&'o Budget<'o>>,
    pub effect_depth: usize,
    pub text_bytes: usize,
    pub config_entries: u64,
    pub graph_nodes: u64,
    pub graph_edges: u64,
    pub effect_count: usize,
    seen: HashSet<u64>,
    progress: f32,
}
impl<'a, 'o> Reader<'a, 'o> {
    pub fn error(&self, kind: ErrorKind, message: impl Into<String>) -> XcfError {
        XcfError::at(kind, self.pos, message)
    }
    pub fn poll(&mut self) -> Result<(), XcfError> {
        if let Some(budget) = self.budget {
            budget.poll("XCF record")?;
            budget.consume_visits(1, "XCF record")?;
        }
        if self.options.cancelled.is_some_and(|f| f()) {
            return Err(self.error(ErrorKind::Cancelled, "cancelled"));
        }
        self.visits = self.visits.checked_add(1).ok_or_else(|| self.error(ErrorKind::Limit, "visit overflow"))?;
        if self.visits > self.limits.visits {
            return Err(self.error(ErrorKind::Limit, "record/work visit limit"));
        }
        Ok(())
    }
    pub fn progress(&mut self, p: f32) {
        self.progress = self.progress.max(p.clamp(0.0, 1.0));
        if let Some(f) = self.options.progress {
            f(self.progress);
        }
    }
    pub fn charge(&mut self, n: usize) -> Result<(), XcfError> {
        self.poll()?;
        self.charged = self.charged.checked_add(n).ok_or_else(|| self.error(ErrorKind::Limit, "resident byte overflow"))?;
        if let Some(budget) = self.budget {
            let aggregate = budget
                .used(BudgetKind::Resident)
                .checked_sub(self.resident.as_ref().map_or(0, Reservation::bytes))
                .and_then(|used| used.checked_add(self.charged as u64))
                .ok_or_else(|| self.error(ErrorKind::Limit, "aggregate resident overflow"))?;
            budget.check(BudgetKind::Resident, aggregate, "XCF allocation")?;
            let ceiling = self
                .resident_at_entry
                .checked_add(self.limits.resident_bytes as u64)
                .ok_or_else(|| self.error(ErrorKind::Limit, "resident policy overflow"))?;
            if aggregate > ceiling {
                return Err(self.error(ErrorKind::Limit, "lowered aggregate resident budget exceeded"));
            }
        }
        if self.charged > self.limits.resident_bytes {
            return Err(self.error(ErrorKind::Limit, "resident memory budget exceeded"));
        }
        if let Some(lease) = &mut self.resident {
            lease.resize(self.charged as u64, "XCF live parsed allocation")?;
        }
        Ok(())
    }
    pub fn vec(&mut self, n: usize) -> Result<Vec<u8>, XcfError> {
        // Vec cannot hold more than isize::MAX bytes; preserve allocator-denial
        // taxonomy after the normal policy check, without attempting allocation.
        if n > isize::MAX as usize {
            self.charge(n)?;
            return Err(self.error(ErrorKind::Allocation, "pixel allocation"));
        }
        let envelope = n.checked_mul(2).ok_or_else(|| self.error(ErrorKind::Limit, "allocation capacity envelope"))?;
        self.charge(envelope)?;
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| self.error(ErrorKind::Allocation, "pixel allocation"))?;
        if v.capacity() > envelope {
            return Err(self.error(ErrorKind::Limit, "allocation capacity exceeded envelope"));
        }
        self.charged = self.charged.checked_sub(envelope - v.capacity()).ok_or_else(|| self.error(ErrorKind::Structure, "allocation capacity accounting"))?;
        if let Some(lease) = &mut self.resident {
            lease.resize(self.charged as u64, "XCF actual allocation capacity")?;
        }
        let zero = [0u8; 65536];
        while v.len() < n {
            self.poll()?;
            let chunk = (n - v.len()).min(zero.len());
            v.extend_from_slice(zero.get(..chunk).ok_or_else(|| self.error(ErrorKind::Structure, "zero fill extent"))?);
        }
        Ok(v)
    }
    pub fn len(&self) -> u64 {
        self.input.len()
    }
    pub fn read_at(&mut self, offset: u64, output: &mut [u8]) -> Result<(), XcfError> {
        let end = offset.checked_add(output.len() as u64).ok_or_else(|| self.error(ErrorKind::Structure, "offset overflow"))?;
        if end > self.len() {
            return Err(self.error(ErrorKind::Truncated, "truncated record"));
        }
        match &mut self.input {
            Input::Bytes(bytes) => {
                let start = usize::try_from(offset).map_err(|_| XcfError::at(ErrorKind::Limit, offset, "local slice offset"))?;
                let end = usize::try_from(end).map_err(|_| XcfError::at(ErrorKind::Limit, offset, "local slice end"))?;
                let input = bytes.get(start..end).ok_or_else(|| XcfError::at(ErrorKind::Truncated, offset, "slice extent"))?;
                for (out, src) in output.chunks_mut(4096).zip(input.chunks(4096)) {
                    if let Some(budget) = self.budget {
                        budget.poll("plain slice copy")?;
                    }
                    if self.options.cancelled.is_some_and(|cancel| cancel()) {
                        return Err(XcfError::at(ErrorKind::Cancelled, offset, "cancelled during slice copy"));
                    }
                    out.copy_from_slice(src);
                }
            }
            Input::Source(source) => {
                let budget = self.budget.ok_or_else(|| XcfError::at(ErrorKind::Limit, offset, "source control missing"))?;
                let mut at = offset;
                for chunk in output.chunks_mut(4096) {
                    budget.poll("plain source read")?;
                    if self.options.cancelled.is_some_and(|cancel| cancel()) {
                        budget.mark_cancelled();
                        return Err(XcfError::at(ErrorKind::Cancelled, at, "cancelled during source read"));
                    }
                    budget.consume_work(chunk.len() as u64, "plain parser bytes")?;
                    source.read_exact_at(at, chunk, budget)?;
                    at = at.checked_add(chunk.len() as u64).ok_or_else(|| XcfError::at(ErrorKind::Structure, at, "source offset overflow"))?;
                }
            }
        }
        Ok(())
    }
    pub fn take(&mut self, n: usize) -> Result<ReadBytes<'a, 'o, 'o>, XcfError> {
        let start = self.pos;
        let end = start.checked_add(n as u64).ok_or_else(|| self.error(ErrorKind::Structure, "offset overflow"))?;
        if end > self.len() {
            return Err(self.error(ErrorKind::Truncated, "truncated record"));
        }
        let data = if let Input::Bytes(bytes) = &self.input {
            let start = usize::try_from(start).map_err(|_| self.error(ErrorKind::Limit, "slice offset"))?;
            let end = usize::try_from(end).map_err(|_| self.error(ErrorKind::Limit, "slice end"))?;
            ReadBytes::Borrowed(bytes.get(start..end).ok_or_else(|| self.error(ErrorKind::Truncated, "record extent"))?)
        } else if n <= 16 {
            let mut bytes = [0; 16];
            self.read_at(start, bytes.get_mut(..n).ok_or_else(|| self.error(ErrorKind::Structure, "inline extent"))?)?;
            ReadBytes::Inline(bytes, n)
        } else {
            let budget = self.budget.ok_or_else(|| self.error(ErrorKind::Limit, "source control missing"))?;
            let envelope = (n as u64).checked_mul(2).ok_or_else(|| self.error(ErrorKind::Limit, "read allocation envelope"))?;
            let lease = budget.reserve(BudgetKind::Resident, envelope, "plain read buffer")?;
            let mut owned = ReadOwned { bytes: Vec::new(), _lease: lease };
            owned.bytes.try_reserve_exact(n).map_err(|_| self.error(ErrorKind::Allocation, "plain read buffer"))?;
            let capacity = owned.bytes.capacity() as u64;
            if capacity > envelope {
                return Err(self.error(ErrorKind::Limit, "plain read capacity"));
            }
            owned._lease.resize(capacity, "plain read capacity")?;
            let zero = [0; 4096];
            while owned.bytes.len() < n {
                budget.poll("plain read initialize")?;
                let count = (n - owned.bytes.len()).min(zero.len());
                owned.bytes.extend_from_slice(zero.get(..count).ok_or_else(|| self.error(ErrorKind::Structure, "initialize extent"))?);
            }
            self.read_at(start, &mut owned.bytes)?;
            ReadBytes::Owned(owned)
        };
        self.pos = end;
        Ok(data)
    }
    pub fn u32(&mut self) -> Result<u32, XcfError> {
        let b = self.take(4)?;
        let a: [u8; 4] = b.as_ref().try_into().map_err(|_| self.error(ErrorKind::Truncated, "u32"))?;
        Ok(u32::from_be_bytes(a))
    }
    pub fn i32(&mut self) -> Result<i32, XcfError> {
        Ok(self.u32()? as i32)
    }
    pub fn float(&mut self) -> Result<f32, XcfError> {
        let f = f32::from_bits(self.u32()?);
        if !f.is_finite() {
            return Err(self.error(ErrorKind::Structure, "nonfinite float"));
        }
        Ok(f)
    }
    pub fn pointer(&mut self) -> Result<u64, XcfError> {
        let p = self.raw_pointer()?;
        if p != 0 && (p < 14 || p >= self.len()) {
            return Err(self.error(ErrorKind::Structure, "pointer outside input"));
        }
        Ok(p)
    }
    fn raw_pointer(&mut self) -> Result<u64, XcfError> {
        Ok(if self.version >= 11 {
            let b = self.take(8)?;
            let a: [u8; 8] = b.as_ref().try_into().map_err(|_| self.error(ErrorKind::Truncated, "pointer"))?;
            u64::from_be_bytes(a)
        } else {
            u64::from(self.u32()?)
        })
    }
    pub fn hierarchy_level(&mut self) -> Result<Option<u64>, XcfError> {
        let first = self.pointer()?;
        if first == 0 {
            return Ok(None);
        }
        // Lower-resolution levels are obsolete, ignored cache pointers. Some valid
        // recompressed files retain stale dummy offsets; never dereference them.
        for _ in 0..=self.limits.group_depth {
            self.poll()?;
            if self.raw_pointer()? == 0 {
                return Ok(Some(first));
            }
        }
        Err(self.error(ErrorKind::Limit, "dummy level count limit"))
    }
    pub fn enter(&mut self, p: u64) -> Result<(), XcfError> {
        self.poll()?;
        if p == 0 || p >= self.len() {
            return Err(self.error(ErrorKind::Structure, "missing record pointer"));
        }
        self.charge(64)?;
        self.seen.try_reserve(1).map_err(|_| self.error(ErrorKind::Allocation, "traversal set"))?;
        if !self.seen.insert(p) {
            return Err(self.error(ErrorKind::Structure, "cycle/aliased structure pointer"));
        }
        self.pos = p;
        Ok(())
    }
    pub fn string(&mut self) -> Result<String, XcfError> {
        let n = self.u32()? as usize;
        if n > self.limits.string_bytes {
            return Err(self.error(ErrorKind::Limit, "string byte limit"));
        }
        self.charge(n)?;
        let b = self.take(n)?;
        if n == 0 {
            return Ok(String::new());
        }
        if b.last() != Some(&0) {
            return Err(self.error(ErrorKind::Structure, "string is not NUL terminated"));
        }
        let text = std::str::from_utf8(b.get(..n - 1).ok_or_else(|| self.error(ErrorKind::Structure, "string length"))?)
            .map_err(|_| self.error(ErrorKind::Structure, "invalid UTF-8 name"))?;
        if text.contains('\0') {
            return Err(self.error(ErrorKind::Structure, "embedded NUL in string"));
        }
        let mut s = String::new();
        s.try_reserve_exact(text.len()).map_err(|_| self.error(ErrorKind::Allocation, "string allocation"))?;
        s.push_str(text);
        Ok(s)
    }
    pub fn dimensions(&mut self, w: u32, h: u32) -> Result<(), XcfError> {
        if w == 0 || h == 0 || w > self.limits.side || h > self.limits.side {
            return Err(self.error(ErrorKind::Limit, "zero/oversized drawable dimensions"));
        }
        self.pixels = self.pixels.checked_add(u64::from(w) * u64::from(h)).ok_or_else(|| self.error(ErrorKind::Limit, "pixel count overflow"))?;
        if self.pixels > self.limits.pixels {
            return Err(self.error(ErrorKind::Limit, "cumulative drawable pixel limit"));
        }
        Ok(())
    }
    pub fn pointers(&mut self, max: usize) -> Result<Vec<u64>, XcfError> {
        let mut v = Vec::new();
        loop {
            self.poll()?;
            let p = self.pointer()?;
            if p == 0 {
                break;
            }
            if v.len() >= max {
                return Err(self.error(ErrorKind::Limit, "pointer count limit"));
            }
            self.charge(std::mem::size_of::<u64>())?;
            v.try_reserve_exact(1).map_err(|_| self.error(ErrorKind::Allocation, "pointer list"))?;
            v.push(p);
        }
        Ok(v)
    }
    pub fn retain_property(&mut self, tag: u32, start: u64, end: u64, declared: u32) -> Result<RawProperty, XcfError> {
        let n = usize::try_from(end.checked_sub(start).ok_or_else(|| self.error(ErrorKind::Structure, "property extent"))?)
            .map_err(|_| self.error(ErrorKind::Limit, "property width"))?;
        self.charge(std::mem::size_of::<RawProperty>())?;
        let mut payload = self.vec(n)?;
        self.read_at(start, &mut payload)?;
        Ok(RawProperty { tag, offset: start, declared_bytes: declared, payload })
    }
    fn effect<S: PlaneSink>(&mut self, p: u64, file: &mut XcfFile<S::Plane>, sink: &mut S) -> Result<Effect<S::Plane>, XcfError> {
        self.enter(p)?;
        self.effect_count = self.effect_count.checked_add(1).ok_or_else(|| self.error(ErrorKind::Limit, "effect count overflow"))?;
        let max_depth = self.budget.map(|b| b.limits().get(BudgetKind::ConfigDepth)).unwrap_or(32);
        if self.effect_depth as u64 >= max_depth {
            return Err(self.error(ErrorKind::Limit, "nested effect/config depth limit"));
        }
        if let Some(budget) = self.budget {
            budget.check(BudgetKind::FilterNodes, self.effect_count as u64, "effect records")?;
        }
        self.effect_depth += 1;
        let result = self.effect_inner(file, sink);
        self.effect_depth -= 1;
        result
    }
    fn effect_inner<S: PlaneSink>(&mut self, file: &mut XcfFile<S::Plane>, sink: &mut S) -> Result<Effect<S::Plane>, XcfError> {
        let name = self.string()?;
        let icon = self.string()?;
        let operation = self.string()?;
        if operation.is_empty() {
            return Err(self.error(ErrorKind::Structure, "empty effect operation identity"));
        }
        let version = if self.version >= 22 { Some(self.string()?) } else { None };
        let mut properties = Vec::new();
        let mut arguments = Vec::new();
        let mut state = EffectState::default();
        let mut singleton_seen = 0u64;
        loop {
            self.poll()?;
            self.props += 1;
            if self.props > self.limits.properties {
                return Err(self.error(ErrorKind::Limit, "effect property count limit"));
            }
            let tag = self.u32()?;
            let declared = self.u32()?;
            let n = usize::try_from(declared).map_err(|_| self.error(ErrorKind::Limit, "effect property width"))?;
            if n > self.limits.property_bytes {
                return Err(self.error(ErrorKind::Limit, "effect property byte limit"));
            }
            if tag == 0 {
                if n != 0 {
                    return Err(self.error(ErrorKind::Structure, "effect END payload"));
                }
                break;
            }
            if matches!(tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 46) {
                crate::properties::unique(self, &mut singleton_seen, tag)?;
            }
            self.metadata = self.metadata.checked_add(n).ok_or_else(|| self.error(ErrorKind::Limit, "effect metadata overflow"))?;
            if self.metadata > self.limits.metadata_bytes {
                return Err(self.error(ErrorKind::Limit, "effect metadata limit"));
            }
            let start = self.pos;
            let end = start.checked_add(n as u64).ok_or_else(|| self.error(ErrorKind::Structure, "effect property extent overflow"))?;
            let property = self.retain_property(tag, start, end, declared)?;
            match tag {
                6 => {
                    let v = self.u32()?;
                    state.opacity_u8 = Some(u8::try_from(v).map_err(|_| self.error(ErrorKind::Structure, "effect opacity exceeds 255"))?);
                }
                7 => state.mode = Some(self.i32()?),
                8 => {
                    state.enabled = Some(match self.u32()? {
                        0 => false,
                        1 => true,
                        _ => return Err(self.error(ErrorKind::Structure, "effect enabled boolean")),
                    })
                }
                33 => {
                    let v = self.float()?;
                    if !(0.0..=1.0).contains(&v) {
                        return Err(self.error(ErrorKind::Structure, "effect opacity outside 0..1"));
                    }
                    state.opacity_bits = Some(v.to_bits());
                }
                35 => state.composite_mode = Some(self.i32()?),
                36 => state.composite_space = Some(self.i32()?),
                37 => state.blend_space = Some(self.i32()?),
                46 => state.clipping = Some(self.u32()?),
                44 => state.region = Some(self.u32()?),
                _ => {}
            }
            if matches!(tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 46) && self.pos != end {
                return Err(self.error(ErrorKind::Structure, "effect state property length"));
            }
            if tag == 45 {
                self.charge(std::mem::size_of::<Argument>())?;
                arguments.try_reserve_exact(1).map_err(|_| self.error(ErrorKind::Allocation, "effect arguments"))?;
                arguments.push(crate::arguments::parse(self, end, properties.len())?);
            }
            properties.try_reserve_exact(1).map_err(|_| self.error(ErrorKind::Allocation, "effect properties"))?;
            properties.push(property);
            self.pos = end;
        }
        let mask = self.pointer()?;
        for argument in &mut arguments {
            if operation == "gegl:gegl"
                && argument.name == "string"
                && let ArgumentValue::String(source) = &argument.value
            {
                argument.graph = Some(crate::graph::parse(self, source)?);
            }
            if let Some(configuration) = &mut argument.configuration {
                configuration.typed = crate::configuration::typed(self, &operation, &configuration.nodes)?;
            }
        }
        let mask = if mask == 0 {
            None
        } else {
            self.charge(std::mem::size_of::<Drawable<S::Plane>>())?;
            Some(Box::new(self.drawable(mask, file, PlaneRole::Mask, sink)?))
        };
        Ok(Effect { name, icon, operation, version, properties, arguments, mask, state })
    }
    pub fn drawable<S: PlaneSink>(&mut self, p: u64, file: &mut XcfFile<S::Plane>, role: PlaneRole, sink: &mut S) -> Result<Drawable<S::Plane>, XcfError> {
        self.drawable_inner(p, file, role, sink).map_err(|mut error| {
            if error.item.is_empty() {
                error.item = format!(" in drawable at {p}");
            }
            error
        })
    }
    fn drawable_inner<S: PlaneSink>(&mut self, p: u64, file: &mut XcfFile<S::Plane>, role: PlaneRole, sink: &mut S) -> Result<Drawable<S::Plane>, XcfError> {
        self.enter(p)?;
        let channel = role != PlaneRole::Raster;
        self.drawable_count += 1;
        if self.drawable_count > self.limits.drawables {
            return Err(self.error(ErrorKind::Limit, "drawable count limit"));
        }
        self.charge(std::mem::size_of::<Drawable<S::Plane>>())?;
        let w = self.u32()?;
        let h = self.u32()?;
        self.dimensions(w, h)?;
        let (model, alpha, channels) = if channel {
            (Model::Gray, false, 1)
        } else {
            match self.u32()? {
                0 => (Model::Rgb, false, 3),
                1 => (Model::Rgb, true, 4),
                2 => (Model::Gray, false, 1),
                3 => (Model::Gray, true, 2),
                4 => (Model::Indexed, false, 1),
                5 => (Model::Indexed, true, 2),
                _ => return Err(self.error(ErrorKind::Structure, "invalid layer type")),
            }
        };
        if !channel && model != file.model {
            return Err(self.error(ErrorKind::Structure, "layer/image color model mismatch"));
        }
        let name = self.string()?;
        let mut d = Drawable { record_offset: p, name, width: w, height: h, model, alpha, visible: !channel, ..Drawable::default() };
        crate::properties::parse(self, file, Some(&mut d)).map_err(|mut e| {
            e.item = format!(" in item '{}'", d.name.chars().take(80).collect::<String>());
            e
        })?;
        let hierarchy = self.pointer()?;
        let mask = if channel { 0 } else { self.pointer()? };
        if !channel && self.version >= 20 || channel && self.version >= 26 {
            let cap = self.budget.map(|b| b.limits().get(BudgetKind::FiltersPerOwner)).unwrap_or(1024);
            let effects = self.pointers(usize::try_from(cap).map_err(|_| self.error(ErrorKind::Limit, "effect owner count width"))?)?;
            if !effects.is_empty() {
                d.unsupported.push("GEGL layer effects".into());
            }
            for pointer in effects {
                self.charge(std::mem::size_of::<Effect<S::Plane>>())?;
                d.effects.try_reserve_exact(1).map_err(|_| self.error(ErrorKind::Allocation, "effect records"))?;
                d.effects.push(self.effect(pointer, file, sink)?);
            }
        }
        if hierarchy != 0 && !d.group {
            let descriptor = PlaneDescriptor {
                    width: w,
                    height: h,
                    channels,
                    sample: if file.sample == Sample::F16 { Sample::F32 } else { file.sample },
                    source_sample: if model == Model::Indexed { Sample::U8 } else { file.sample },
                    model,
                    alpha,
                    role,
                    record_offset: p,
                    offset: d.offset,
                };
            d.plane = Some(if self.metadata_only {
                // Metadata planning does not inspect hierarchy/tile payload and
                // is never decoded-pixel authority. Actual decode verifies it.
                let mut control=PlaneSinkControl{reader:self};
                sink.begin(&descriptor,&mut control)?;
                sink.finish(&mut control)?
            }else{crate::pixels::plane(self,hierarchy,descriptor,&d.name,file.compression,sink)?});
        } else if hierarchy != 0 {
            // GIMP 2.8 writes an empty pixel level for groups. Group appearance comes
            // from its editable children; the optional group cache is never a fallback.
            self.enter(hierarchy)?;
            let cache_width = self.u32()?;
            let cache_height = self.u32()?;
            let cache_bpp = self.u32()? as usize;
            let sample = if model == Model::Indexed { Sample::U8 } else { file.sample };
            if cache_width != w || cache_height != h || cache_bpp != channels * sample.source_bytes() {
                return Err(self.error(ErrorKind::Structure, "group hierarchy dimensions/stride mismatch"));
            }
            let _ = self.hierarchy_level()?;
        } else if !d.group {
            return Err(self.error(ErrorKind::Structure, "raster layer missing pixels"));
        }
        if mask != 0 {
            self.charge(std::mem::size_of::<Drawable<S::Plane>>())?;
            let m = self.drawable(mask, file, PlaneRole::Mask, sink)?;
            d.mask = Some(Box::new(m));
        }
        Ok(d)
    }
}
fn effective_limits(q: &Limits) -> Limits {
    let h = Limits::default();
    Limits {
        input_bytes: q.input_bytes.min(h.input_bytes),
        resident_bytes: q.resident_bytes.min(h.resident_bytes),
        side: q.side.min(h.side),
        pixels: q.pixels.min(h.pixels),
        drawables: q.drawables.min(h.drawables),
        properties: q.properties.min(h.properties),
        string_bytes: q.string_bytes.min(h.string_bytes),
        profile_bytes: q.profile_bytes.min(h.profile_bytes),
        property_bytes: q.property_bytes.min(h.property_bytes),
        metadata_bytes: q.metadata_bytes.min(h.metadata_bytes),
        visits: q.visits.min(h.visits),
        group_depth: q.group_depth.min(h.group_depth),
    }
}
pub(crate) fn decode<'o>(bytes: &[u8], options: &'o DecodeOptions<'o>, budget: &'o Budget<'o>, input_already_charged: bool) -> Result<XcfFile, XcfError> {
    decode_input(Input::Bytes(bytes), options, budget, input_already_charged, &mut CollectingPlaneSink::default())
}
pub(crate) fn decode_source<'o, S: PlaneSink>(
    source: &mut dyn Source,
    options: &'o DecodeOptions<'o>,
    budget: &'o Budget<'o>,
    sink: &mut S,
) -> Result<XcfFile<S::Plane>, XcfError> {
    decode_input(Input::Source(source), options, budget, true, sink)
}
fn decode_input<'o, S: PlaneSink>(
    input: Input<'_>,
    options: &'o DecodeOptions<'o>,
    budget: &'o Budget<'o>,
    input_already_charged: bool,
    sink: &mut S,
) -> Result<XcfFile<S::Plane>, XcfError> {
    decode_input_mode(input,options,budget,input_already_charged,sink,false)
}
pub(crate) fn decode_source_metadata<'o,S:PlaneSink>(source:&mut dyn Source,options:&'o DecodeOptions<'o>,budget:&'o Budget<'o>,sink:&mut S)->Result<XcfFile<S::Plane>,XcfError>{
    decode_input_mode(Input::Source(source),options,budget,true,sink,true)
}
fn decode_input_mode<'o,S:PlaneSink>(input:Input<'_>,options:&'o DecodeOptions<'o>,budget:&'o Budget<'o>,input_already_charged:bool,sink:&mut S,metadata_only:bool)->Result<XcfFile<S::Plane>,XcfError>{
    let limits = effective_limits(&options.limits);
    let length = input.len();
    if length > limits.input_bytes as u64 {
        return Err(XcfError::new(ErrorKind::Limit, 0, "encoded input budget exceeded"));
    }
    let mut r = Reader {
        input,
        pos: 0,
        version: 0,
        metadata_only,
        options,
        limits,
        charged: 0,
        resident: Some(budget.reserve(BudgetKind::Resident, 0, "XCF parser entry")?),
        resident_at_entry: budget.used(BudgetKind::Resident),
        metadata: 0,
        visits: 0,
        props: 0,
        drawable_count: 0,
        pixels: 0,
        budget: Some(budget),
        effect_depth: 0,
        text_bytes: 0,
        config_entries: 0,
        graph_nodes: 0,
        graph_edges: 0,
        effect_count: 0,
        seen: HashSet::new(),
        progress: 0.0,
    };
    let result = decode_records(&mut r, length, input_already_charged, sink);
    if result.is_err() {
        sink.abort();
    }
    result
}
fn decode_records<S: PlaneSink>(r: &mut Reader<'_, '_>, length: u64, input_already_charged: bool, sink: &mut S) -> Result<XcfFile<S::Plane>, XcfError> {
    r.poll()?;
    if !input_already_charged {
        r.charge(usize::try_from(length).map_err(|_| r.error(ErrorKind::Limit, "input address space"))?)?;
    }
    let magic = r.take(14)?;
    if magic.as_ref() == b"gimp xcf file\0" {
        r.version = 0;
    } else {
        if magic.get(..10) != Some(b"gimp xcf v") || magic.last() != Some(&0) {
            return Err(r.error(ErrorKind::Header, "invalid plain GIMP XCF header"));
        }
        let digits = magic.get(10..13).ok_or_else(|| r.error(ErrorKind::Header, "version digits"))?;
        let s = std::str::from_utf8(digits).map_err(|_| r.error(ErrorKind::Header, "invalid version digits"))?;
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(r.error(ErrorKind::Header, "invalid version digits"));
        }
        r.version = s.parse().map_err(|_| r.error(ErrorKind::Header, "invalid version"))?;
    }
    if r.version > 26 || matches!(r.version, 4..=6) {
        return Err(r.error(ErrorKind::Version, "unsupported GIMP/CinePaint/development XCF version"));
    }
    let width = r.u32()?;
    let height = r.u32()?;
    r.dimensions(width, height)?;
    r.pixels = 0;
    let model = match r.u32()? {
        0 => Model::Rgb,
        1 => Model::Gray,
        2 => Model::Indexed,
        _ => return Err(r.error(ErrorKind::Structure, "invalid base model")),
    };
    let precision = if r.version >= 7 { r.u32()? } else { 150 };
    let (sample, encoding) = match precision {
        100 => (Sample::U8, Encoding::Linear),
        150 => (Sample::U8, Encoding::NonlinearProfile),
        175 => (Sample::U8, Encoding::Perceptual),
        200 => (Sample::U16, Encoding::Linear),
        250 => (Sample::U16, Encoding::NonlinearProfile),
        275 => (Sample::U16, Encoding::Perceptual),
        300 => (Sample::U32, Encoding::Linear),
        350 => (Sample::U32, Encoding::NonlinearProfile),
        375 => (Sample::U32, Encoding::Perceptual),
        500 => (Sample::F16, Encoding::Linear),
        550 => (Sample::F16, Encoding::NonlinearProfile),
        575 => (Sample::F16, Encoding::Perceptual),
        600 => (Sample::F32, Encoding::Linear),
        650 => (Sample::F32, Encoding::NonlinearProfile),
        675 => (Sample::F32, Encoding::Perceptual),
        700 => (Sample::F64, Encoding::Linear),
        750 => (Sample::F64, Encoding::NonlinearProfile),
        775 => (Sample::F64, Encoding::Perceptual),
        _ => return Err(r.error(ErrorKind::Unsupported, format!("unsupported precision {precision}"))),
    };
    if (r.version < 12 && sample != Sample::U8) || (model == Model::Indexed && (sample != Sample::U8 || r.version == 0)) {
        return Err(r.error(ErrorKind::Unsupported, "ambiguous development sample byte order or invalid indexed precision"));
    }
    let mut file = XcfFile {
        paths: Vec::new(),
        tattoo: None,
        item_sets: Vec::new(),
        version: r.version,
        width,
        height,
        model,
        precision,
        sample,
        encoding,
        compression: Compression::Rle,
        icc: Vec::new(),
        dpi: (72.0, 72.0),
        unit: Unit::Inch,
        palette: Vec::new(),
        layers: Vec::new(),
        channels: Vec::new(),
        guides: Vec::new(),
        diagnostics: Vec::new(),
        raw_properties: Vec::new(),
        charged_bytes: 0,
    };
    crate::properties::parse(r, &mut file, None)?;
    if model == Model::Indexed && file.palette.is_empty() {
        return Err(r.error(ErrorKind::Structure, "indexed image missing palette"));
    }
    let layers = r.pointers(r.limits.drawables)?;
    let channels = r.pointers(r.limits.drawables)?;
    let paths = if r.version >= 18 { r.pointers(r.limits.drawables)? } else { Vec::new() };
    let total = layers.len() + channels.len();
    r.progress(0.05);
    for (i, p) in layers.into_iter().enumerate() {
        let d = r.drawable(p, &mut file, PlaneRole::Raster, sink)?;
        file.layers.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "layers"))?;
        file.layers.push(d);
        r.progress(0.05 + 0.9 * ((i + 1) as f32) / (total.max(1) as f32));
    }
    for p in channels {
        let d = r.drawable(p, &mut file, PlaneRole::Channel, sink)?;
        file.channels.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "channels"))?;
        file.channels.push(d);
    }
    for p in paths {
        let path = crate::paths::modern(r, p)?;
        crate::paths::append(r, &mut file, path)?;
    }
    r.poll()?;
    r.progress(1.0);
    file.charged_bytes = r.charged;
    Ok(file)
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    #[test]
    fn fallible_allocator_capacity_denial_is_an_error() {
        // Exercise the allocator denial branch without exhausting real memory. Public
        // options cannot bypass the hard budget; this test-only reader construction can.
        let options = DecodeOptions::default();
        let mut reader = Reader {
            input: Input::Bytes(&[]),
            pos: 0,
            version: 12,
            metadata_only: false,
            options: &options,
            limits: Limits { resident_bytes: usize::MAX, ..Default::default() },
            charged: 0,
            resident: None,
            resident_at_entry: 0,
            metadata: 0,
            visits: 0,
            props: 0,
            drawable_count: 0,
            pixels: 0,
            budget: None,
            effect_depth: 0,
            text_bytes: 0,
            config_entries: 0,
            graph_nodes: 0,
            graph_edges: 0,
            effect_count: 0,
            seen: HashSet::new(),
            progress: 0.0,
        };
        let error = reader.vec(usize::MAX).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Allocation);
        assert!(error.message.contains("pixel allocation"));
    }
}
