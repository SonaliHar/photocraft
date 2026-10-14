//! Borrowed exact typed operation-schema binding, never rendering admission.
//! Schemas/defaults/validators must be supplied by an authoritative caller.
use photocraft_doc::{SampleEncoding, item_identity::ResourceId, xcf_filters::{ExactNumber, FilterArgument, FilterValue, GraphNode, GraphVersionOrigin, OperationIdentity}};
use photocraft_raster::Interrupt;
use std::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Cancelled, RaisedLimits, Limit, Allocation, InvalidSchema, Identity, UnknownProperty, PropertyOrder, MissingProperty, Value, Index, StoredColorUnsupported }
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "typed operation schema: {self:?}") }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub properties: usize,
    pub index_bytes: usize,
    pub depth: usize,
    pub values: usize,
    pub retained_bytes: usize,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self { Self { properties: 256, index_bytes: 64 << 10, depth: 32, values: 100_000, retained_bytes: 16 << 20, work_units: 1 << 24 } }
}
impl Limits {
    fn checked(self) -> Result<Self> {
        let ceiling = Self::default();
        if self.properties > ceiling.properties || self.index_bytes > ceiling.index_bytes || self.depth > ceiling.depth
            || self.values > ceiling.values || self.retained_bytes > ceiling.retained_bytes || self.work_units > ceiling.work_units { return Err(Error::RaisedLimits); }
        Ok(self)
    }
}
/// Shared cooperative budget for nested caller validators. Such validators must
/// remain bounded and allocation-free, or prepay their allocations externally.
/// This API grants no ambient/resource access and cannot police arbitrary Rust.
pub struct ValidationContext<'c, 'i> {
    control: &'c Interrupt<'i>,
    limits: Limits,
    visited: usize,
    retained: usize,
    work: u64,
}
impl<'c, 'i> ValidationContext<'c, 'i> {
    fn new(limits: Limits, control: &'c Interrupt<'i>) -> Self { Self { control, limits, visited: 0, retained: 0, work: 0 } }
    pub fn poll(&self) -> Result<()> { self.control.check().map_err(|_| Error::Cancelled) }
    pub fn interrupt(&self) -> &Interrupt<'i> { self.control }
    pub fn consume_work(&mut self, units: u64) -> Result<()> {
        self.poll()?;
        self.work = self.work.checked_add(units).filter(|n| *n <= self.limits.work_units).ok_or(Error::Limit)?;
        Ok(())
    }
    pub fn consume_values(&mut self, count: usize) -> Result<()> {
        self.visited = self.visited.checked_add(count).filter(|n| *n <= self.limits.values).ok_or(Error::Limit)?;
        self.consume_work(u64::try_from(count).map_err(|_| Error::Limit)?)
    }
    pub fn consume_retained_bytes(&mut self, bytes: usize) -> Result<()> {
        self.retained = self.retained.checked_add(bytes).filter(|n| *n <= self.limits.retained_bytes).ok_or(Error::Limit)?;
        self.consume_work(u64::try_from(bytes).map_err(|_| Error::Limit)?)
    }
    fn string(&mut self, value: &String, maximum: usize) -> Result<()> {
        if value.len() > maximum { return Err(Error::Limit); }
        self.consume_retained_bytes(value.capacity())
    }
    fn vector<T>(&mut self, value: &Vec<T>) -> Result<()> {
        self.consume_values(value.len())?;
        self.consume_retained_bytes(value.capacity().checked_mul(size_of::<T>()).ok_or(Error::Limit)?)
    }
    fn arguments(&mut self, values: &Vec<FilterArgument>, depth: usize) -> Result<()> {
        if depth > self.limits.depth { return Err(Error::Limit); }
        self.vector(values)?;
        for argument in values {
            self.string(&argument.name, 256)?;
            self.envelope(&argument.value, depth)?;
        }
        Ok(())
    }
    fn envelope(&mut self, value: &FilterValue, depth: usize) -> Result<()> {
        self.poll()?;
        if depth > self.limits.depth { return Err(Error::Limit); }
        self.consume_values(1)?;
        match value {
            FilterValue::Number(_) | FilterValue::Bool(_) | FilterValue::Resource(_) => {}
            FilterValue::String(value) => self.string(value, 64 << 10)?,
            FilterValue::Enum { domain, .. } => self.string(domain, 256)?,
            FilterValue::Color { components, .. } => self.vector(components)?,
            FilterValue::StoredColor(storage) => {
                self.string(&storage.layout, 64 << 10)?; self.vector(&storage.samples)?; self.vector(&storage.opaque_tail)?;
            }
            FilterValue::Curve { points_f64_bits, point_types, samples_f64_bits, .. } => {
                self.vector(points_f64_bits)?; self.vector(point_types)?; self.vector(samples_f64_bits)?;
            }
            FilterValue::Config(arguments) => self.arguments(arguments, depth.checked_add(1).ok_or(Error::Limit)?)?,
            FilterValue::Graph(graph) => {
                let next = depth.checked_add(1).ok_or(Error::Limit)?;
                if next > self.limits.depth || graph.nodes.len() > 4096 || graph.edges.len() > 16384 { return Err(Error::Limit); }
                self.vector(&graph.nodes)?; self.vector(&graph.edges)?;
                if let Some(pipeline) = &graph.original_pipeline { self.string(pipeline, 64 << 10)?; }
                for node in &graph.nodes {
                    self.string(&node.operation.name, 256)?; self.string(&node.operation.stored_version, 64)?; self.string(&node.operation.schema_epoch, 256)?;
                    self.arguments(&node.arguments, next)?;
                }
                for edge in &graph.edges { self.string(&edge.input_port, 256)?; self.string(&edge.output_port, 256)?; }
            }
        }
        self.poll()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Float32Policy { RawBits, Finite, Range { minimum: f32, maximum: f32 } }
#[derive(Clone, Copy, Debug)]
pub enum Float64Policy { RawBits, Finite, Range { minimum: f64, maximum: f64 } }
#[derive(Clone, Copy, Debug)]
pub enum NumberRule {
    Signed { width: u8, minimum: i64, maximum: i64 },
    Unsigned { width: u8, minimum: u64, maximum: u64 },
    Float32(Float32Policy),
    Float64(Float64Policy),
}
pub type NestedValidator<'a> = dyn Fn(&FilterValue, &mut ValidationContext<'_, '_>) -> Result<()> + 'a;
#[derive(Clone, Copy)]
pub enum ProfileRule<'a> { Absent, Exact(ResourceId), Validate(&'a NestedValidator<'a>) }
#[derive(Clone, Copy)]
pub enum ValueRule<'a> {
    Number(NumberRule),
    Bool,
    String { maximum_bytes: usize, allow_nul: bool },
    /// Values must be caller-declared, strictly sorted and unique.
    Enum { domain: &'a str, values: &'a [i32] },
    Color { components: &'a [NumberRule], encoding: SampleEncoding, profile: ProfileRule<'a> },
    Config(&'a NestedValidator<'a>),
    Graph(&'a NestedValidator<'a>),
    Resource(&'a NestedValidator<'a>),
    Curve(&'a NestedValidator<'a>),
}
#[derive(Clone, Copy)]
pub enum Presence<'a> {
    Required,
    OptionalAbsent,
    /// Borrowed fallback only; never inserted into or normalized over source values.
    OptionalDefault(&'a FilterValue),
}
#[derive(Clone, Copy)]
pub struct Property<'a> { pub name: &'a str, pub rule: ValueRule<'a>, pub presence: Presence<'a> }
pub struct Schema<'a> {
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    /// Declaration order is authoritative. Present properties must follow it;
    /// omissions are legal only under the explicit optional rule.
    pub properties: &'a [Property<'a>],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidationUsage { pub visited_values: usize, pub retained_bytes: usize, pub work_units: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaRequirements {
    /// Actual prepared index capacity plus its inline prepared-owner structure.
    pub owned_index_bytes: u64,
    /// Known borrowed schema/default spans and declared validator debits. Shared
    /// spans may be charged repeatedly; borrowed-slice backing capacity and opaque
    /// validator captures remain the enclosing owner's explicit responsibility.
    pub known_borrowed_bytes: u64,
    pub validation_work_limit: u64,
    pub validation_retained_limit: u64,
}
pub struct PreparedSchema<'a> { schema: Schema<'a>, names: Vec<usize>, limits: Limits, requirements: SchemaRequirements }

