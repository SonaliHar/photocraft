//! CLI transport for paid readonly XCF discovery/filter replies. No raw Value is detached.
use photocraft_automation::{AutomationError, Headless, budgets};
use serde_json::Value;
use std::io::Write;

pub(super) fn handles(headless: &Headless, command: &str) -> bool {
    matches!(command, "xcf.filter.inspect" | "xcf.inspect") && headless.has_owned_filter_inspection()
}

pub(super) fn write(headless: &mut Headless, command: &str, params: &Value, output: &mut dyn Write) -> Result<(), AutomationError> {
    // Headless creates ONE bounded control for the actual query and CLI encoding.
    // The returned wire owns both the reply and its same-catalog encoded bytes.
    // `run` has always printed compact command/result JSON, one line per command.
    let wire = headless.inspect_owned_command_cli_bounded(command, params, true, budgets::MAX_RESPONSE_BYTES)?;
    wire.write_line_to(output).map_err(|error| AutomationError::Io(error.to_string()))?;
    // Honor the carrier's output contract: its genuine reply and byte fees stay
    // live until buffered output has also completed its explicit flush.
    output.flush().map_err(|error| AutomationError::Io(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
