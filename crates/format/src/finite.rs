//! Save-time check that the manifest holds only finite floats (issue #1101).
//!
//! JSON cannot spell NaN or the infinities: `serde_json` writes them as `null`, which the
//! manifest's plain `f32`/`f64` fields refuse on load. Such a document would save (and autosave)
//! with a success report and then never open again. So the save is refused instead, naming the
//! value, by walking the manifest with a serializer that produces nothing and fails on the first
//! non-finite float.

use std::fmt;

use serde::ser::{self, Serialize};

/// Path of the first non-finite float, e.g. `document.layers[2].content.adjustment.exposure`
/// (`Other` is reserved for a `Serialize` impl failing on its own, which the derived manifest
/// never does).
#[derive(Debug)]
pub enum Problem {
    NonFinite { path: String },
    Other(String),
    Cancelled,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::NonFinite { path } => write!(f, "`{path}` is NaN or infinite"),
            Problem::Other(m) => f.write_str(m),
            Problem::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Problem {}

impl ser::Error for Problem {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Problem::Other(msg.to_string())
    }
}

/// `Ok(())` when every float in `value`'s serialized form is finite; otherwise the path of the
/// first one that is not.
#[cfg(test)]
pub fn check<T: Serialize + ?Sized>(value: &T) -> Result<(), Problem> {
    check_with_interrupt(value, photocraft_raster::Interrupt::NONE)
}

pub fn check_with_interrupt<T: Serialize + ?Sized>(value: &T, control: photocraft_raster::Interrupt<'_>) -> Result<(), Problem> {
    let visits = std::cell::Cell::new(0usize);
    let poll = Poll { control, visits: &visits };
    poll.check()?;
    value.serialize(Check { path: String::new(), poll })
}
#[derive(Clone, Copy)]
struct Poll<'a> {
    control: photocraft_raster::Interrupt<'a>,
    visits: &'a std::cell::Cell<usize>,
}
impl Poll<'_> {
    fn check(self) -> R {
        let count = self.visits.get();
        self.visits.set(count.wrapping_add(1));
        if count.is_multiple_of(4096) && self.control.cancelled() { Err(Problem::Cancelled) } else { Ok(()) }
    }
}
struct Check<'a> {
    path: String,
    poll: Poll<'a>,
}
impl<'a> Check<'a> {
    fn at(&self, seg: fmt::Arguments<'_>) -> Check<'a> {
        Check { path: format!("{}{}", self.path, seg), poll: self.poll }
    }
    fn float(self, finite: bool) -> R {
        self.poll.check()?;
        if finite { Ok(()) } else { Err(Problem::NonFinite { path: self.path.trim_start_matches('.').to_owned() }) }
    }
}

type R = Result<(), Problem>;

impl<'a> ser::Serializer for Check<'a> {
    type Ok = ();
    type Error = Problem;
    type SerializeSeq = Seq<'a>;
    type SerializeTuple = Seq<'a>;
    type SerializeTupleStruct = Seq<'a>;
    type SerializeTupleVariant = Seq<'a>;
    type SerializeMap = Map<'a>;
    type SerializeStruct = Struct<'a>;
    type SerializeStructVariant = Struct<'a>;