fn identity_valid(identity: &OperationIdentity) -> bool {
    !identity.name.is_empty() && identity.name.len() <= 256 && !identity.stored_version.is_empty() && identity.stored_version.len() <= 64
        && !identity.schema_epoch.is_empty() && identity.schema_epoch.len() <= 256
        && !identity.name.contains('\0') && !identity.stored_version.contains('\0') && !identity.schema_epoch.contains('\0')
}
fn lookup_work(count: usize, key_bytes: u64) -> Result<u64> {
    let comparisons = if count == 0 { 0 } else { u64::from(usize::BITS - count.leading_zeros()).checked_add(1).ok_or(Error::Limit)? };
    comparisons.checked_mul(key_bytes).and_then(|n| n.checked_add(2048)).ok_or(Error::Limit)
}
fn number_rule(rule: NumberRule) -> Result<()> {
    let valid = match rule {
        NumberRule::Signed { width, minimum, maximum } => {
            if !(1..=64).contains(&width) { false } else {
                let (low, high) = if width == 64 { (i64::MIN, i64::MAX) } else {
                    let magnitude = 1i64.checked_shl(u32::from(width - 1)).ok_or(Error::InvalidSchema)?;
                    (-magnitude, magnitude - 1)
                };
                minimum <= maximum && minimum >= low && maximum <= high
            }
        }
        NumberRule::Unsigned { width, minimum, maximum } => {
            if !(1..=64).contains(&width) { false } else {
                let high = if width == 64 { u64::MAX } else { 1u64.checked_shl(u32::from(width)).ok_or(Error::InvalidSchema)? - 1 };
                minimum <= maximum && maximum <= high
            }
        }
        NumberRule::Float32(Float32Policy::Range { minimum, maximum }) => minimum.is_finite() && maximum.is_finite() && minimum <= maximum,
        NumberRule::Float64(Float64Policy::Range { minimum, maximum }) => minimum.is_finite() && maximum.is_finite() && minimum <= maximum,
        _ => true,
    };
    if valid { Ok(()) } else { Err(Error::InvalidSchema) }
}
fn check_number(rule: NumberRule, value: &ExactNumber) -> Result<()> {
    let valid = match (rule, value) {
        (NumberRule::Signed { width, minimum, maximum }, ExactNumber::Signed { width: actual, value }) => width == *actual && (minimum..=maximum).contains(value),
        (NumberRule::Unsigned { width, minimum, maximum }, ExactNumber::Unsigned { width: actual, value }) => width == *actual && (minimum..=maximum).contains(value),
        (NumberRule::Float32(policy), ExactNumber::Float32Bits(bits)) => {
            let value = f32::from_bits(*bits);
            match policy { Float32Policy::RawBits => true, Float32Policy::Finite => value.is_finite(), Float32Policy::Range { minimum, maximum } => value.is_finite() && value >= minimum && value <= maximum }
        }
        (NumberRule::Float64(policy), ExactNumber::Float64Bits(bits)) => {
            let value = f64::from_bits(*bits);
            match policy { Float64Policy::RawBits => true, Float64Policy::Finite => value.is_finite(), Float64Policy::Range { minimum, maximum } => value.is_finite() && value >= minimum && value <= maximum }
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(Error::Value) }
}
fn prepare_rule(rule: ValueRule<'_>, context: &mut ValidationContext<'_, '_>) -> Result<()> {
    context.consume_values(1)?;
    match rule {
        ValueRule::Number(rule) => number_rule(rule)?,
        ValueRule::String { maximum_bytes, .. } if maximum_bytes > 64 << 10 => return Err(Error::InvalidSchema),
        ValueRule::Enum { domain, values } => {
            if domain.is_empty() || domain.len() > 256 || domain.contains('\0') || values.is_empty() || values.len() > 4096 { return Err(Error::InvalidSchema); }
            context.consume_retained_bytes(domain.len())?; context.consume_values(values.len())?;
            context.consume_retained_bytes(values.len().checked_mul(size_of::<i32>()).ok_or(Error::Limit)?)?;
            for pair in values.windows(2) { context.poll()?; if pair.first() >= pair.get(1) { return Err(Error::InvalidSchema); } }
        }
        ValueRule::Color { components, profile, .. } => {
            if components.is_empty() || components.len() > 5 { return Err(Error::InvalidSchema); }
            context.consume_retained_bytes(components.len().checked_mul(size_of::<NumberRule>()).ok_or(Error::Limit)?)?;
            for component in components { context.poll()?; number_rule(*component)?; }
            if matches!(profile, ProfileRule::Exact(ResourceId(0))) { return Err(Error::InvalidSchema); }
        }
        _ => {}
    }
    Ok(())
}
fn check_value(rule: ValueRule<'_>, value: &FilterValue, context: &mut ValidationContext<'_, '_>) -> Result<()> {
    context.envelope(value, 0)?;
    if matches!(value, FilterValue::StoredColor(_)) { return Err(Error::StoredColorUnsupported); }
    match (rule, value) {
        (ValueRule::Number(rule), FilterValue::Number(value)) => check_number(rule, value),
        (ValueRule::Bool, FilterValue::Bool(_)) => Ok(()),
        (ValueRule::String { maximum_bytes, allow_nul }, FilterValue::String(value)) if value.len() <= maximum_bytes && (allow_nul || !value.contains('\0')) => Ok(()),
        (ValueRule::Enum { domain: expected, values }, FilterValue::Enum { domain, value }) if domain == expected && values.binary_search(value).is_ok() => Ok(()),
        (ValueRule::Color { components: rules, encoding: expected, profile }, FilterValue::Color { components, encoding, profile: actual }) if components.len() == rules.len() && *encoding == expected => {
            for (rule, component) in rules.iter().zip(components) { context.poll()?; check_number(*rule, component)?; }
            match profile {
                ProfileRule::Absent if actual.is_none() => Ok(()),
                ProfileRule::Exact(expected) if *actual == Some(expected) => Ok(()),
                ProfileRule::Validate(validator) => { validator(value, context)?; context.poll() }
                _ => Err(Error::Value),
            }
        }
        (ValueRule::Config(validator), FilterValue::Config(_)) | (ValueRule::Graph(validator), FilterValue::Graph(_))
            | (ValueRule::Curve(validator), FilterValue::Curve { .. }) => { validator(value, context)?; context.poll() }
        (ValueRule::Resource(validator), FilterValue::Resource(id)) if id.0 != 0 => { validator(value, context)?; context.poll() }
        _ => Err(Error::Value),
    }
}
fn omitted(property: Property<'_>, context: &mut ValidationContext<'_, '_>) -> Result<()> {
    context.poll()?;
    match property.presence {
        Presence::Required => Err(Error::MissingProperty),
        Presence::OptionalAbsent => context.consume_values(1),
        Presence::OptionalDefault(default) => {
            context.consume_retained_bytes(size_of::<FilterValue>())?;
            check_value(property.rule, default, context)
        }
    }
}

impl<'a> PreparedSchema<'a> {
    /// Fallible, polled property index preparation. No source/default values are
    /// cloned, no schema/stock inventory is read, and no hidden default is chosen.
    pub fn prepare(schema: Schema<'a>, limits: Limits, control: &Interrupt<'_>) -> Result<Self> {
        let limits = limits.checked()?;
        let mut context = ValidationContext::new(limits, control);
        context.poll()?;
        if !identity_valid(schema.identity) || schema.properties.len() > limits.properties { return Err(Error::InvalidSchema); }
        context.consume_retained_bytes(size_of::<OperationIdentity>())?;
        context.consume_retained_bytes(schema.properties.len().checked_mul(size_of::<Property<'_>>()).ok_or(Error::Limit)?)?;
        context.string(&schema.identity.name, 256)?; context.string(&schema.identity.stored_version, 64)?; context.string(&schema.identity.schema_epoch, 256)?;
        for property in schema.properties {
            if property.name.is_empty() || property.name.len() > 256 || property.name.contains('\0') { return Err(Error::InvalidSchema); }
            context.consume_retained_bytes(property.name.len())?;
            prepare_rule(property.rule, &mut context)?;
            if let Presence::OptionalDefault(default) = property.presence {
                context.consume_retained_bytes(size_of::<FilterValue>())?;
                check_value(property.rule, default, &mut context)?;
            }
        }
        let requested = schema.properties.len().checked_mul(size_of::<usize>()).ok_or(Error::Limit)?;
        let envelope = if requested == 0 { 0 } else { requested.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or(Error::Limit)? };
        if envelope > limits.index_bytes { return Err(Error::Limit); }
        if context.retained.checked_add(envelope).and_then(|n| n.checked_add(size_of::<Self>())).is_none_or(|n| n > limits.retained_bytes) { return Err(Error::Limit); }
        context.poll()?;
        let mut names: Vec<usize> = Vec::new();
        names.try_reserve_exact(schema.properties.len()).map_err(|_| Error::Allocation)?;
        if names.capacity().checked_mul(size_of::<usize>()).ok_or(Error::Limit)? > envelope { return Err(Error::Limit); }
        for (index, property) in schema.properties.iter().enumerate() {
            context.consume_work((names.len() as u64).checked_add(lookup_work(names.len(), 512)?).ok_or(Error::Limit)?)?;
            let position = match names.binary_search_by(|existing| schema.properties.get(*existing).map(|p| p.name).cmp(&Some(property.name))) {
                Ok(_) => return Err(Error::InvalidSchema), Err(position) => position,
            };
            if position > names.len() { return Err(Error::Index); }
            names.insert(position, index);
        }
        context.poll()?;
        let known_borrowed_bytes = u64::try_from(context.retained).map_err(|_| Error::Limit)?;
        let owned = names.capacity().checked_mul(size_of::<usize>()).and_then(|n| n.checked_add(size_of::<Self>())).ok_or(Error::Limit)?;
        context.consume_retained_bytes(owned)?;
        let requirements = SchemaRequirements { owned_index_bytes: owned as u64, known_borrowed_bytes, validation_work_limit: limits.work_units, validation_retained_limit: limits.retained_bytes as u64 };
        context.poll()?;
        Ok(Self { schema, names, limits, requirements })
    }
    pub fn identity(&self) -> &OperationIdentity { self.schema.identity }
    pub fn version_origin(&self) -> GraphVersionOrigin { self.schema.version_origin }
    pub fn limits(&self) -> Limits { self.limits }
    /// Cached allocation-free census; not total allocator/RSS or callback capture accounting.
    pub fn requirements(&self) -> SchemaRequirements { self.requirements }
    fn property_index(&self, name: &str) -> Result<usize> {
        if name.len() > 256 { return Err(Error::UnknownProperty); }
        let index = self.names.binary_search_by(|index| self.schema.properties.get(*index).map(|p| p.name).cmp(&Some(name))).map_err(|_| Error::UnknownProperty)?;
        self.names.get(index).copied().ok_or(Error::Index)
    }
    /// Arguments retain exact declared order and exact value variants/widths.
    /// RawBits admits raw floating words only structurally; arithmetic/rendering
    /// must separately choose finite policies. No float or integer conversion occurs.
    pub fn validate_node<'s, 'v>(&'s self, node: &'v GraphNode, control: &Interrupt<'_>) -> Result<ValidatedArguments<'s, 'a, 'v>> {
        let mut context = ValidationContext::new(self.limits, control);
        context.poll()?;
        if node.id.0 == 0 || !identity_valid(&node.operation) || node.operation != *self.schema.identity || node.version_origin != self.schema.version_origin { return Err(Error::Identity); }
        if node.arguments.len() > self.limits.properties { return Err(Error::Limit); }
        context.vector(&node.arguments)?;
        context.string(&node.operation.name, 256)?; context.string(&node.operation.stored_version, 64)?; context.string(&node.operation.schema_epoch, 256)?;
        let mut next = 0usize;
        for argument in &node.arguments {
            context.string(&argument.name, 256)?;
            context.consume_work(lookup_work(self.names.len(), 512)?)?;
            let index = self.property_index(&argument.name)?;
            if index < next { return Err(Error::PropertyOrder); }
            for property in self.schema.properties.get(next..index).ok_or(Error::Index)? {
                omitted(*property, &mut context)?;
            }
            let property = self.schema.properties.get(index).ok_or(Error::Index)?;
            check_value(property.rule, &argument.value, &mut context)?;
            next = index.checked_add(1).ok_or(Error::Limit)?;
        }
        for property in self.schema.properties.get(next..).ok_or(Error::Index)? {
            omitted(*property, &mut context)?;
        }
        context.poll()?;
        let usage = ValidationUsage { visited_values: context.visited, retained_bytes: context.retained, work_units: context.work };
        Ok(ValidatedArguments { schema: self, arguments: &node.arguments, usage })
    }
    /// Returns declared/metered work and retained-input envelope after success.
    /// Callers must predebit limits before validation; errors return no partial
    /// result or credit, and unmetered validator work is never implicitly covered.
    pub fn validate_node_with_usage<'s, 'v>(&'s self, node: &'v GraphNode, control: &Interrupt<'_>) -> Result<(ValidatedArguments<'s, 'a, 'v>, ValidationUsage)> {
        let validated = self.validate_node(node, control)?;
        let usage = validated.usage();
        Ok((validated, usage))
    }
    /// Callable adapter for the existing mandatory GraphNode callback contract.
    /// This remains a structural result, not execution/source fidelity admission.
    pub fn validate_graph_node(&self, node: &GraphNode, control: &Interrupt<'_>) -> std::result::Result<(), super::graph_execute::GraphExecuteError> {
        self.validate_node(node, control).map(|_| ()).map_err(|error| match error {
            Error::Cancelled => super::graph_execute::GraphExecuteError::Cancelled,
            _ => super::graph_execute::GraphExecuteError::Operation("caller typed schema binding refused"),
        })
    }
}
pub enum ResolvedValue<'schema, 'values> { Explicit(&'values FilterValue), DeclaredDefault(&'schema FilterValue), Absent }
pub struct ValidatedArguments<'s, 'schema, 'values> { schema: &'s PreparedSchema<'schema>, arguments: &'values [FilterArgument], usage: ValidationUsage }
impl<'schema, 'values> ValidatedArguments<'_, 'schema, 'values> {
    pub fn usage(&self) -> ValidationUsage { self.usage }
    /// Resolution borrows the original value or caller-declared default, and
    /// reports absence separately. The original DTO/float words never change.
    pub fn resolve(&self, name: &str, control: &Interrupt<'_>) -> Result<ResolvedValue<'schema, 'values>> {
        control.check().map_err(|_| Error::Cancelled)?;
        let index = self.schema.property_index(name)?;
        for argument in self.arguments {
            control.check().map_err(|_| Error::Cancelled)?;
            if argument.name == name { return Ok(ResolvedValue::Explicit(&argument.value)); }
        }
        let property = self.schema.schema.properties.get(index).ok_or(Error::Index)?;
        Ok(match property.presence { Presence::OptionalDefault(value) => ResolvedValue::DeclaredDefault(value), Presence::OptionalAbsent => ResolvedValue::Absent, Presence::Required => return Err(Error::MissingProperty) })
    }
}

/// Exact identity/version-origin lookup over a caller-supplied set only. This
/// index never discovers stock operations, aliases versions or reads a registry.
pub struct PreparedSchemaIndex<'s, 'schema> {
    schemas: &'s [&'s PreparedSchema<'schema>],
    order: Vec<usize>,
    requirements: SchemaIndexRequirements,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaIndexRequirements {
    pub owned_index_bytes: u64,
    /// Included in known_borrowed_bytes; exposed separately for index-budget debit.
    pub held_schema_index_bytes: u64,
    /// Prepared-schema indexes/known spans plus the borrowed reference array.
    /// No deduplication or unknown closure/backing-allocation claim is made.
    pub known_borrowed_bytes: u64,
    /// Conservative complete exact-key lookup debit, payable before lookup.
    pub lookup_work_units: u64,
}
fn key(identity: &OperationIdentity, origin: GraphVersionOrigin) -> (&str, &str, &str, bool) {
    (&identity.name, &identity.stored_version, &identity.schema_epoch, origin.is_stored())
}
impl<'s, 'schema> PreparedSchemaIndex<'s, 'schema> {
    pub fn prepare(schemas: &'s [&'s PreparedSchema<'schema>], limits: Limits, control: &Interrupt<'_>) -> Result<Self> {
        let limits = limits.checked()?;
        let mut context = ValidationContext::new(limits, control);
        context.poll()?;
        if schemas.len() > 1024 { return Err(Error::Limit); }
        let requested = schemas.len().checked_mul(size_of::<usize>()).ok_or(Error::Limit)?;
        let envelope = if requested == 0 { 0 } else { requested.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or(Error::Limit)? };
        if envelope > limits.index_bytes { return Err(Error::Limit); }
        let mut known_borrowed_bytes = schemas.len().checked_mul(size_of::<&PreparedSchema<'_>>()).ok_or(Error::Limit)? as u64;
        let mut held_schema_index_bytes = 0u64;
        context.consume_retained_bytes(usize::try_from(known_borrowed_bytes).map_err(|_| Error::Limit)?)?;
        for schema in schemas {
            let held = schema.requirements();
            let bytes = held.owned_index_bytes.checked_add(held.known_borrowed_bytes).ok_or(Error::Limit)?;
            known_borrowed_bytes = known_borrowed_bytes.checked_add(bytes).ok_or(Error::Limit)?;
            held_schema_index_bytes = held_schema_index_bytes.checked_add(held.owned_index_bytes).ok_or(Error::Limit)?;
            context.consume_retained_bytes(usize::try_from(bytes).map_err(|_| Error::Limit)?)?;
        }
        if context.retained.checked_add(envelope).and_then(|n| n.checked_add(size_of::<Self>())).is_none_or(|n| n > limits.retained_bytes) { return Err(Error::Limit); }
        context.poll()?;
        let mut order: Vec<usize> = Vec::new();
        order.try_reserve_exact(schemas.len()).map_err(|_| Error::Allocation)?;
        if order.capacity().checked_mul(size_of::<usize>()).ok_or(Error::Limit)? > envelope { return Err(Error::Limit); }
        for (index, schema) in schemas.iter().enumerate() {
            context.consume_work((order.len() as u64).checked_add(lookup_work(order.len(), 1152)?).ok_or(Error::Limit)?)?;
            let identity = schema.identity();
            let wanted = key(identity, schema.version_origin());
            let position = match order.binary_search_by(|existing| schemas.get(*existing).map(|s| key(s.identity(), s.version_origin())).cmp(&Some(wanted))) {
                Ok(_) => return Err(Error::InvalidSchema), Err(position) => position,
            };
            if position > order.len() { return Err(Error::Index); }
            order.insert(position, index);
        }
        context.poll()?;
        let owned = order.capacity().checked_mul(size_of::<usize>()).and_then(|n| n.checked_add(size_of::<Self>())).ok_or(Error::Limit)?;
        context.consume_retained_bytes(owned)?;
        // Each bounded identity comparison reads at most 2*(256+64+256)
        // bytes; the fixed term covers initial identity validation/polling.
        let lookup_work_units = lookup_work(schemas.len(), 1152)?;
        let requirements = SchemaIndexRequirements { owned_index_bytes: owned as u64, held_schema_index_bytes, known_borrowed_bytes, lookup_work_units };
        context.poll()?;
        Ok(Self { schemas, order, requirements })
    }
    pub fn requirements(&self) -> SchemaIndexRequirements { self.requirements }
    pub fn lookup(&self, node: &GraphNode, control: &Interrupt<'_>) -> Result<&'s PreparedSchema<'schema>> {
        control.check().map_err(|_| Error::Cancelled)?;
        if node.id.0 == 0 || !identity_valid(&node.operation) { return Err(Error::Identity); }
        let wanted = key(&node.operation, node.version_origin);
        let slot = self.order.binary_search_by(|index| self.schemas.get(*index).map(|s| key(s.identity(), s.version_origin())).cmp(&Some(wanted))).map_err(|_| Error::Identity)?;
        let index = *self.order.get(slot).ok_or(Error::Index)?;
        let schema = *self.schemas.get(index).ok_or(Error::Index)?;
        control.check().map_err(|_| Error::Cancelled)?;
        Ok(schema)
    }
    pub fn validate_graph_node(&self, node: &GraphNode, control: &Interrupt<'_>) -> std::result::Result<(), super::graph_execute::GraphExecuteError> {
        let schema = self.lookup(node, control).map_err(|error| match error {
            Error::Cancelled => super::graph_execute::GraphExecuteError::Cancelled,
            _ => super::graph_execute::GraphExecuteError::Operation("caller schema identity lookup refused"),
        })?;
        schema.validate_graph_node(node, control)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::item_identity::GraphNodeId;
    fn identity() -> OperationIdentity { OperationIdentity { name: "own:typed".into(), stored_version: "own-v1".into(), schema_epoch: "caller-explicit-schema".into() } }
    fn node(identity: &OperationIdentity, arguments: Vec<FilterArgument>) -> GraphNode { GraphNode { id: GraphNodeId(1), operation: identity.clone(), version_origin: GraphVersionOrigin::Stored, arguments } }
    fn argument(name: &str, value: FilterValue) -> FilterArgument { FilterArgument { name: name.into(), value } }

    #[test]
    fn native_width_and_raw_float_bits_defaults_and_absence_are_distinct() {
        let identity = identity();
        let default = FilterValue::Bool(true);
        let properties = [
            Property { name: "code", rule: ValueRule::Number(NumberRule::Unsigned { width: 32, minimum: 0, maximum: u64::from(u32::MAX) }), presence: Presence::Required },
            Property { name: "raw", rule: ValueRule::Number(NumberRule::Float64(Float64Policy::RawBits)), presence: Presence::Required },
            Property { name: "flag", rule: ValueRule::Bool, presence: Presence::OptionalDefault(&default) },
            Property { name: "label", rule: ValueRule::String { maximum_bytes: 8, allow_nul: false }, presence: Presence::OptionalAbsent },
        ];
        let schema = PreparedSchema::prepare(Schema { identity: &identity, version_origin: GraphVersionOrigin::Stored, properties: &properties }, Limits::default(), &Interrupt::NONE).unwrap();
        for bits in [0x7ff8_0000_0000_1234u64, (-0.0f64).to_bits(), 1.0f64.to_bits() + 1] {
            let source = node(&identity, vec![argument("code", FilterValue::Number(ExactNumber::Unsigned { width: 32, value: 16_777_217 })), argument("raw", FilterValue::Number(ExactNumber::Float64Bits(bits)))]);
            let before = source.clone();
            let validated = schema.validate_node(&source, &Interrupt::NONE).unwrap();
            assert!(matches!(validated.resolve("raw", &Interrupt::NONE).unwrap(), ResolvedValue::Explicit(FilterValue::Number(ExactNumber::Float64Bits(actual))) if *actual == bits));
            assert!(matches!(validated.resolve("flag", &Interrupt::NONE).unwrap(), ResolvedValue::DeclaredDefault(FilterValue::Bool(true))));
            assert!(matches!(validated.resolve("label", &Interrupt::NONE).unwrap(), ResolvedValue::Absent));
            assert_eq!(source, before);
        }
        let source = node(&identity, vec![argument("code", FilterValue::Number(ExactNumber::Unsigned { width: 16, value: 7 })), argument("raw", FilterValue::Number(ExactNumber::Float64Bits(0)))]);
        assert!(matches!(schema.validate_node(&source, &Interrupt::NONE), Err(Error::Value)));
        let source = node(&identity, vec![argument("code", FilterValue::Number(ExactNumber::Unsigned { width: 32, value: 7 })), argument("raw", FilterValue::Number(ExactNumber::Float32Bits(0)))]);
        assert!(matches!(schema.validate_node(&source, &Interrupt::NONE), Err(Error::Value)));
    }

    #[test]
    fn declared_order_uniqueness_required_members_enum_and_finite_ranges_are_checked() {
        let identity = identity();
        let members = [1, 3];
        let properties = [
            Property { name: "mode", rule: ValueRule::Enum { domain: "own-mode", values: &members }, presence: Presence::Required },
            Property { name: "amount", rule: ValueRule::Number(NumberRule::Float32(Float32Policy::Range { minimum: 0.0, maximum: 1.0 })), presence: Presence::Required },
        ];
        let schema = PreparedSchema::prepare(Schema { identity: &identity, version_origin: GraphVersionOrigin::Stored, properties: &properties }, Limits::default(), &Interrupt::NONE).unwrap();
        let mode = || argument("mode", FilterValue::Enum { domain: "own-mode".into(), value: 3 });
        let amount = || argument("amount", FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())));
        assert!(schema.validate_node(&node(&identity, vec![mode(), amount()]), &Interrupt::NONE).is_ok());
        assert!(matches!(schema.validate_node(&node(&identity, vec![amount(), mode()]), &Interrupt::NONE), Err(Error::MissingProperty | Error::PropertyOrder)));
        assert!(matches!(schema.validate_node(&node(&identity, vec![mode(), mode(), amount()]), &Interrupt::NONE), Err(Error::PropertyOrder)));
        assert!(matches!(schema.validate_node(&node(&identity, vec![mode()]), &Interrupt::NONE), Err(Error::MissingProperty)));
        assert!(matches!(schema.validate_node(&node(&identity, vec![argument("mode", FilterValue::Enum { domain: "own-mode".into(), value: 2 }), amount()]), &Interrupt::NONE), Err(Error::Value)));
        assert!(matches!(schema.validate_node(&node(&identity, vec![mode(), argument("amount", FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())))]), &Interrupt::NONE), Err(Error::Value)));
        let duplicates = [properties[0], properties[0]];
        assert!(matches!(PreparedSchema::prepare(Schema { identity: &identity, version_origin: GraphVersionOrigin::Stored, properties: &duplicates }, Limits::default(), &Interrupt::NONE), Err(Error::InvalidSchema)));
    }

    #[test]
    fn nested_envelope_precedes_validator_and_explicit_resource_defaults_revalidate() {
        let identity = identity();
        let called = std::cell::Cell::new(0);
        let nested = |_: &FilterValue, context: &mut ValidationContext<'_, '_>| { called.set(called.get() + 1); context.consume_work(1) };
        let properties = [Property { name: "config", rule: ValueRule::Config(&nested), presence: Presence::Required }];
        let schema = PreparedSchema::prepare(Schema { identity: &identity, version_origin: GraphVersionOrigin::Stored, properties: &properties }, Limits { depth: 1, ..Default::default() }, &Interrupt::NONE).unwrap();
        let value = FilterValue::Config(vec![argument("inner", FilterValue::Config(vec![argument("leaf", FilterValue::Bool(true))]))]);
        let source = node(&identity, vec![argument("config", value)]);
        assert!(matches!(schema.validate_node(&source, &Interrupt::NONE), Err(Error::Limit)));
        assert_eq!(called.get(), 0);
        let allowed = std::cell::Cell::new(true);
        let resource = |value: &FilterValue, context: &mut ValidationContext<'_, '_>| {
            context.consume_work(1)?;
            if allowed.get() && matches!(value, FilterValue::Resource(ResourceId(7))) { Ok(()) } else { Err(Error::Value) }
        };
        let default = FilterValue::Resource(ResourceId(7));
        let properties = [Property { name: "resource", rule: ValueRule::Resource(&resource), presence: Presence::OptionalDefault(&default) }];
        let schema = PreparedSchema::prepare(Schema { identity: &identity, version_origin: GraphVersionOrigin::Stored, properties: &properties }, Limits::default(), &Interrupt::NONE).unwrap();
        let source = node(&identity, Vec::new());
        assert!(schema.validate_node(&source, &Interrupt::NONE).is_ok());
        allowed.set(false);
        assert!(matches!(schema.validate_node(&source, &Interrupt::NONE), Err(Error::Value)));
        assert!(source.arguments.is_empty());
    }

    #[test]
    fn schema_index_never_aliases_version_origin_and_cancellation_limits_are_typed() {
        let identity = identity();
        let schema = PreparedSchema::prepare(Schema { identity: &identity, version_origin: GraphVersionOrigin::Stored, properties: &[] }, Limits::default(), &Interrupt::NONE).unwrap();
        let schemas = [&schema];
        let index = PreparedSchemaIndex::prepare(&schemas, Limits::default(), &Interrupt::NONE).unwrap();
        let mut source = node(&identity, Vec::new());
        assert!(index.lookup(&source, &Interrupt::NONE).is_ok());
        source.version_origin = GraphVersionOrigin::SchemaResolved;
        assert!(matches!(index.lookup(&source, &Interrupt::NONE), Err(Error::Identity)));
        assert!(matches!(index.lookup(&source, &Interrupt::cancel_only(&|| true)), Err(Error::Cancelled)));
        let duplicates = [&schema, &schema];
        assert!(matches!(PreparedSchemaIndex::prepare(&duplicates, Limits::default(), &Interrupt::NONE), Err(Error::InvalidSchema)));
        assert!(matches!(PreparedSchemaIndex::prepare(&schemas, Limits { index_bytes: 0, ..Default::default() }, &Interrupt::NONE), Err(Error::Limit)));
        assert!(matches!(PreparedSchema::prepare(Schema { identity: &identity, version_origin: GraphVersionOrigin::Stored, properties: &[] }, Limits { depth: 33, ..Default::default() }, &Interrupt::NONE), Err(Error::RaisedLimits)));
        assert!(matches!(schema.validate_graph_node(&source, &Interrupt::cancel_only(&|| true)), Err(super::super::graph_execute::GraphExecuteError::Cancelled)));
    }

    #[test]
    fn requirements_cover_held_schema_indexes_and_usage_keeps_defaults_borrowed() {
        let identity = identity();
        let default = FilterValue::String("owned-default".into());
        let properties = [Property { name: "label", rule: ValueRule::String { maximum_bytes: 32, allow_nul: false }, presence: Presence::OptionalDefault(&default) }];
        let schema = PreparedSchema::prepare(Schema { identity: &identity, version_origin: GraphVersionOrigin::Stored, properties: &properties }, Limits::default(), &Interrupt::NONE).unwrap();
        let requirements = schema.requirements();
        assert!(requirements.owned_index_bytes >= size_of::<PreparedSchema<'_>>() as u64 + size_of::<usize>() as u64);
        assert!(requirements.known_borrowed_bytes >= size_of::<Property<'_>>() as u64 + size_of::<FilterValue>() as u64 + 13);
        assert_eq!(requirements.validation_work_limit, schema.limits().work_units);
        let schemas = [&schema];
        let index = PreparedSchemaIndex::prepare(&schemas, Limits::default(), &Interrupt::NONE).unwrap();
        let held = index.requirements();
        assert_eq!(held.held_schema_index_bytes, requirements.owned_index_bytes);
        assert!(held.known_borrowed_bytes >= requirements.owned_index_bytes + requirements.known_borrowed_bytes);
        assert!(held.lookup_work_units > 0);
        let source = node(&identity, Vec::new());
        let (validated, usage) = schema.validate_node_with_usage(&source, &Interrupt::NONE).unwrap();
        assert_eq!(validated.usage(), usage);
        assert!(usage.work_units > 0 && usage.work_units <= schema.limits().work_units);
        assert!(usage.retained_bytes >= size_of::<FilterValue>() + 13);
        match validated.resolve("label", &Interrupt::NONE).unwrap() {
            ResolvedValue::DeclaredDefault(value) => assert!(std::ptr::eq(value, &default)),
            _ => panic!("caller default must remain a separately borrowed fallback"),
        }
        assert!(source.arguments.is_empty());
    }
}
