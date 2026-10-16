//! Fresh-mapper source-property association only, never CURRENT provenance recovery.
use super::{E, PlanBudget, charge, length, scope};
use photocraft_doc::{Document, LayerId, item_identity::ItemRef, xcf_filters::FilterStack, xcf_provenance::OpaqueRecord};
use photocraft_raster::Interrupt;
const RECORD_CAP: usize = 4096; // Existing filtered-flat L1 class limit, not generic XCF.
struct SeenInterval {
    start: u64,
    end: u64,
}
struct Cursor<'a> {
    id: LayerId,
    stack: Option<&'a FilterStack>,
    started: bool,
    groups: usize,
    next_order: u32,
    mode: Option<u32>,
    opacity: Option<u32>,
    legacy: bool,
    last_end: Option<u64>,
    closed: bool,
}
fn number(raw: &[u8]) -> Result<u64, E> {
    if raw.is_empty() || (raw.len() > 1 && raw.first() == Some(&b'0')) {
        return Err(scope());
    }
    let mut n = 0u64;
    for c in raw {
        if !c.is_ascii_digit() {
            return Err(scope());
        }
        n = n.checked_mul(10).and_then(|n| n.checked_add(u64::from(*c - b'0'))).ok_or_else(length)?;
    }
    Ok(n)
}
fn identity(text: &str) -> Result<(u32, u64, u32), E> {
    if text.len() > 42 || !text.is_ascii() {
        return Err(scope());
    }
    let bytes = text.as_bytes();
    let at = bytes.iter().position(|c| *c == b'@').ok_or_else(scope)?;
    let colon = bytes.iter().position(|c| *c == b':').ok_or_else(scope)?;
    if at >= colon {
        return Err(scope());
    }
    let tag = u32::try_from(number(bytes.get(..at).ok_or_else(scope)?)?).map_err(|_| length())?;
    let offset = number(bytes.get(at + 1..colon).ok_or_else(scope)?)?;
    let declared = u32::try_from(number(bytes.get(colon + 1..).ok_or_else(scope)?)?).map_err(|_| length())?;
    Ok((tag, offset, declared))
}
fn word<B: PlanBudget>(raw: &OpaqueRecord, declared: u32, i: &Interrupt<'_>, b: &mut B) -> Result<u32, E> {
    if declared != 4 || raw.bytes.len() != 4 || raw.blake3.len() != 64 {
        return Err(scope());
    }
    charge(b, i, (std::mem::size_of::<blake3::Hasher>() as u64).checked_add(4 + 256).ok_or_else(length)?)?;
    let mut hash = blake3::Hasher::new();
    hash.update(&raw.bytes);
    let digest = hash.finalize();
    let hex = digest.to_hex();
    if hex.as_bytes() != raw.blake3.as_bytes() {
        return Err(scope());
    }
    let bytes: [u8; 4] = raw.bytes.as_slice().try_into().map_err(|_| length())?;
    Ok(u32::from_be_bytes(bytes))
}
fn finish<B: PlanBudget>(c: &mut Cursor<'_>, nonunit: &mut bool, i: &Interrupt<'_>, b: &mut B) -> Result<(), E> {
    charge(b, i, 128)?;
    if !c.started {
        return Err(scope());
    }
    if c.groups == 0 {
        if c.mode != Some(28) {
            return Err(scope());
        }
    } else {
        if c.mode != Some(62) || c.legacy {
            return Err(scope());
        }
        let node = c.stack.and_then(|s| s.nodes.get(c.groups - 1)).ok_or_else(scope)?;
        let value = f32::from_bits(c.opacity.ok_or_else(scope)?);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) || f64::from(value).to_bits() != node.opacity_f64_bits {
            return Err(scope());
        }
        *nonunit |= node.opacity_f64_bits != 1f64.to_bits();
    }
    c.groups = c.groups.checked_add(1).ok_or_else(length)?;
    Ok(())
}
pub(super) fn verify<B: PlanBudget>(doc: &Document, i: &Interrupt<'_>, b: &mut B) -> Result<(), E> {
    let state = doc.xcf_state.as_ref().ok_or_else(scope)?;
    let count = doc.layers.len();
    if !(1..=4096).contains(&count) || state.opaque.len() > RECORD_CAP || state.filter_stacks.len() > count || state.filter_masks.len() > 4096 {
        return Err(scope());
    }
    let scratch = (std::mem::size_of::<blake3::Hasher>() as u64).checked_add(1024).ok_or_else(length)?;
    let bytes = count
        .checked_mul(2)
        .and_then(|n| n.checked_mul(std::mem::size_of::<Cursor<'_>>()))
        .and_then(|n| n.checked_add(std::mem::size_of::<Vec<Cursor<'_>>>()))
        .and_then(|n| n.checked_add(std::mem::size_of::<B::Lease>()))
        .and_then(|n| {
            state.opaque.len().checked_mul(2).and_then(|count| count.checked_mul(std::mem::size_of::<SeenInterval>())).and_then(|extra| n.checked_add(extra))
        })
        .and_then(|n| n.checked_add(std::mem::size_of::<Vec<SeenInterval>>()))
        .ok_or_else(length)?;
    let fee = b.reserve(u64::try_from(bytes).map_err(|_| length())?.checked_add(scratch).and_then(|n| n.checked_add(128)).ok_or_else(length)?)?;
    let mut cursors: Vec<Cursor<'_>> = Vec::new();
    cursors.try_reserve_exact(count).map_err(|_| E::Allocation)?;
    if cursors.capacity() > count.checked_mul(2).ok_or_else(length)? {
        return Err(E::Allocation);
    }
    let mut intervals: Vec<SeenInterval> = Vec::new();
    intervals.try_reserve_exact(state.opaque.len()).map_err(|_| E::Allocation)?;
    if intervals.capacity() > state.opaque.len().checked_mul(2).ok_or_else(length)? {
        return Err(E::Allocation);
    }
    for layer in &doc.layers {
        for c in &cursors {
            charge(b, i, 1)?;
            if c.id == layer.id {
                return Err(scope());
            }
        }
        cursors.push(Cursor {
            id: layer.id,
            stack: None,
            started: false,
            groups: 0,
            next_order: 0,
            mode: None,
            opacity: None,
            legacy: false,
            last_end: None,
            closed: false,
        });
    }
    for stack in &state.filter_stacks {
        let ItemRef::Layer(id) = stack.owner else {
            return Err(scope());
        };
        let mut found = None;
        for (cn, c) in cursors.iter().enumerate() {
            charge(b, i, 1)?;
            if c.id == id {
                found = Some(cn);
                break;
            }
        }
        let c = cursors.get_mut(found.ok_or_else(scope)?).ok_or_else(scope)?;
        if c.stack.is_some() {
            return Err(scope());
        }
        c.stack = Some(stack);
    }
    let mut total = 0usize;
    let mut nonunit = false;
    let mut active_layer = None;
    for raw in &state.opaque {
        charge(b, i, 1)?;
        if raw.bytes.len() > 64 << 20 {
            return Err(scope());
        }
        total = total.checked_add(raw.bytes.len()).ok_or_else(length)?;
        if total > 256 << 20 {
            return Err(scope());
        }
        if raw.namespace.len() > 1 << 20 {
            return Err(scope());
        }
        charge(b, i, raw.namespace.len() as u64 + 1)?;
        if raw.namespace != "gimp/xcf/source-property" {
            continue;
        }
        let id = match raw.owner {
            Some(ItemRef::Layer(id)) => id,
            None => continue,
            Some(ItemRef::Mask(id)) => {
                let mut found = false;
                for m in &state.filter_masks {
                    charge(b, i, 1)?;
                    if m.id == id {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(scope());
                }
                continue;
            }
            _ => return Err(scope()),
        };
        if raw.source_version != 22 || raw.flags != 0 || raw.rendering_required || raw.identity.len() > 42 {
            return Err(scope());
        }
        charge(b, i, raw.identity.len() as u64 + 128)?;
        let (tag, offset, declared) = identity(&raw.identity)?;
        if usize::try_from(declared).map_err(|_| length())? != raw.bytes.len() {
            return Err(scope());
        }
        let end = offset.checked_add(u64::from(declared)).ok_or_else(length)?;
        for previous in &intervals {
            charge(b, i, 1)?;
            if previous.start == offset || (offset < previous.end && previous.start < end) {
                return Err(scope());
            }
        }
        intervals.push(SeenInterval { start: offset, end });
        let mut found = None;
        for (cn, c) in cursors.iter().enumerate() {
            charge(b, i, 1)?;
            if c.id == id {
                found = Some(cn);
                break;
            }
        }
        if active_layer != Some(id) {
            if let Some(previous) = active_layer {
                for c in &mut cursors {
                    charge(b, i, 1)?;
                    if c.id == previous {
                        c.closed = true;
                        break;
                    }
                }
            }
            active_layer = Some(id);
        }
        let c = cursors.get_mut(found.ok_or_else(scope)?).ok_or_else(scope)?;
        if c.closed {
            return Err(scope());
        }
        if raw.order == 0 {
            if c.started {
                finish(c, &mut nonunit, i, b)?;
            }
            c.started = true;
            c.next_order = 0;
            c.mode = None;
            c.opacity = None;
            c.legacy = false;
            c.last_end = None;
        }
        if !c.started || raw.order != c.next_order || c.last_end.is_some_and(|end| offset < end) {
            return Err(scope());
        }
        c.next_order = c.next_order.checked_add(1).ok_or_else(length)?;
        c.last_end = Some(offset.checked_add(u64::from(declared)).ok_or_else(length)?);
        if tag == 7 {
            if c.mode.is_some() {
                return Err(scope());
            }
            c.mode = Some(word(raw, declared, i, b)?);
        }
        if c.groups != 0 && tag == 33 {
            if c.opacity.is_some() {
                return Err(scope());
            }
            c.opacity = Some(word(raw, declared, i, b)?);
        }
        if c.groups != 0 && tag == 6 {
            c.legacy = true;
        }
    }
    for c in &mut cursors {
        finish(c, &mut nonunit, i, b)?;
        if c.groups != 1 + c.stack.map_or(0, |s| s.nodes.len()) {
            return Err(scope());
        }
    }
    if !nonunit {
        return Err(scope());
    }
    charge(b, i, 0)?;
    drop(cursors);
    drop(intervals);
    drop(fee);
    Ok(())
}
