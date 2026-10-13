#[path = "../../io/tests/common/xcf.rs"]
mod common;
use photocraft_automation::{AuthorizedWorkspace, PhotocraftMcp};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};
#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}
async fn call(c: &RunningService<RoleClient, Client>, name: &str, args: Value) -> CallToolResult {
    let mut p = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    c.call_tool(p).await.unwrap()
}
fn value(r: CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let text = r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    serde_json::from_str(&text).unwrap()
}
#[tokio::test(flavor = "multi_thread")]
async fn xcf_actual_mcp_open_migration_and_capabilities() {
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = common::image(12, 150, 0, 1, 1, &[17, 34, 51, 255]);
    std::fs::write(dir.join("project.pcraft"), &bytes).unwrap();
    std::fs::write(dir.join("half.xcf"), common::image(12, 550, 0, 1, 1, &[0x38, 0, 0x34, 0, 0x30, 0, 0x3c, 0])).unwrap();
    std::fs::write(dir.join("broken.xcf"), b"gimp xcf v012\0").unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&dir), Some(&dir)).unwrap();
    let server = PhotocraftMcp::headless_with_workspace(workspace);
    let (s, c) = tokio::io::duplex(1 << 20);
    let task = tokio::spawn(async move {
        let running = server.serve(s).await.unwrap();
        running.waiting().await.unwrap();
    });
    let client = Client.serve(c).await.unwrap();
    let opened = value(call(&client, "doc_open", json!({"path":"project.pcraft"})).await);
    assert_eq!(opened["layers"], 1);
    assert!(opened["warnings"].is_array());
    let before = value(call(&client, "doc_inspect", json!({})).await);
    assert_eq!(before["width"], 1);
    let semantics = value(call(&client, "command_run", json!({"id":"xcf.inspect","params":{"limit":1}})).await);
    assert_eq!(semantics["format"], "xcf");
    assert_eq!(semantics["items"]["entries"][0]["item"]["kind"], "layer");
    assert!(semantics["items"]["entries"][0]["item"]["id"].is_string());
    assert_eq!(semantics["resolution"]["xDpi"], semantics["resolution"]["yDpi"]);
    for params in [json!({"limit":129}), json!({"readLocator":true}), json!({"offset":"1"})] {
        assert_eq!(call(&client, "command_run", json!({"id":"xcf.inspect","params":params})).await.is_error, Some(true));
    }
    assert_eq!(value(call(&client, "doc_inspect", json!({})).await), before);
    let failed = call(&client, "doc_open", json!({"path":"broken.xcf"})).await;
    assert_eq!(failed.is_error, Some(true));
    let details = failed.structured_content.as_ref().unwrap();
    assert_eq!(details["errorDetails"]["sourceFormat"], "xcf");
    assert_eq!(details["errorDetails"]["diagnostics"][0]["code"], "truncated_xcf");
    assert_eq!(value(call(&client, "doc_inspect", json!({})).await)["name"], before["name"]);
    for args in [json!({}), json!({"path":"project.pcraft"}), json!({"path":"new.XCF","format":"pcraft"})] {
        assert_eq!(call(&client, "doc_save", args).await.is_error, Some(true));
    }
    value(call(&client, "doc_save", json!({"path":"edited.pcraft"})).await);
    value(call(&client, "doc_save", json!({})).await);
    assert_eq!(call(&client, "doc_save", json!({"path":"project.pcraft"})).await.is_error, Some(true));
    for name in ["../project.pcraft", "/project.pcraft", "C:/project.pcraft"] {
        assert_eq!(call(&client, "doc_open", json!({"path":name})).await.is_error, Some(true));
    }
    assert_eq!(call(&client, "command_run", json!({"id":"file.open","params":{"path":"project.pcraft"}})).await.is_error, Some(true));
    assert_eq!(std::fs::read(dir.join("project.pcraft")).unwrap(), bytes);
    value(call(&client, "doc_open", json!({"path":"edited.pcraft"})).await);
    let reopened = value(call(&client, "doc_inspect", json!({})).await);
    assert_eq!(reopened["width"], before["width"]);
    assert_eq!(reopened["height"], before["height"]);
    assert_eq!(value(call(&client, "command_run", json!({"id":"xcf.inspect","params":{"limit":1}})).await), semantics);
    let half = value(call(&client, "doc_open", json!({"path":"half.xcf"})).await);
    assert!(half["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("F16")));
    client.cancel().await.unwrap();
    task.await.unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
