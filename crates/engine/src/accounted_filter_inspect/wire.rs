//! Paid serialized inspection envelopes. Neither request IDs nor result trees
//! are cloned; the original inspection owner remains held through output.
use super::OwnedFilterInspectReply;
use crate::{EngineError, Result};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{QuotaKind, QuotaLease},
};
use serde::Serialize;
use serde_json::Value;
use std::io::Write;

pub struct OwnedFilterInspectWire {
    bytes: Vec<u8>,
    _reply: OwnedFilterInspectReply,
    _lease: QuotaLease,
}
impl OwnedFilterInspectWire {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// The caller retains this carrier through its subsequent flush. No token
    /// check or fallible metadata publication follows a successful write.
    pub fn write_line_to(&self, output: &mut (impl Write + ?Sized)) -> std::io::Result<()> {
        output.write_all(&self.bytes)?;
        output.write_all(b"\n")
    }
}
#[derive(Serialize)]
struct Rpc<'a> {
    id: &'a Value,
    ok: bool,
    result: &'a Value,
}
#[derive(Serialize)]
struct Cli<'a> {
    command: &'static str,
    result: &'a Value,
}
pub fn validate_rpc_id(id: &Value, control: &ReadControl<'_>) -> Result<()> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize, text: &mut usize, control: &ReadControl<'_>) -> Result<()> {
        control.check(1)?;
        *nodes = nodes.checked_add(1).ok_or(BackingError::Length)?;
        if depth > 8 || *nodes > 4096 {
            return Err(BackingError::Length.into());
        }
        let mut add = |n: usize| -> Result<()> {
            *text = text.checked_add(n).ok_or(BackingError::Length)?;
            if *text > 1 << 20 {
                return Err(BackingError::Length.into());
            }
            control.check(u64::try_from(n).map_err(|_| BackingError::Length)?.checked_mul(2).ok_or(BackingError::Length)?)?;
            Ok(())
        };
        match value {
            Value::String(value) => add(value.len())?,
            Value::Array(values) => {
                for value in values {
                    visit(value, depth + 1, nodes, text, control)?;
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    *text = text.checked_add(key.len()).ok_or(BackingError::Length)?;
                    if *text > 1 << 20 {
                        return Err(BackingError::Length.into());
                    }
                    control.check(u64::try_from(key.len()).map_err(|_| BackingError::Length)?.checked_mul(2).ok_or(BackingError::Length)?)?;
                    visit(value, depth + 1, nodes, text, control)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(id, 0, &mut 0, &mut 0, control)
}
struct Counter<'a, 'c> {
    length: usize,
    limit: usize,
    control: &'a ReadControl<'c>,
    failure: Option<BackingError>,
}
impl Write for Counter<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Err(error) = self.control.check(bytes.len() as u64 + 1) {
            self.failure = Some(error);
            return Err(std::io::Error::other("inspection wire work"));
        }
        let Some(length) = self.length.checked_add(bytes.len()).filter(|n| *n < self.limit) else {
            self.failure = Some(BackingError::Length);
            return Err(std::io::Error::other("inspection wire limit"));
        };
        self.length = length;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Buffer<'a, 'c> {
    bytes: Vec<u8>,
    control: &'a ReadControl<'c>,
    failure: Option<BackingError>,
}
impl Write for Buffer<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Err(error) = self.control.check(bytes.len() as u64 + 1) {
            self.failure = Some(error);
            return Err(std::io::Error::other("inspection wire work"));
        }
        if self.bytes.len().checked_add(bytes.len()).is_none_or(|n| n > self.bytes.capacity()) {
            self.failure = Some(BackingError::Allocation);
            return Err(std::io::Error::other("inspection wire capacity"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn serialize(value: &impl Serialize, pretty: bool, writer: &mut impl Write) -> std::result::Result<(), serde_json::Error> {
    if pretty { serde_json::to_writer_pretty(writer, value) } else { serde_json::to_writer(writer, value) }
}
fn encoded(reply: OwnedFilterInspectReply, id: Option<&Value>, pretty: bool, limit: usize, control: &ReadControl<'_>) -> Result<OwnedFilterInspectWire> {
    control.check(1)?;
    if limit == 0 {
        return Err(BackingError::Length.into());
    }
    if let Some(id) = id {
        validate_rpc_id(id, control)?;
    }
    // Both serde traversals/escaping scans happen before their Write callbacks.
    // Cover retained result metadata prospectively as well as actual output
    // bytes below; this conservative fixed multiple is not a CPU-cycle claim.
    control.check(reply.charged_index_bytes().checked_mul(8).ok_or(BackingError::Length)?)?;
    let rpc = id.map(|id| Rpc { id, ok: true, result: reply.value() });
    let cli = Cli { command: reply.command(), result: reply.value() };
    let mut count = Counter { length: 0, limit, control, failure: None };
    let counted = match &rpc {
        Some(value) => serialize(value, pretty, &mut count),
        None => serialize(&cli, pretty, &mut count),
    };
    if let Some(error) = count.failure {
        return Err(error.into());
    }
    counted.map_err(|_| EngineError::Other("inspection wire encoding failed".into()))?;
    let capacity = count.length.checked_add(64).ok_or(BackingError::Length)?;
    let header = std::mem::size_of::<OwnedFilterInspectWire>()
        .checked_add(std::mem::size_of::<Buffer<'_, '_>>())
        .and_then(|n| n.checked_add(128))
        .ok_or(BackingError::Length)?;
    let planned = capacity.checked_add(header).ok_or(BackingError::Length)?;
    let mut lease = reply.quota().reserve(QuotaKind::Resident, planned as u64).map_err(|_| BackingError::Quota)?;
    let mut buffer = Buffer { bytes: Vec::new(), control, failure: None };
    control.check(1)?;
    buffer.bytes.try_reserve_exact(count.length).map_err(|_| BackingError::Allocation)?;
    if buffer.bytes.capacity() > capacity {
        return Err(BackingError::Allocation.into());
    }
    let result = match &rpc {
        Some(value) => serialize(value, pretty, &mut buffer),
        None => serialize(&cli, pretty, &mut buffer),
    };
    if let Some(error) = buffer.failure {
        return Err(error.into());
    }
    result.map_err(|_| EngineError::Other("inspection wire encoding failed".into()))?;
    if buffer.bytes.len() != count.length {
        return Err(BackingError::Integrity.into());
    }
    let actual = buffer.bytes.capacity().checked_add(header).ok_or(BackingError::Length)?;
    lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Quota)?;
    control.check(1)?;
    Ok(OwnedFilterInspectWire { bytes: buffer.bytes, _reply: reply, _lease: lease })
}
/// Exact legacy RPC envelope, with newline included in the caller's byte limit.
pub fn encode_rpc(reply: OwnedFilterInspectReply, id: &Value, limit: usize, control: &ReadControl<'_>) -> Result<OwnedFilterInspectWire> {
    encoded(reply, Some(id), false, limit, control)
}
/// Exact existing CLI command/result record; `compact=false` preserves pretty output.
pub fn encode_cli(reply: OwnedFilterInspectReply, compact: bool, limit: usize, control: &ReadControl<'_>) -> Result<OwnedFilterInspectWire> {
    encoded(reply, None, !compact, limit, control)
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
