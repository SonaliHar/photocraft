//! Lower-only bridge between legacy parser ceilings and transaction resource policy.
//! This grants no capability and performs no source I/O.
use photocraft_xcf::{BudgetKind, ErrorKind, Limits, ResourceLimits, XcfError};

#[derive(Clone, Debug)]
pub struct ImportPolicy {
    pub parser: Limits,
    pub resources: ResourceLimits,
}
fn width(name: &str) -> XcfError {
    XcfError { kind: ErrorKind::Limit, offset: 0, item: String::new(), message: format!("{name} policy exceeds parser address space") }
}
/// Derive parser defaults from an already validated selected resource profile.
/// New component/work/deadline ceilings stay in `resources`; they have no legacy field.
pub fn for_resources(base: &ResourceLimits) -> Result<ImportPolicy, XcfError> {
    let mut parser = Limits::default();
    macro_rules! lower {
        ($field:ident, $kind:ident) => {
            parser.$field = parser.$field.min(usize::try_from(base.get(BudgetKind::$kind)).map_err(|_| width(BudgetKind::$kind.name()))?);
        };
    }
    parser.input_bytes =
        parser.input_bytes.min(usize::try_from(base.get(BudgetKind::Encoded).min(base.get(BudgetKind::Logical))).map_err(|_| width("inputBytes"))?);
    lower!(resident_bytes, Resident);
    lower!(drawables, Drawables);
    lower!(properties, Properties);
    lower!(string_bytes, String);
    lower!(profile_bytes, Profile);
    lower!(property_bytes, Property);
    lower!(metadata_bytes, Metadata);
    lower!(visits, Visits);
    lower!(group_depth, GroupDepth);
    parser.group_depth = parser.group_depth.min(photocraft_doc::MAX_GROUP_DEPTH);
    parser.side = parser.side.min(u32::try_from(base.get(BudgetKind::Side)).map_err(|_| width("side"))?);
    parser.pixels = parser.pixels.min(base.get(BudgetKind::Pixels));
    from_parser_limits(parser, base)
}
/// Legacy `input_bytes` is a ceiling on both encoded source and logical XCF.
/// Every requested field must be at or below the selected base profile (including
/// caller reductions). Zero is retained as zero. Raises fail before source access.
pub fn from_parser_limits(mut parser: Limits, base: &ResourceLimits) -> Result<ImportPolicy, XcfError> {
    let overrides = [
        ("encodedBytes", u64::try_from(parser.input_bytes).map_err(|_| width("inputBytes"))?),
        ("logicalBytes", u64::try_from(parser.input_bytes).map_err(|_| width("inputBytes"))?),
        ("residentBytes", u64::try_from(parser.resident_bytes).map_err(|_| width("residentBytes"))?),
        ("side", u64::from(parser.side)),
        ("pixels", parser.pixels),
        ("drawables", u64::try_from(parser.drawables).map_err(|_| width("drawables"))?),
        ("properties", u64::try_from(parser.properties).map_err(|_| width("properties"))?),
        ("stringBytes", u64::try_from(parser.string_bytes).map_err(|_| width("stringBytes"))?),
        ("profileBytes", u64::try_from(parser.profile_bytes).map_err(|_| width("profileBytes"))?),
        ("propertyBytes", u64::try_from(parser.property_bytes).map_err(|_| width("propertyBytes"))?),
        ("metadataBytes", u64::try_from(parser.metadata_bytes).map_err(|_| width("metadataBytes"))?),
        ("visits", u64::try_from(parser.visits).map_err(|_| width("visits"))?),
        ("groupDepth", u64::try_from(parser.group_depth).map_err(|_| width("groupDepth"))?),
    ];
    let mut resources = base.lowered(&overrides)?;
    parser.group_depth = parser.group_depth.min(photocraft_doc::MAX_GROUP_DEPTH);
    resources = resources.lowered(&[("groupDepth", u64::try_from(parser.group_depth).map_err(|_| width("groupDepth"))?)])?;
    Ok(ImportPolicy { parser, resources })
}
