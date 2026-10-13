mod common;
use photocraft_xcf::*;
fn fixture(source: &str) -> Vec<u8> {
    fn string(bytes: &mut Vec<u8>, s: &str) {
        common::u32be(bytes, s.len() as u32 + 1);
        bytes.extend_from_slice(s.as_bytes());
        bytes.push(0);
    }
    let mut effect = Vec::new();
    for s in ["Own graph", "icon", "gegl:gegl", "0:0"] {
        string(&mut effect, s);
    }
    let mut argument = Vec::new();
    string(&mut argument, "string");
    common::u32be(&mut argument, 4);
    string(&mut argument, source);
    common::prop(&mut effect, 45, &argument);
    common::prop(&mut effect, 0, &[]);
    common::pointer(&mut effect, 0, 22);
    common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &common::FixtureOptions { effect_records: vec![effect], ..Default::default() })
}
#[test]
fn relative_units_quoted_unicode_and_inert_locators_retain_exact_spelling() {
    let source = "id=123 exposure exposure=-0.0 over aux=[ ref=123 gaussian-blur std-dev-x=0.1rel ] load path='https://invalid.example/Own Ω.png'";
    let file = decode(&fixture(source), &Default::default()).unwrap();
    let arg = &file.layers[0].effects[0].arguments[0];
    assert_eq!(arg.value, ArgumentValue::String(source.into()));
    let graph = arg.graph.as_ref().unwrap();
    assert_eq!((graph.nodes, graph.edges), (4, 5));
    let chain =
        graph.entries.iter().find_map(|e| if let GraphEntry::Property { value: GraphValue::Chain(chain), .. } = e { Some(chain) } else { None }).unwrap();
    assert!(chain.iter().any(|e| matches!(e,GraphEntry::Property{value:GraphValue::Literal{source,numeric_bits:None},..} if source=="0.1rel")));
    assert!(
        graph.entries.iter().any(
            |e| matches!(e,GraphEntry::Property{value:GraphValue::Literal{source,numeric_bits:Some(bits)},..} if source=="-0.0"&&*bits==(-0.0f64).to_bits())
        )
    );
    assert!(file.layers[0].unsupported.iter().any(|s| s.contains("effects")));
}
#[test]
fn quoted_delimiters_and_identifiers_do_not_hide_required_structure() {
    for source in ["load path='owned'junk", "id=a id=a exposure", "over aux=[ref=missing]"] {
        assert!(decode(&fixture(source), &Default::default()).is_err(), "{source}");
    }
    assert!(decode(&fixture("over aux=[ id=a ref=a exposure ]"), &Default::default()).is_ok());
    // Repeated ordinary values remain ordered source syntax, requiring an explicit
    // downstream policy rather than being silently deduplicated by this reader.
    let file = decode(&fixture("exposure exposure=0 exposure=1"), &Default::default()).unwrap();
    assert_eq!(file.layers[0].effects[0].arguments[0].graph.as_ref().unwrap().entries.len(), 3);
}
#[test]
fn graph_source_scanning_and_fanout_observe_cancellation_and_lowered_limits() {
    let source = format!("id=own {}", "over aux=[ref=own exposure] ".repeat(64));
    let bytes = fixture(&source);
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("graphEdges", 32)]).unwrap();
    assert_eq!(decode_with_resources(&bytes, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
    let polls = std::cell::Cell::new(0);
    let cancelled = || {
        polls.set(polls.get() + 1);
        polls.get() > 100
    };
    assert_eq!(decode(&bytes, &DecodeOptions { cancelled: Some(&cancelled), ..Default::default() }).unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(polls.get(), 101);
}
