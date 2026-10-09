use crate::*;
use std::collections::HashSet;
pub(crate) struct Reader<'a, 'o> {
    pub bytes: &'a [u8],
    pub pos: usize,
    pub version: u32,
    pub options: &'o DecodeOptions<'o>,
    pub limits: Limits,
    pub charged: usize,
    pub metadata: usize,
    pub visits: usize,
    pub props: usize,
    pub drawable_count: usize,
    pub pixels: u64,
    seen: HashSet<usize>,
    progress: f32,
}
impl<'a, 'o> Reader<'a, 'o> {
    pub fn error(&self, kind: ErrorKind, message: impl Into<String>) -> XcfError {
        XcfError::new(kind, self.pos, message)
    }
    pub fn poll(&mut self) -> Result<(), XcfError> {
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
        self.charged = self.charged.checked_add(n).ok_or_else(|| self.error(ErrorKind::Limit, "resident byte overflow"))?;
        if self.charged > self.limits.resident_bytes {
            return Err(self.error(ErrorKind::Limit, "resident memory budget exceeded"));
        }
        Ok(())
    }
    pub fn vec(&mut self, n: usize) -> Result<Vec<u8>, XcfError> {
        self.charge(n)?;
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| self.error(ErrorKind::Allocation, "pixel allocation"))?;
        v.resize(n, 0);
        Ok(v)
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], XcfError> {
        let end = self.pos.checked_add(n).ok_or_else(|| self.error(ErrorKind::Structure, "offset overflow"))?;
        let data = self.bytes.get(self.pos..end).ok_or_else(|| self.error(ErrorKind::Truncated, "truncated record"))?;
        self.pos = end;
        Ok(data)
    }
    pub fn u32(&mut self) -> Result<u32, XcfError> {
        let b = self.take(4)?;
        let a: [u8; 4] = b.try_into().map_err(|_| self.error(ErrorKind::Truncated, "u32"))?;
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
    pub fn pointer(&mut self) -> Result<usize, XcfError> {
        let p = self.raw_pointer()?;
        let p = usize::try_from(p).map_err(|_| self.error(ErrorKind::Structure, "pointer exceeds address space"))?;
        if p != 0 && (p < 14 || p >= self.bytes.len()) {
            return Err(self.error(ErrorKind::Structure, "pointer outside input"));
        }
        Ok(p)
    }
    fn raw_pointer(&mut self) -> Result<u64, XcfError> {
        Ok(if self.version >= 11 {
            let b = self.take(8)?;
            let a: [u8; 8] = b.try_into().map_err(|_| self.error(ErrorKind::Truncated, "pointer"))?;
            u64::from_be_bytes(a)
        } else {
            u64::from(self.u32()?)
        })
    }
    pub fn hierarchy_level(&mut self) -> Result<Option<usize>, XcfError> {
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
    pub fn enter(&mut self, p: usize) -> Result<(), XcfError> {
        self.poll()?;
        if p == 0 || p >= self.bytes.len() {
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
    pub fn pointers(&mut self, max: usize) -> Result<Vec<usize>, XcfError> {
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
            self.charge(std::mem::size_of::<usize>())?;
            v.try_reserve_exact(1).map_err(|_| self.error(ErrorKind::Allocation, "pointer list"))?;
            v.push(p);
        }
        Ok(v)
    }
    pub fn drawable(&mut self, p: usize, file: &mut XcfFile, channel: bool) -> Result<Drawable, XcfError> {
        self.drawable_inner(p, file, channel).map_err(|mut error| {
            if error.item.is_empty() {
                error.item = format!(" in drawable at {p}");
            }
            error
        })
    }
    fn drawable_inner(&mut self, p: usize, file: &mut XcfFile, channel: bool) -> Result<Drawable, XcfError> {
        self.enter(p)?;
        self.drawable_count += 1;
        if self.drawable_count > self.limits.drawables {
            return Err(self.error(ErrorKind::Limit, "drawable count limit"));
        }
        self.charge(std::mem::size_of::<Drawable>())?;
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
        let mut d = Drawable { name, width: w, height: h, model, alpha, ..Drawable::default() };
        crate::properties::parse(self, file, Some(&mut d)).map_err(|mut e| {
            e.item = format!(" in item '{}'", d.name.chars().take(80).collect::<String>());
            e
        })?;
        let hierarchy = self.pointer()?;
        let mask = if channel { 0 } else { self.pointer()? };
        if !channel && self.version >= 20 {
            let effects = self.pointers(self.limits.drawables)?;
            if !effects.is_empty() {
                d.unsupported.push("GEGL layer effects".into());
            }
        }
        if hierarchy != 0 && !d.group {
            d.plane =
                Some(crate::pixels::plane(self, hierarchy, w, h, channels, if model == Model::Indexed { Sample::U8 } else { file.sample }, file.compression)?);
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
        if (channel || alpha)
            && let Some(plane) = &d.plane
            && plane.sample == Sample::F32
        {
            let stride = plane.channels * 4;
            let coverage = if channel { 0 } else { (plane.channels - 1) * 4 };
            for (i, pixel) in plane.bytes.chunks_exact(stride).enumerate() {
                if i.is_multiple_of(4096) {
                    self.poll()?;
                }
                let bytes = pixel.get(coverage..coverage + 4).ok_or_else(|| self.error(ErrorKind::Structure, "coverage bounds"))?;
                let a: [u8; 4] = bytes.try_into().map_err(|_| self.error(ErrorKind::Structure, "coverage sample"))?;
                if !(0.0..=1.0).contains(&f32::from_ne_bytes(a)) {
                    return Err(
                        self.error(ErrorKind::Unsupported, format!("{}: alpha/mask coverage outside 0..1", d.name.chars().take(80).collect::<String>()))
                    );
                }
            }
        }
        if mask != 0 {
            self.charge(std::mem::size_of::<Drawable>())?;
            let m = self.drawable(mask, file, true)?;
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
pub(crate) fn decode(bytes: &[u8], options: &DecodeOptions<'_>) -> Result<XcfFile, XcfError> {
    let limits = effective_limits(&options.limits);
    if bytes.len() > limits.input_bytes {
        return Err(XcfError::new(ErrorKind::Limit, 0, "encoded input budget exceeded"));
    }
    let mut r = Reader {
        bytes,
        pos: 0,
        version: 0,
        options,
        limits,
        charged: 0,
        metadata: 0,
        visits: 0,
        props: 0,
        drawable_count: 0,
        pixels: 0,
        seen: HashSet::new(),
        progress: 0.0,
    };
    r.poll()?;
    r.charge(bytes.len())?;
    let magic = r.take(14)?;
    if magic == b"gimp xcf file\0" {
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
        500 => (Sample::F16, Encoding::Linear),
        550 => (Sample::F16, Encoding::NonlinearProfile),
        575 => (Sample::F16, Encoding::Perceptual),
        600 => (Sample::F32, Encoding::Linear),
        650 => (Sample::F32, Encoding::NonlinearProfile),
        675 => (Sample::F32, Encoding::Perceptual),
        _ => return Err(r.error(ErrorKind::Unsupported, format!("unsupported precision {precision} (U32/F64 are not imported)"))),
    };
    if (r.version < 12 && sample != Sample::U8) || (model == Model::Indexed && (sample != Sample::U8 || r.version == 0)) {
        return Err(r.error(ErrorKind::Unsupported, "ambiguous development sample byte order or invalid indexed precision"));
    }
    let mut file = XcfFile {
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
        palette: Vec::new(),
        layers: Vec::new(),
        channels: Vec::new(),
        guides: Vec::new(),
        diagnostics: Vec::new(),
        charged_bytes: 0,
    };
    crate::properties::parse(&mut r, &mut file, None)?;
    if model == Model::Indexed && file.palette.is_empty() {
        return Err(r.error(ErrorKind::Structure, "indexed image missing palette"));
    }
    let layers = r.pointers(r.limits.drawables)?;
    let channels = r.pointers(r.limits.drawables)?;
    if r.version >= 18 {
        let paths = r.pointers(r.limits.drawables)?;
        if !paths.is_empty() {
            file.diagnostics.push("GIMP paths omitted; path editing is unavailable".into());
        }
    }
    let total = layers.len() + channels.len();
    r.progress(0.05);
    for (i, p) in layers.into_iter().enumerate() {
        let d = r.drawable(p, &mut file, false)?;
        file.layers.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "layers"))?;
        file.layers.push(d);
        r.progress(0.05 + 0.9 * ((i + 1) as f32) / (total.max(1) as f32));
    }
    for p in channels {
        let d = r.drawable(p, &mut file, true)?;
        file.channels.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "channels"))?;
        file.channels.push(d);
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
            bytes: &[],
            pos: 0,
            version: 12,
            options: &options,
            limits: Limits { resident_bytes: usize::MAX, ..Default::default() },
            charged: 0,
            metadata: 0,
            visits: 0,
            props: 0,
            drawable_count: 0,
            pixels: 0,
            seen: HashSet::new(),
            progress: 0.0,
        };
        let error = reader.vec(usize::MAX).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Allocation);
        assert!(error.message.contains("pixel allocation"));
    }
}
