//! Localhost JSON-lines control server (one request per line, one reply per line).
//! Loopback only. This is the transport the MCP server will wrap.

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use std::time::{Duration, Instant};

use photocraft_automation::budgets::write_reply;
use photocraft_automation::security::{
    ConnectionLimiter, LineRead, MAX_CONNECTIONS, MAX_REQUEST_BYTES, authentication_reply, configure_stream, read_bounded_line, repeat_auth_reply,
};
use photocraft_ui_egui::ControlRequest;
use photocraft_ui_egui::control::REPLY_DEADLINE;
use serde_json::{Value, json};

pub fn start(port: u16, token: String, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("photocraft: control server failed to bind 127.0.0.1:{port}: {e}");
            return rx;
        }
    };
    eprintln!("photocraft: control server listening on 127.0.0.1:{port}");
    std::thread::spawn(move || {
        let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
        let token = Arc::new(token);
        for mut stream in listener.incoming().flatten() {
            let Some(permit) = limiter.try_acquire() else {
                let _ = configure_stream(&stream);
                let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
                continue;
            };
            let tx = tx.clone();
            let ctx = ctx.clone();
            let token = Arc::clone(&token);
            std::thread::spawn(move || {
                let _permit = permit;
                serve(stream, &token, tx, ctx);
            });
        }
    });
    rx
}

/// How long `events.poll {"waitMs"}` may hold a reply for events at most.
const MAX_WAIT: Duration = Duration::from_secs(25);
/// How often a waiting poll asks the app again.
const WAIT_STEP: Duration = Duration::from_millis(50);
/// How often a subscribed connection checks for pushed events between requests.
const PUSH_STEP: Duration = Duration::from_millis(100);

/// Ask the app once; `None` when it went away.
fn ask(
    tx: &Sender<ControlRequest>,
    ctx: &egui::Context,
    mut req: ControlRequest,
    rrx: Receiver<photocraft_ui_egui::control::ControlResponse>,
    deadline: Instant,
) -> Option<Value> {
    req.deadline = Some(deadline);
    let dispatch = req.dispatch.clone();
    if tx.send(req).is_err() {
        return None;
    }
    ctx.request_repaint();
    Some(rrx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| json!({"ok": false, "error": dispatch.give_up()})))
}

/// Write every pushed event line waiting for a subscribed connection. False once writing fails.
fn push_pending(out: &mut TcpStream, push: &Receiver<String>) -> bool {
    while let Ok(line) = push.try_recv() {
        if writeln!(out, "{line}").is_err() || out.flush().is_err() {
            return false;
        }
    }
    true
}

