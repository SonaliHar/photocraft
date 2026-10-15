//! Single-command retained inspection bridge. No batch or generic raw Value
//! entry receives paid inspection authority from this adapter.
use super::*;

pub(super) async fn run(server: &PhotocraftMcp, params: Value, wait: bool) -> Result<CallToolResult, McpError> {
    match &*server.backend {
        Backend::Headless(_) => {
            let result = server
                .headless_op_without_sync(move |headless| {
                    if headless.has_owned_filter_inspection() {
                        let reply = headless.inspect_owned_filter_bounded(&params, wait)?;
                        // The actual engine fee remains live through payload JSON
                        // encoding and bounded SDK-envelope serialization checking.
                        // SDK-owned text after this return uses the existing bounded
                        // transport handoff; it is not a catalog-paid socket lifetime.
                        Ok(ok_json(reply.value()))
                    } else {
                        // Preserve the old MCP resident query behavior, including
                        // its job synchronization, authorization and ordinary reply.
                        headless.sync_jobs();
                        let value = headless.command_start("xcf.filter.inspect", params, wait)?;
                        Ok(ok_json(&value))
                    }
                })
                .await;
            match result {
                Some(Ok(result)) => Ok(result),
                Some(Err(error)) => to_result(Err(error)),
                None => Ok(no_backend()),
            }
        }
        Backend::Bridge(bridge) => {
            // The remote control service supplies its own retained writer and
            // authorization. No local capability, fee or fallback is invented.
            to_result(bridge.call("engine.execute", json!({"command":"xcf.filter.inspect","params":params,"wait":wait})).await)
        }
    }
}

#[cfg(test)]
#[path = "owned_filter_inspect_tests.rs"]
mod tests;