    fn serialize_bool(self, _: bool) -> R {
        self.poll.check()
    }
    fn serialize_i8(self, _: i8) -> R {
        self.poll.check()
    }
    fn serialize_i16(self, _: i16) -> R {
        self.poll.check()
    }
    fn serialize_i32(self, _: i32) -> R {
        self.poll.check()
    }
    fn serialize_i64(self, _: i64) -> R {
        self.poll.check()
    }
    fn serialize_i128(self, _: i128) -> R {
        self.poll.check()
    }
    fn serialize_u8(self, _: u8) -> R {
        self.poll.check()
    }
    fn serialize_u16(self, _: u16) -> R {
        self.poll.check()
    }
    fn serialize_u32(self, _: u32) -> R {
        self.poll.check()
    }
    fn serialize_u64(self, _: u64) -> R {
        self.poll.check()
    }
    fn serialize_u128(self, _: u128) -> R {
        self.poll.check()
    }
    fn serialize_f32(self, v: f32) -> R {
        self.float(v.is_finite())
    }
    fn serialize_f64(self, v: f64) -> R {
        self.float(v.is_finite())
    }
    fn serialize_char(self, _: char) -> R {
        self.poll.check()
    }
    fn serialize_str(self, _: &str) -> R {
        self.poll.check()
    }
    fn serialize_bytes(self, _: &[u8]) -> R {
        self.poll.check()
    }
    fn serialize_none(self) -> R {
        self.poll.check()
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> R {
        v.serialize(self)
    }
    fn serialize_unit(self) -> R {
        self.poll.check()
    }
    fn serialize_unit_struct(self, _: &'static str) -> R {
        self.poll.check()
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> R {
        self.poll.check()
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, v: &T) -> R {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, variant: &'static str, v: &T) -> R {
        v.serialize(self.at(format_args!(".{variant}")))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Seq<'a>, Problem> {
        self.poll.check()?;
        Ok(Seq { poll: self.poll, path: self.path, i: 0 })
    }
    fn serialize_tuple(self, _: usize) -> Result<Seq<'a>, Problem> {
        self.poll.check()?;
        Ok(Seq { poll: self.poll, path: self.path, i: 0 })
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Seq<'a>, Problem> {
        self.poll.check()?;
        Ok(Seq { poll: self.poll, path: self.path, i: 0 })
    }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, variant: &'static str, _: usize) -> Result<Seq<'a>, Problem> {
        self.poll.check()?;
        Ok(Seq { poll: self.poll, path: format!("{}.{variant}", self.path), i: 0 })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Map<'a>, Problem> {
        self.poll.check()?;
        Ok(Map { poll: self.poll, path: self.path, key: String::new() })
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Struct<'a>, Problem> {
        self.poll.check()?;
        Ok(Struct { poll: self.poll, path: self.path })
    }
    fn serialize_struct_variant(self, _: &'static str, _: u32, variant: &'static str, _: usize) -> Result<Struct<'a>, Problem> {
        self.poll.check()?;
        Ok(Struct { poll: self.poll, path: format!("{}.{variant}", self.path) })
    }
}

struct Seq<'a> {
    poll: Poll<'a>,
    path: String,
    i: usize,
}

impl Seq<'_> {
    fn next<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        let c = Check { poll: self.poll, path: format!("{}[{}]", self.path, self.i) };
        self.i += 1;
        v.serialize(c)
    }
}

impl ser::SerializeSeq for Seq<'_> {
    type Ok = ();
    type Error = Problem;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        self.next(v)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeTuple for Seq<'_> {
    type Ok = ();
    type Error = Problem;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        self.next(v)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for Seq<'_> {
    type Ok = ();
    type Error = Problem;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        self.next(v)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeTupleVariant for Seq<'_> {
    type Ok = ();
    type Error = Problem;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        self.next(v)
    }
    fn end(self) -> R {
        Ok(())
    }
}

struct Map<'a> {
    poll: Poll<'a>,
    path: String,
    key: String,
}