fn serve(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context) {
    if configure_stream(&stream).is_err() {
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read);
    let mut out = stream;
    let mut line = String::new();
    let mut authenticated = false;
    // Pushed events for this connection once it subscribed (#4939). A subscriber reads with a
    // short timeout so the pushes go out between requests, and an idle one stays connected.
    let mut push: Option<Receiver<String>> = None;
    'conn: loop {
        match read_bounded_line(&mut reader, &mut line) {
            Err(e) if push.is_some() && matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if let Some(rx) = &push
                    && !push_pending(&mut out, rx)
                {
                    break;
                }
                continue;
            }
            Ok(LineRead::Eof) | Err(_) => break,
            Ok(LineRead::TooLong) => {
                let reply = json!({
                    "id": null,
                    "ok": false,
                    "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
                });
                let _ = write_reply(&mut out, &reply);
                let _ = out.flush();
                break;
            }
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => {}
        }
        if !authenticated {
            let (reply, ok) = authentication_reply(&line, token);
            authenticated = ok;
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() || !authenticated {
                break;
            }
            continue;
        }
        if let Some(reply) = repeat_auth_reply(&line) {
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() {
                break;
            }
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let deadline = Instant::now() + REPLY_DEADLINE;
                let mut r = match method.as_str() {
                    "events.subscribe" => {
                        // The app pushes into this queue; this thread writes it out.
                        let (ptx, prx) = sync_channel::<String>(photocraft_ui_egui::control::PUSH_QUEUE);
                        let (mut req, rrx) = ControlRequest::new(method, params);
                        req.push = Some(ptx);
                        let Some(r) = ask(&tx, &ctx, req, rrx, deadline) else { break };
                        if r.get("ok").and_then(Value::as_bool) == Some(true) {
                            push = Some(prx);
                            let _ = out.set_read_timeout(Some(PUSH_STEP));
                        }
                        r
                    }
                    "events.unsubscribe" => {
                        // Dropping the queue ends the subscription in the app.
                        push = None;
                        let _ = configure_stream(&out);
                        let (req, rrx) = ControlRequest::new(method, params);
                        let Some(r) = ask(&tx, &ctx, req, rrx, deadline) else { break };
                        r
                    }
                    "events.poll" => {
                        // `waitMs`: hold the reply until an event arrives or the wait is over,
                        // asking the app again every few frames (long poll).
                        let wait = params.get("waitMs").and_then(Value::as_u64).map(|ms| Duration::from_millis(ms).min(MAX_WAIT)).unwrap_or(Duration::ZERO);
                        let until = Instant::now() + wait;
                        let mut p = params.clone();
                        if let Some(o) = p.as_object_mut() {
                            o.remove("waitMs");
                        }
                        loop {
                            let (req, rrx) = ControlRequest::new(method.clone(), p.clone());
                            let Some(r) = ask(&tx, &ctx, req, rrx, deadline) else { break 'conn };
                            let empty = r.get("ok").and_then(Value::as_bool) == Some(true) && r["result"]["events"].as_array().is_some_and(Vec::is_empty);
                            if !empty || Instant::now() >= until {
                                break r;
                            }
                            std::thread::sleep(WAIT_STEP);
                        }
                    }
                    _ => {
                        let (req, rrx) = ControlRequest::new(method, params);
                        let Some(r) = ask(&tx, &ctx, req, rrx, deadline) else { break };
                        r
                    }
                };
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
        };
        if write_reply(&mut out, &reply).is_err() {
            break;
        }
        if out.flush().is_err() {
            break;
        }
        if let Some(rx) = &push
            && !push_pending(&mut out, rx)
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::time::Duration;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A subscribed connection receives the lines the app pushes between its requests, and
    /// `events.poll` with `waitMs` holds its reply until there is something to return (#4939).
    #[test]
    fn subscribed_connections_receive_pushed_events_and_polls_can_wait() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        // A stand-in for the app: answers requests, keeps the subscriber's queue, pushes into it.
        let handler = std::thread::spawn(move || {
            let mut push: Option<std::sync::mpsc::SyncSender<String>> = None;
            let mut polls = 0;
            while let Ok(req) = rx.recv_timeout(Duration::from_secs(10)) {
                match req.method.as_str() {
                    "events.subscribe" => {
                        push = req.push.clone();
                        assert!(push.is_some(), "the transport hands the app a queue");
                        req.reply.send(json!({"ok": true, "result": {"subscribed": true, "next": 0}})).unwrap();
                        // Pushed after the reply, as the app does on later frames.
                        for i in 1..=3 {
                            push.as_ref().unwrap().try_send(json!({"seq": i, "event": "document.changed"}).to_string()).unwrap();
                        }
                    }
                    "events.poll" => {
                        polls += 1;
                        assert!(req.params.get("waitMs").is_none(), "the wait stays in the transport");
                        // Empty twice, then an event: the transport keeps asking meanwhile.
                        let events = if polls < 3 { json!([]) } else { json!([{"seq": 9, "event": "tool.changed"}]) };
                        req.reply.send(json!({"ok": true, "result": {"events": events, "next": 9, "gap": false}})).unwrap();
                    }
                    "events.unsubscribe" => {
                        req.reply.send(json!({"ok": true, "result": {"subscribed": false}})).unwrap();
                        assert!(push.take().is_some());
                    }
                    other => req.reply.send(json!({"ok": true, "result": other})).unwrap(),
                }
            }
            polls
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        let mut read = |reader: &mut BufReader<TcpStream>| -> Value {
            line.clear();
            reader.read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        };
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
        assert_eq!(read(&mut reader)["ok"], true);
        writeln!(stream, "{}", json!({"id": 2, "method": "events.subscribe", "params": {}})).unwrap();
        let r = read(&mut reader);
        assert_eq!((r["id"].as_i64(), r["ok"].as_bool()), (Some(2), Some(true)), "{r}");
        // The three pushed lines arrive without a request, each a JSON object without an id.
        for i in 1..=3 {
            let e = read(&mut reader);
            assert_eq!(e["seq"], i, "{e}");
            assert!(e.get("id").is_none());
        }
        // A long poll: the transport re-asks the app until there is an event.
        let t = Instant::now();
        writeln!(stream, "{}", json!({"id": 3, "method": "events.poll", "params": {"since": 0, "waitMs": 5000}})).unwrap();
        let r = read(&mut reader);
        assert_eq!(r["id"], 3);
        assert_eq!(r["result"]["events"][0]["seq"], 9, "{r}");
        assert!(t.elapsed() >= WAIT_STEP * 2 && t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
        writeln!(stream, "{}", json!({"id": 4, "method": "events.unsubscribe"})).unwrap();
        assert_eq!(read(&mut reader)["result"]["subscribed"], false);
        writeln!(stream, "{}", json!({"id": 5, "method": "test.after"})).unwrap();
        assert_eq!(read(&mut reader)["result"], "test.after", "the connection keeps serving requests");
        drop(stream);
        drop(reader);
        server.join().unwrap();
        assert_eq!(handler.join().unwrap(), 3, "two empty polls, then the one with an event");
    }

    #[test]
    fn oversized_reply_preserves_framing_id_and_the_next_control_request() {
        use photocraft_automation::budgets::MAX_RESPONSE_BYTES;
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        let handler = std::thread::spawn(move || {
            let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(first.method, "test.large");
            assert!(first.deadline.is_some_and(|deadline| deadline > std::time::Instant::now()), "native requests carry their reply deadline");
            first.reply.send(json!({"ok": true, "result": "x".repeat(MAX_RESPONSE_BYTES)})).unwrap();
            let second = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(second.method, "test.small");
            second.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true);
        writeln!(stream, "{}", json!({"id": 2, "method": "test.large"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let rejected: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(rejected["id"], 2);
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"].as_str().unwrap().contains("operation may have completed"));
        writeln!(stream, "{}", json!({"id": 3, "method": "test.small"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let accepted: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(accepted["id"], 3);
        assert_eq!(accepted["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn bad_json_reply_carries_a_null_id_and_the_connection_keeps_serving() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        let handler = std::thread::spawn(move || {
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.after");
            req.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": "auth", "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true);
        writeln!(stream, "{{not json}}").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let bad: Value = serde_json::from_str(&line).unwrap();
        let bad = bad.as_object().unwrap();
        assert_eq!(bad.get("id"), Some(&Value::Null), "bad JSON reply must carry `id: null`: {line}");
        assert_eq!(bad.get("ok"), Some(&Value::Bool(false)));
        assert!(bad.get("error").and_then(Value::as_str).unwrap().starts_with("bad JSON: "));
        writeln!(stream, "{}", json!({"id": 7, "method": "test.after"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let after: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(after["id"], 7);
        assert_eq!(after["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn repeat_auth_reports_already_authenticated_without_dispatching() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        let handler = std::thread::spawn(move || {
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.after", "a repeat auth must not reach the UI dispatcher");
            req.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        for id in ["auth", "again"] {
            writeln!(stream, "{}", json!({"id": id, "method": "auth", "params": {"token": TOKEN}})).unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
        }
        let again: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(again["id"], "again");
        assert_eq!(again["ok"], false);
        assert_eq!(again["error"], "already authenticated");
        writeln!(stream, "{}", json!({"id": 9, "method": "test.after"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let after: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(after["id"], 9);
        assert_eq!(after["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }
}
