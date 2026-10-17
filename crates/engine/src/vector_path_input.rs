//! Borrowed closed path envelope admission followed by fallible decoding.
use photocraft_doc::{FillRule, Knot, Path, PathOp, Subpath};
use photocraft_geom::Point;
use serde_json::{Map, Value};

const MAX_SUBPATHS: usize = 10_000;
const MAX_KNOTS: usize = 100_000;
const MAX_COORD: f64 = 1_000_000.0;
const IN_KEYS: [&str; 3] = ["in", "inCtrl", "in_ctrl"];
const OUT_KEYS: [&str; 3] = ["out", "outCtrl", "out_ctrl"];
type Result<T> = std::result::Result<T, String>;

fn record<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>> {
    let object = value.as_object().ok_or("path record must be an object")?;
    if object.len() > keys.len() || object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("unknown path record field".into());
    }
    Ok(object)
}
fn coordinate(value: &Value) -> Result<f64> {
    value.as_f64().filter(|number| number.is_finite() && number.abs() <= MAX_COORD).ok_or_else(|| "path coordinate must be finite and within +/-1000000".into())
}
fn point(value: &Value) -> Result<Point> {
    let (x, y) = match value {
        Value::Array(values) if values.len() == 2 => (values.first().ok_or("point needs x")?, values.get(1).ok_or("point needs y")?),
        Value::Object(_) => {
            let object = record(value, &["x", "y"])?;
            if object.len() != 2 {
                return Err("point requires exactly x and y".into());
            }
            (object.get("x").ok_or("point needs x")?, object.get("y").ok_or("point needs y")?)
        }
        _ => return Err("point must be exactly [x,y] or {x,y}".into()),
    };
    Ok(Point::new(coordinate(x)?, coordinate(y)?))
}
fn boolean(object: &Map<String, Value>, key: &str, default: bool) -> Result<bool> {
    match object.get(key) {
        None => Ok(default),
        Some(value) => value.as_bool().ok_or_else(|| "provided path boolean has wrong type".into()),
    }
}
fn operation(value: Option<&Value>) -> Result<PathOp> {
    let Some(value) = value else { return Ok(PathOp::Combine) };
    let text = value.as_str().filter(|text| text.len() <= 16).ok_or("path op must be a bounded string")?;
    for (names, op) in [
        (&["combine", "add", "union"][..], PathOp::Combine),
        (&["subtract", "minus"][..], PathOp::Subtract),
        (&["intersect"][..], PathOp::Intersect),
        (&["exclude", "xor"][..], PathOp::Exclude),
        (&["join"][..], PathOp::Join),
    ] {
        if names.iter().any(|name| text.eq_ignore_ascii_case(name)) {
            return Ok(op);
        }
    }
    Err("unknown path operation".into())
}
fn fill(value: Option<&Value>) -> Result<FillRule> {
    let Some(value) = value else { return Ok(FillRule::NonZero) };
    let text = value.as_str().filter(|text| text.len() <= 16).ok_or("fill rule must be a bounded string")?;
    if ["evenodd", "even-odd", "even_odd"].iter().any(|name| text.eq_ignore_ascii_case(name)) {
        return Ok(FillRule::EvenOdd);
    }
    if text.eq_ignore_ascii_case("nonzero") {
        return Ok(FillRule::NonZero);
    }
    Err("unknown path fill rule".into())
}
fn knot(value: &Value) -> Result<Knot> {
    if value.is_array() {
        let anchor = point(value)?;
        return Ok(Knot::corner(anchor.x, anchor.y));
    }
    let object = record(value, &["anchor", "in", "inCtrl", "in_ctrl", "out", "outCtrl", "out_ctrl", "smooth"])?;
    let anchor = point(object.get("anchor").ok_or("knot needs anchor")?)?;
    // Validate every provided alias. An invalid explicit handle must not be
    // skipped in favour of another alias, or silently replaced by its anchor.
    for key in IN_KEYS.iter().chain(&OUT_KEYS) {
        if let Some(value) = object.get(*key) {
            point(value)?;
        }
    }
    let handle = |keys: &[&str]| -> Result<Point> {
        match keys.iter().find_map(|key| object.get(*key)) {
            Some(value) => point(value),
            None => Ok(anchor),
        }
    };
    Ok(Knot { anchor, in_ctrl: handle(&IN_KEYS)?, out_ctrl: handle(&OUT_KEYS)?, smooth: boolean(object, "smooth", false)? })
}
/// Successful admission allocates nothing and never recursively visits unknown
/// values. Counts/types/shapes/coordinates are checked before owned path vectors.
pub(super) fn preflight(value: &Value) -> Result<()> {
    let object = record(value, &["subpaths", "fillRule", "fill_rule", "inverted"])?;
    let subpaths = object.get("subpaths").and_then(Value::as_array).ok_or("path needs subpaths")?;
    if subpaths.len() > MAX_SUBPATHS {
        return Err("path subpath limit".into());
    }
    for key in ["fillRule", "fill_rule"] {
        if let Some(value) = object.get(key) {
            fill(Some(value))?;
        }
    }
    boolean(object, "inverted", false)?;
    let mut count = 0usize;
    for subpath in subpaths {
        let object = record(subpath, &["closed", "op", "knots"])?;
        let knots = object.get("knots").and_then(Value::as_array).ok_or("subpath needs knots")?;
        count = count.checked_add(knots.len()).filter(|count| *count <= MAX_KNOTS).ok_or("path aggregate knot limit")?;
        boolean(object, "closed", true)?;
        operation(object.get("op"))?;
        for value in knots {
            knot(value)?;
        }
    }
    Ok(())
}
pub(super) fn decode(value: &Value) -> Result<Path> {
    preflight(value)?;
    let object = value.as_object().ok_or("path must be an object")?;
    let subpaths = object.get("subpaths").and_then(Value::as_array).ok_or("path needs subpaths")?;
    let mut output = Vec::new();
    output.try_reserve_exact(subpaths.len()).map_err(|_| "path allocation failed")?;
    for subpath in subpaths {
        let subpath = subpath.as_object().ok_or("subpath must be an object")?;
        let values = subpath.get("knots").and_then(Value::as_array).ok_or("subpath needs knots")?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(values.len()).map_err(|_| "knot allocation failed")?;
        for value in values {
            knots.push(knot(value)?);
        }
        output.push(Subpath { knots, closed: boolean(subpath, "closed", true)?, op: operation(subpath.get("op"))? });
    }
    Ok(Path { subpaths: output, fill_rule: fill(object.get("fillRule").or_else(|| object.get("fill_rule")))?, inverted: boolean(object, "inverted", false)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn compact_serde_and_handle_aliases_preserve_exact_path_semantics() {
        for (incoming, outgoing) in [("in", "out"), ("inCtrl", "outCtrl"), ("in_ctrl", "out_ctrl")] {
            let mut value = json!({"subpaths":[{"closed":false,"op":"MiNuS","knots":[{"anchor":[-1,2],"smooth":true}]}],"fill_rule":"EvenOdd","inverted":true});
            let object = value["subpaths"][0]["knots"][0].as_object_mut().unwrap();
            object.insert(incoming.into(), json!({"x":-2,"y":3}));
            object.insert(outgoing.into(), json!([4, 5]));
            let path = decode(&value).unwrap();
            let sub = path.subpaths.first().unwrap();
            let point = sub.knots.first().unwrap();
            assert_eq!(sub.op, PathOp::Subtract);
            assert!(!sub.closed);
            assert_eq!(path.fill_rule, FillRule::EvenOdd);
            assert!(path.inverted);
            assert_eq!(point.in_ctrl, Point::new(-2., 3.));
            assert_eq!(point.out_ctrl, Point::new(4., 5.));
            assert!(point.smooth);
            let serde = serde_json::to_value(&path).unwrap();
            assert_eq!(decode(&serde).unwrap(), path);
        }
        let path = decode(&json!({"subpaths":[{"knots":[[-1000000,1000000]]}]})).unwrap();
        assert_eq!(path.subpaths[0].knots[0], Knot::corner(-1000000., 1000000.));
    }
    #[test]
    fn malformed_explicit_handles_and_point_shapes_never_fall_back_to_anchor() {
        for handle in [Value::Null, json!("wrong"), json!([1, 2, 3]), json!({"x":1,"y":2,"ignored":[]}), json!([1000001, 2]), json!([1e300, 2])] {
            let value = json!({"subpaths":[{"knots":[{"anchor":[0,0],"in":[1,1],"in_ctrl":handle}]}]});
            assert!(preflight(&value).is_err());
            assert!(decode(&value).is_err());
        }
        for value in [
            json!({"subpaths":[{"knots":[[1,2,3]]}]}),
            json!({"subpaths":[{"knots":[[1,"2"]]}]}),
            json!({"subpaths":[],"inverted":"false"}),
            json!({"subpaths":[{"knots":[],"closed":1}]}),
            json!({"subpaths":[],"unread":{"nested":[]}}),
        ] {
            assert!(preflight(&value).is_err());
        }
    }
    #[test]
    fn counts_are_admitted_before_owned_vectors_and_aggregate_limit_spans_subpaths() {
        let too_many = json!({"subpaths":vec![json!({"knots":[]});MAX_SUBPATHS+1]});
        assert!(preflight(&too_many).is_err());
        let knots = vec![json!([0, 0]); MAX_KNOTS / 2 + 1];
        let too_many = json!({"subpaths":[{"knots":knots.clone()},{"knots":knots}]});
        assert!(preflight(&too_many).is_err());
        let allowed = json!({"subpaths":vec![json!({"knots":[]});MAX_SUBPATHS]});
        assert!(preflight(&allowed).is_ok());
    }
}
