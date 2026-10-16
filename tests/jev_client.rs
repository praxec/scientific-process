//! HTTP-wire behavior for the Jev client, exercised against a minimal local
//! stub server on loopback. No test reaches the real Jev or OpenRouter.

#![cfg(feature = "jev")]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use scientific_process::assist::UnavailableReason;
use scientific_process::assist::jev::{HttpJudge, JevAnswer, JevConfig, JevQuestion, Judge};
use serde_json::{Value, json};

fn config_for(base: &str, timeout_secs: u64) -> JevConfig {
    let timeout = timeout_secs.to_string();
    JevConfig::from_lookup(&|key| match key {
        "SCIENTIFIC_PROCESS_ASSIST" => Some("jev".to_string()),
        "TYPESAFE_JEV_API_KEY" => Some("sk-test".to_string()),
        "SCIENTIFIC_PROCESS_JEV_BASE_URL" => Some(base.to_string()),
        "SCIENTIFIC_PROCESS_JEV_TIMEOUT_SECS" => Some(timeout.clone()),
        _ => None,
    })
    .expect("test config must build")
}

fn noul_question() -> JevQuestion {
    JevQuestion::Noul {
        instructions: "is it urgent?".to_string(),
        criteria: None,
    }
}

fn questions_of(count: usize) -> BTreeMap<String, JevQuestion> {
    (0..count)
        .map(|index| (format!("q{index}"), noul_question()))
        .collect()
}

fn one_question(id: &str, question: JevQuestion) -> BTreeMap<String, JevQuestion> {
    BTreeMap::from([(id.to_string(), question)])
}

/// A one-connection HTTP/1.1 stub. Returns its loopback base URL and the
/// request text it received.
fn spawn_stub(status: u16, body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let request = read_request(&mut stream);
            sink.lock().expect("stub mutex").push(request);
            let reason = if (200..300).contains(&status) {
                "OK"
            } else {
                "Error"
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{}", addr.port()), captured)
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if let Some(position) = find_subsequence(&buffer, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buffer[..position + 4]).to_string();
                    let content_length = head
                        .to_lowercase()
                        .split("content-length:")
                        .nth(1)
                        .and_then(|rest| rest.split("\r\n").next())
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buffer.len() >= position + 4 + content_length {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[tokio::test]
async fn choice_reply_decodes_to_the_choice_answer() {
    let (base, _) = spawn_stub(
        200,
        r#"{"model":"jev-latest","answers":{"support":{"type":"choice","choice":"refutes","probabilities":{"refutes":0.8,"supports":0.1,"irrelevant":0.1},"confidence":0.8}}}"#,
    );
    let judge = HttpJudge::new(config_for(&base, 5));
    let question = JevQuestion::Choice {
        instructions: "support, refute or irrelevant?".to_string(),
        criteria: BTreeMap::from([
            ("supports".to_string(), "d".to_string()),
            ("refutes".to_string(), "d".to_string()),
            ("irrelevant".to_string(), "d".to_string()),
        ]),
    };

    let reply = judge
        .ask(json!({"text": "hi"}), one_question("support", question))
        .await
        .expect("choice reply must decode");

    assert_eq!(
        reply.answers["support"],
        JevAnswer::Choice {
            choice: "refutes".to_string(),
            probabilities: BTreeMap::from([
                ("refutes".to_string(), 0.8),
                ("supports".to_string(), 0.1),
                ("irrelevant".to_string(), 0.1),
            ]),
            confidence: 0.8,
        }
    );
}

#[tokio::test]
async fn request_carries_bearer_authorization() {
    let (base, captured) = spawn_stub(
        200,
        r#"{"model":"jev-latest","answers":{"q":{"type":"noul","noul":0.25}}}"#,
    );
    let judge = HttpJudge::new(config_for(&base, 5));

    let _ = judge
        .ask(json!({"text": "hi"}), one_question("q", noul_question()))
        .await;
    let request = captured.lock().expect("stub mutex")[0].to_lowercase();

    assert!(request.contains("authorization: bearer sk-test"));
}

#[tokio::test]
async fn http_401_yields_http4xx() {
    let (base, _) = spawn_stub(401, "invalid key");
    let judge = HttpJudge::new(config_for(&base, 5));

    let err = judge
        .ask(json!({"text": "hi"}), one_question("q", noul_question()))
        .await
        .expect_err("401 must fail");

    assert_eq!(err.reason, UnavailableReason::Http4xx);
}

#[tokio::test]
async fn fifty_one_questions_yields_bad_config() {
    let (base, _) = spawn_stub(200, "{}");
    let judge = HttpJudge::new(config_for(&base, 5));

    let err = judge
        .ask(json!({"text": "hi"}), questions_of(51))
        .await
        .expect_err("too many questions must fail");

    assert_eq!(err.reason, UnavailableReason::BadConfig);
}

#[tokio::test]
async fn fifty_one_questions_sends_no_request() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let base = format!(
        "http://127.0.0.1:{}",
        listener.local_addr().expect("addr").port()
    );
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let judge = HttpJudge::new(config_for(&base, 5));

    let _ = judge.ask(json!({"text": "hi"}), questions_of(51)).await;

    assert!(listener.accept().is_err());
}

#[tokio::test]
async fn request_body_carries_the_model() {
    let (base, captured) = spawn_stub(
        200,
        r#"{"model":"jev-latest","answers":{"q":{"type":"noul","noul":0.25}}}"#,
    );
    let judge = HttpJudge::new(config_for(&base, 5));

    let _ = judge
        .ask(json!({"text": "hi"}), one_question("q", noul_question()))
        .await;
    let request = captured.lock().expect("stub mutex")[0].clone();
    let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap_or("{}"))
        .expect("request body json");

    assert_eq!(body["model"], "jev-latest");
}
