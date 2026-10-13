//! Dispatch facade for compatible saved-set management, with early captured/context guards.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::Value;
pub const RENAME: &str = "xcf.itemSet.rename";
pub const DELETE: &str = "xcf.itemSet.delete";
pub const MEMBERS: &str = "xcf.itemSet.setMembers";
/// Shared dispatch calls before any floating normalization, job setup or params copy.
pub(crate) fn dispatch_preflight(session: &Session, command: &str, params: &Value, background: bool) -> Result<()> {
    if !matches!(command, RENAME | DELETE | MEMBERS) {
        return Ok(());
    }
    if background {
        return Err(EngineError::BadParams { cmd: command.into(), msg: "saved-set management requires synchronous dispatch".into() });
    }
    crate::xcf_editing::dispatch_set_preflight(session, command, params)
}
/// Retain existing public IDs/labels/wire compatibility. Root registration must
/// exclude these same three entries from relationship_specs before extending this.
pub fn specs() -> Vec<CommandSpec> {
    crate::xcf_editing::relationship_specs().into_iter().filter(|spec| matches!(spec.id, RENAME | DELETE | MEMBERS)).collect()
}
