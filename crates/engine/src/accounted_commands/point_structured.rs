//! Closed borrowed configured Point wire and genuinely charged temporary DTOs.
//! No serde tree deserialization or arbitrary recursive configuration is used.
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument};
const CHANNELS: [&str; 5] = ["value", "red", "green", "blue", "alpha"];
const LEVELS: [&str; 5] = ["low-input", "high-input", "gamma", "low-output", "high-output"];
const RANGES: [&str; 3] = ["shadows", "midtones", "highlights"];
const AXES: [&str; 3] = ["cyan-red", "magenta-green", "yellow-blue"];
#[derive(Clone, Copy)]
pub(super) struct Plan {
    pub filter: FilterId,
    pub enabled: Option<bool>,
    pub depth: usize,
    bytes: u64,
}
impl Plan {
    /// Conservative known helper schedule for repeated fixed-hex parsing,
    /// record checks and typed copies. Actual provider/model work is separate.
    pub(super) fn helper_work(self) -> Result<u64> {
        self.bytes.checked_mul(8).map(|n| n.max(32768)).ok_or_else(|| BackingError::Length.into())
    }
}
enum ScanError {
    Invalid,
    Engine(EngineError),
}
type WireResult<T> = std::result::Result<T, ScanError>;
impl From<BackingError> for ScanError {
    fn from(error: BackingError) -> Self {
        if error == BackingError::Unsupported { Self::Invalid } else { Self::Engine(error.into()) }
    }
}
fn fail() -> ScanError {
    ScanError::Invalid
}
fn object<'a>(value: &'a Value, keys: &[&str]) -> WireResult<&'a serde_json::Map<String, Value>> {
    let v = value.as_object().ok_or_else(fail)?;
    if v.len() != keys.len() || keys.iter().any(|key| !v.contains_key(*key)) {
        return Err(fail());
    }
    Ok(v)
}
fn hex(value: &Value, c: &ReadControl<'_>) -> WireResult<u64> {
    c.check(32)?;
    let text = value.as_str().filter(|v| v.len() == 18 && v.starts_with("0x")).ok_or_else(fail)?;
    let digits = text.get(2..).ok_or_else(fail)?;
    if !digits.bytes().all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase()) {
        return Err(fail());
    }
    let bits = u64::from_str_radix(digits, 16).map_err(|_| fail())?;
    if !f64::from_bits(bits).is_finite() {
        return Err(fail());
    }
    Ok(bits)
}
struct Scan<'a, 'b> {
    control: &'a ReadControl<'b>,
    bytes: u64,
    f64_words: u64,
    deep: bool,
    depth: usize,
}
impl Scan<'_, '_> {
    fn add(&mut self, n: usize) -> WireResult<()> {
        self.bytes = self.bytes.checked_add(n as u64).ok_or(BackingError::Length)?;
        Ok(())
    }
    fn words(&mut self, count: u64) -> WireResult<()> {
        self.f64_words = self.f64_words.checked_add(count).filter(|n| *n <= 1280).ok_or(BackingError::Length)?;
        Ok(())
    }
    fn vector<T>(&mut self, n: usize) -> WireResult<()> {
        self.add(n.checked_mul(2).and_then(|v| v.checked_mul(std::mem::size_of::<T>())).ok_or(BackingError::Length)?)
    }
    fn name(&mut self, name: &str) -> WireResult<()> {
        if name.is_empty() || name.len() > 128 {
            return Err(fail());
        }
        self.control.check(name.len() as u64 + 1)?;
        if name.as_bytes().contains(&0) {
            return Err(fail());
        }
        self.add(name.len().checked_mul(2).ok_or(BackingError::Length)?)
    }
    fn number(&mut self, value: &Value) -> WireResult<()> {
        let outer = object(value, &["Number"])?;
        let n = object(outer.get("Number").ok_or_else(fail)?, &["Float64Bits"])?;
        self.words(1)?;
        hex(n.get("Float64Bits").ok_or_else(fail)?, self.control)?;
        Ok(())
    }
    fn fields(&mut self, value: &Value, names: &[&str]) -> WireResult<()> {
        let config = object(value, &["Config"])?;
        let rows = config.get("Config").and_then(Value::as_array).filter(|v| v.len() == names.len()).ok_or_else(fail)?;
        self.vector::<FilterArgument>(rows.len())?;
        for (row, name) in rows.iter().zip(names) {
            self.control.check(1)?;
            let row = object(row, &["name", "value"])?;
            if row.get("name").and_then(Value::as_str) != Some(*name) {
                return Err(fail());
            }
            self.name(name)?;
            self.number(row.get("value").ok_or_else(fail)?)?;
        }
        Ok(())
    }
    fn curve(&mut self, value: &Value) -> WireResult<()> {
        let outer = object(value, &["Curve"])?;
        let curve = object(outer.get("Curve").ok_or_else(fail)?, &["free", "points_f64_bits", "point_types", "samples_f64_bits"])?;
        if curve.get("free").and_then(Value::as_bool) != Some(true)
            || !curve.get("points_f64_bits").and_then(Value::as_array).is_some_and(Vec::is_empty)
            || !curve.get("point_types").and_then(Value::as_array).is_some_and(Vec::is_empty)
        {
            return Err(fail());
        }
        let samples = curve.get("samples_f64_bits").and_then(Value::as_array).filter(|v| v.len() == 256).ok_or_else(fail)?;
        self.words(256)?;
        self.vector::<u64>(256)?;
        for sample in samples {
            let bits = hex(sample, self.control)?;
            if !(0.0..=1.0).contains(&f64::from_bits(bits)) {
                return Err(fail());
            }
        }
        Ok(())
    }
    fn configuration(&mut self, value: &Value) -> WireResult<()> {
        let outer = object(value, &["Config"])?;
        let rows = outer.get("Config").and_then(Value::as_array).filter(|v| v.len() == 3 || v.len() == 5).ok_or_else(fail)?;
        self.vector::<FilterArgument>(rows.len())?;
        let names: &[&str] = if rows.len() == 3 { &RANGES } else { &CHANNELS };
        let curves = rows.first().and_then(|v| v.get("value")).is_some_and(|v| v.get("Curve").is_some());
        if curves && rows.len() != 5 {
            return Err(fail());
        }
        for (row, name) in rows.iter().zip(names) {
            self.control.check(1)?;
            let row = object(row, &["name", "value"])?;
            if row.get("name").and_then(Value::as_str) != Some(*name) {
                return Err(fail());
            }
            self.name(name)?;
            let value = row.get("value").ok_or_else(fail)?;
            if curves {
                self.curve(value)?;
            } else {
                self.fields(value, if rows.len() == 3 { &AXES } else { &LEVELS })?;
            }
        }
        self.deep = true;
        self.depth = self.depth.max(if curves { 9 } else { 11 });
        Ok(())
    }
    fn value(&mut self, value: &Value) -> WireResult<()> {
        self.control.check(1)?;
        let map = value.as_object().ok_or_else(fail)?;
        if let Some((v, domain)) = super::flat_value(map) {
            if let Some(domain) = domain {
                self.add(domain.len().checked_mul(2).ok_or(BackingError::Length)?)?;
            }
            if matches!(v, PointArgumentValue::Color4F32(_)) {
                self.vector::<ExactNumber>(4)?;
                self.depth = self.depth.max(7);
            } else if matches!(v, PointArgumentValue::Signed32(_)) {
                self.depth = self.depth.max(6);
            }
            return Ok(());
        }
        if map.contains_key("Config") {
            self.configuration(value)
        } else {
            self.number(value)?;
            self.deep = true;
            self.depth = self.depth.max(5);
            Ok(())
        }
    }
}
fn checked(id: &str, params: &Value, c: &ReadControl<'_>) -> WireResult<Plan> {
    if !matches!(id, "xcf.filter.set" | "xcf.filter.enable") {
        return Err(fail());
    }
    let p = params.as_object().ok_or_else(fail)?;
    if p.len() > 4
        || p.keys()
            .any(|k| !matches!(k.as_str(), "filterId" | "enabled") && (id == "xcf.filter.enable" || !matches!(k.as_str(), "arguments" | "opacityF64Bits")))
    {
        return Err(fail());
    }
    let text = p
        .get("filterId")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 20 && !v.starts_with('0') && v.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(fail)?;
    let filter = FilterId(text.parse().map_err(|_| fail())?);
    let enabled = p.get("enabled").map(|v| v.as_bool().ok_or_else(fail)).transpose()?;
    if id == "xcf.filter.enable" && enabled.is_none() {
        return Err(fail());
    }
    if p.get("opacityF64Bits").is_some_and(|v| v.as_str() != Some("0x3ff0000000000000")) {
        return Err(fail());
    }
    let args = p.get("arguments").and_then(Value::as_array).filter(|v| !v.is_empty() && v.len() <= 10).ok_or_else(fail)?;
    let mut scan = Scan { control: c, bytes: std::mem::size_of::<Temporary>() as u64 + 128, f64_words: 0, deep: false, depth: 5 };
    scan.vector::<FilterArgument>(args.len())?;
    for (i, arg) in args.iter().enumerate() {
        let row = object(arg, &["name", "value"])?;
        let name = row.get("name").and_then(Value::as_str).ok_or_else(fail)?;
        scan.name(name)?;
        c.check((i * 128) as u64)?;
        if args.iter().take(i).any(|v| v.get("name").and_then(Value::as_str) == Some(name)) {
            return Err(fail());
        }
        let value = row.get("value").ok_or_else(fail)?;
        if value.get("Config").is_some() && name != "config" {
            return Err(fail());
        }
        scan.value(value)?;
    }
    if !scan.deep {
        return Err(fail());
    }
    Ok(Plan { filter, enabled, depth: scan.depth, bytes: scan.bytes })
}
pub(super) fn preflight(id: &str, params: &Value, c: &ReadControl<'_>) -> Result<Option<Plan>> {
    c.check(32768)?;
    match checked(id, params, c) {
        Ok(v) => Ok(Some(v)),
        Err(ScanError::Invalid) => Ok(None),
        Err(ScanError::Engine(e)) => Err(e),
    }
}
// Typed payload precedes its actual same-ledger fee on every success/error path.
pub(super) struct Temporary {
    arguments: Vec<FilterArgument>,
    _lease: QuotaLease,
}
impl Temporary {
    pub fn arguments(&self) -> &[FilterArgument] {
        &self.arguments
    }
    pub fn parse(params: &Value, operation: &str, plan: Plan, q: &BackingQuota, c: &ReadControl<'_>) -> Result<Self> {
        let mut fee = q.reserve(QuotaKind::Index, plan.bytes).map_err(|_| BackingError::Quota)?;
        c.check(plan.helper_work()?)?;
        let arguments = crate::xcf_filter_cmds::paid_point_patches(params, operation)?;
        // Reuse the L1 exhaustive controlled heap/shape census. The slice
        // counts root length; its actual spare Vec capacity is separately held.
        if arguments.capacity() > arguments.len().checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Allocation.into());
        }
        let shape = photocraft_doc::accounted_document::point_argument_shape(&arguments, c)?;
        let spare =
            arguments.capacity().checked_sub(arguments.len()).and_then(|n| n.checked_mul(std::mem::size_of::<FilterArgument>())).ok_or(BackingError::Length)?
                as u64;
        let actual =
            shape.input_index_bytes.checked_add(spare).and_then(|v| v.checked_add(std::mem::size_of::<Self>() as u64 + 128)).ok_or(BackingError::Length)?;
        if actual > plan.bytes {
            return Err(BackingError::Allocation.into());
        }
        fee.reconcile_capacity(actual).map_err(|_| BackingError::Quota)?;
        let owner = Self { arguments, _lease: fee };
        c.check(1)?;
        Ok(owner)
    }
}
