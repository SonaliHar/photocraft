//! CLI transport for the paid readonly-filter reply. No raw Value is detached.
use photocraft_automation::{AutomationError, Headless, budgets};
use serde_json::Value;
use std::io::Write;

pub(super) fn handles(headless: &Headless, command: &str) -> bool {
    command == "xcf.filter.inspect" && headless.has_owned_filter_inspection()
}

pub(super) fn write(headless: &mut Headless, params: &Value, output: &mut dyn Write) -> Result<(), AutomationError> {
    // Headless creates ONE bounded control for the actual query and CLI encoding.
    // The returned wire owns both the reply and its same-catalog encoded bytes.
    // `run` has always printed compact command/result JSON, one line per command.
    let wire = headless.inspect_owned_filter_cli_bounded(params, true, budgets::MAX_RESPONSE_BYTES)?;
    wire.write_line_to(output).map_err(|error| AutomationError::Io(error.to_string()))?;
    // Honor the carrier's output contract: its genuine reply and byte fees stay
    // live until buffered output has also completed its explicit flush.
    output.flush().map_err(|error| AutomationError::Io(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