impl ser::SerializeMap for Map<'_> {
    type Ok = ();
    type Error = Problem;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> R {
        // Keys are checked like any value; their text (when they have one) names the entry.
        key.serialize(Check { poll: self.poll, path: format!("{}.<key>", self.path) })?;
        struct KeyWriter<'a> {
            bytes: Vec<u8>,
            poll: Poll<'a>,
        }
        impl std::io::Write for KeyWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.poll.control.cancelled() {
                    return Err(std::io::Error::other("cancelled"));
                }
                if self.bytes.len().saturating_add(bytes.len()) > 65536 {
                    return Err(std::io::Error::other("manifest key limit"));
                }
                self.bytes.try_reserve(bytes.len()).map_err(|_| std::io::Error::other("manifest key allocation"))?;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = KeyWriter { bytes: Vec::new(), poll: self.poll };
        let result = serde_json::to_writer(&mut output, &crate::convert::interruptible_json::Value { value: key });
        if self.poll.control.cancelled() {
            return Err(Problem::Cancelled);
        }
        result.map_err(|e| Problem::Other(e.to_string()))?;
        self.key = String::from_utf8(output.bytes).map_err(|e| Problem::Other(e.to_string()))?.trim_matches('"').to_owned();
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        v.serialize(Check { poll: self.poll, path: format!("{}.{}", self.path, self.key) })
    }
    fn end(self) -> R {
        Ok(())
    }
}

struct Struct<'a> {
    poll: Poll<'a>,
    path: String,
}

impl Struct<'_> {
    fn field<T: Serialize + ?Sized>(&mut self, key: &'static str, v: &T) -> R {
        v.serialize(Check { poll: self.poll, path: format!("{}.{key}", self.path) })
    }
}

impl ser::SerializeStruct for Struct<'_> {
    type Ok = ();
    type Error = Problem;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, v: &T) -> R {
        self.field(key, v)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeStructVariant for Struct<'_> {
    type Ok = ();
    type Error = Problem;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, v: &T) -> R {
        self.field(key, v)
    }
    fn end(self) -> R {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Inner {
        gamma: f32,
        offsets: Vec<f64>,
    }
    #[derive(Serialize)]
    enum Content {
        Exposure { exposure: f32 },
        Levels(f64, f64),
    }
    #[derive(Serialize)]
    struct Outer {
        dpi: f32,
        inner: Option<Inner>,
        layers: Vec<Content>,
        named: std::collections::BTreeMap<String, f32>,
    }

    fn outer() -> Outer {
        Outer {
            dpi: 72.0,
            inner: Some(Inner { gamma: 1.0, offsets: vec![0.0, 1.5] }),
            layers: vec![Content::Levels(0.0, 1.0), Content::Exposure { exposure: 0.5 }],
            named: [("blur".to_owned(), 2.0)].into_iter().collect(),
        }
    }

    fn path(v: &impl Serialize) -> String {
        match check(v) {
            Err(Problem::NonFinite { path }) => path,
            other => panic!("expected a non-finite report, got {other:?}"),
        }
    }

    #[test]
    fn finite_values_pass_and_each_position_is_named() {
        assert!(check(&outer()).is_ok());
        let mut o = outer();
        o.dpi = f32::NAN;
        assert_eq!(path(&o), "dpi");
        let mut o = outer();
        if let Some(i) = o.inner.as_mut() {
            i.offsets[1] = f64::INFINITY;
        }
        assert_eq!(path(&o), "inner.offsets[1]");
        let mut o = outer();
        o.layers[1] = Content::Exposure { exposure: f32::NEG_INFINITY };
        assert_eq!(path(&o), "layers[1].Exposure.exposure");
        let mut o = outer();
        o.layers[0] = Content::Levels(0.0, f64::NAN);
        assert_eq!(path(&o), "layers[0].Levels[1]");
        let mut o = outer();
        o.named.insert("blur".to_owned(), f32::NAN);
        assert_eq!(path(&o), "named.blur");
        assert_eq!(path(&f32::NAN), "");
    }
}

#[cfg(test)]
mod interrupt_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn long_finite_validation_polls_before_late_nonfinite() {
        let mut values = vec![0.5f64; 20000];
        values.push(f64::NAN);
        let polls = AtomicUsize::new(0);
        let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 3;
        assert!(matches!(check_with_interrupt(&values, photocraft_raster::Interrupt::cancel_only(&cancelled)), Err(Problem::Cancelled)));
        assert!(polls.load(Ordering::Relaxed) >= 4);
        assert!(matches!(check(&values),Err(Problem::NonFinite{path}) if path=="[20000]"));
    }
}
