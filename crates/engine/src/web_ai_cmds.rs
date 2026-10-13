//! Browser futures feed the normal external-job inbox; all document edits stay local.
use crate::{EngineError, Result, Session, ai_cmds::Snapshot};
use photocraft_ai::{Control, ImageSize, Operation, browser::Gateway, gateway};
use photocraft_geom::Rect;
use serde_json::{Value, json};

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    session: &mut Session,
    snapshot: Snapshot,
    params: Value,
    command: &str,
    label: &str,
    operation: Operation,
    roi: Rect,
    canvas: Option<(ImageSize, u32, u32)>,
) -> Result<Value> {
    let endpoint = session.ai.gateway.clone().ok_or_else(|| EngineError::Other("The browser AI gateway is not configured".into()))?;
    let document = snapshot.gateway_document()?;
    let mut providers = serde_json::to_value(&session.prefs().integrations).map_err(|e| EngineError::Other(e.to_string()))?;
    photocraft_ai::secret::redact(&mut providers);
    let request = gateway::Request { id: photocraft_ai::browser::job_id(), command: command.into(), params, providers };
    let completion = session.start_external_job(command, label, Some(snapshot.document()), |session, result| {
        let id = session.retain_ai_result(result)?;
        Ok(json!({"result":id,"preview":true}))
    })?;
    let id = completion.id;
    wasm_bindgen_futures::spawn_local(async move {
        let cancel = completion.ctx.clone();
        let progress = completion.ctx.clone();
        let control = Control::connected(move || cancel.cancelled(), move |_, fraction| progress.progress(fraction, "Running AI workflow on the server"));
        let result = async {
            photocraft_ai::browser::flush_configuration().await?;
            let updates = completion.ctx.clone();
            let bytes = Gateway::new(&endpoint)?.run(&request, &document, &control, move |status| updates.progress(status.progress, &status.message)).await?;
            snapshot.finish_gateway(&bytes, operation, roi, canvas).map_err(|e| photocraft_ai::AiError::InvalidImage(e.to_string()))
        }
        .await
        .map_err(|e| if e == photocraft_ai::AiError::Cancelled { EngineError::Cancelled } else { EngineError::Other(e.to_string()) });
        completion.complete(result);
    });
    Ok(json!({"job":id.0,"pending":true}))
}
