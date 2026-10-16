use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::transport::stdio;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};

use scientific_process::assist::tools::Judge;
use scientific_process::store::{self, FilesystemStore};
use scientific_process::{
    Conclusion, Event, Experiment, Hypothesis, Observation, Session, Status, Verdict,
};

#[cfg(feature = "jev")]
use scientific_process::assist::jev::{HttpJudge, JevConfig};

type McpError = rmcp::ErrorData;

// ─── Server struct ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct S {
    store: Arc<FilesystemStore>,
    active: Arc<Mutex<Option<ActiveSession>>>,
    /// Resolved Jev judge, or `None` when assist is disabled/unconfigured.
    judge: Option<Arc<dyn Judge>>,
}

/// Session used when `session.open` names none (pre-persistence callers).
const DEFAULT_SESSION_ID: &str = "default";

struct ActiveSession {
    session_id: String,
    session: Session,
}

impl S {
    fn new() -> Self {
        Self::with_state_dir(store::resolve_state_dir())
    }

    fn with_state_dir(state_dir: impl Into<PathBuf>) -> Self {
        Self::with_state_dir_and_judge(state_dir, resolve_judge_from_env())
    }

    fn with_state_dir_and_judge(
        state_dir: impl Into<PathBuf>,
        judge: Option<Arc<dyn Judge>>,
    ) -> Self {
        let store = FilesystemStore::new(state_dir).expect("create scientific-process state dir");
        Self {
            store: Arc::new(store),
            active: Arc::new(Mutex::new(None)),
            judge,
        }
    }

    fn make_tool(name: &'static str, description: &'static str, schema: Value) -> Tool {
        let schema: Map<String, Value> =
            serde_json::from_value(schema).expect("valid tool input schema");
        Tool::new(name, description, Arc::new(schema))
    }

    fn tool_list() -> Vec<Tool> {
        vec![
            Self::make_tool(
                "session.open",
                "Open a session. With an existing session_id the append-only JSONL log is \
                replayed and the session resumes; with a new id it starts fresh. \
                session_id defaults to \"default\".",
                serde_json::json!({
                    "type": "object",
                    "properties": { "session_id": {"type": "string"} }
                }),
            ),
            Self::make_tool(
                "append",
                "Append an event to the session log. \
                variant must be one of: \
                add_hypothesis (needs id, statement), \
                add_experiment (needs id, hypothesis_id, design, prediction), \
                add_observation (needs id, experiment_id, result, supports:bool, evidence), \
                conclude (needs hypothesis_id, verdict:Supported|Refuted|Inconclusive, rationale). \
                Rejects duplicate ids and unknown references with stable error prefixes \
                (DUPLICATE_ID, HYPOTHESIS_NOT_FOUND, EXPERIMENT_NOT_FOUND).",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "variant": {
                            "type": "string",
                            "enum": ["add_hypothesis", "add_experiment", "add_observation", "conclude"]
                        },
                        "id": {"type": "string"},
                        "statement": {"type": "string"},
                        "hypothesis_id": {"type": "string"},
                        "experiment_id": {"type": "string"},
                        "design": {"type": "string"},
                        "prediction": {"type": "string"},
                        "result": {"type": "string"},
                        "supports": {"type": "boolean"},
                        "evidence": {"type": "string"},
                        "verdict": {
                            "type": "string",
                            "enum": ["Supported", "Refuted", "Inconclusive"]
                        },
                        "rationale": {"type": "string"}
                    },
                    "required": ["variant"]
                }),
            ),
            Self::make_tool(
                "state.get",
                "Return the computed standing: all hypotheses (with derived status), \
                experiments, and observations. Standing is a pure fold over the \
                append-only event log.",
                serde_json::json!({ "type": "object", "properties": {} }),
            ),
            Self::make_tool(
                "experiment.next",
                "Return the highest-leverage open hypothesis (the one with the fewest \
                observations), or null if no open hypothesis remains.",
                serde_json::json!({ "type": "object", "properties": {} }),
            ),
            Self::make_tool(
                "report.export",
                "Export a structured report: each hypothesis with its experiments, \
                observations, and verdict.",
                serde_json::json!({ "type": "object", "properties": {} }),
            ),
            Self::make_tool(
                "adjudicate",
                "Stateless batch falsification adjudicator (does NOT touch the live session). \
                Input: { cases: [ { use_case_id, supports:bool, evidence, rationale? } ] }. For \
                each case it folds an adequacy hypothesis -> refutation experiment -> \
                observation(supports) -> conclusion (Refuted iff supports=false), then returns the \
                COMPUTED verdict { adequate (true iff no case refuted), refuted:[use_case_id...], \
                report }. The caller supplies ONLY the per-case `supports` observation; the verdict \
                is derived here, never by the caller.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "cases": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "use_case_id": {"type": "string"},
                                    "supports": {"type": "boolean"},
                                    "evidence": {"type": "string"},
                                    "rationale": {"type": "string"}
                                },
                                "required": ["use_case_id", "supports"]
                            }
                        }
                    },
                    "required": ["cases"]
                }),
            ),
            Self::make_tool(
                "assist.experiment_check",
                "ADVISORY (never appends events). Ask Jev whether an experiment's design is \
                falsifiable and its prediction specific and measurable. Returns a probability \
                for each, concrete fixes for the weak half, the exact question text sent, the \
                model and token usage. With no configured key it returns ASSIST_UNAVAILABLE.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {"type": "string"},
                        "experiment_id": {"type": "string"}
                    },
                    "required": ["experiment_id"]
                }),
            ),
            Self::make_tool(
                "assist.observation_support",
                "ADVISORY (never appends events). Ask Jev for P(supports / refutes / irrelevant) \
                that an observation supports its hypothesis, from the result and evidence. Flags \
                disagreement with the caller's `supports` flag and returns per-option \
                probabilities. With no configured key it returns ASSIST_UNAVAILABLE.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {"type": "string"},
                        "observation_id": {"type": "string"}
                    },
                    "required": ["observation_id"]
                }),
            ),
            Self::make_tool(
                "assist.verdict_check",
                "ADVISORY (never appends events). Ask Jev whether a stated verdict is backed by \
                the observations, and return deterministic warnings (for example Supported with \
                zero or refuting observations) plus a confidence. With no configured key it \
                returns ASSIST_UNAVAILABLE.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {"type": "string"},
                        "hypothesis_id": {"type": "string"},
                        "verdict": {
                            "type": "string",
                            "enum": ["Supported", "Refuted", "Inconclusive"]
                        }
                    },
                    "required": ["hypothesis_id", "verdict"]
                }),
            ),
            Self::make_tool(
                "assist.hypothesis_check",
                "ADVISORY (never appends events). Ask Jev whether a candidate hypothesis \
                statement is testable and falsifiable, and report near-duplicates of the \
                hypotheses already in the session. With no configured key it returns \
                ASSIST_UNAVAILABLE.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": {"type": "string"},
                        "statement": {"type": "string"}
                    },
                    "required": ["statement"]
                }),
            ),
        ]
    }

    fn call_tool_impl(&self, name: &str, args: Value) -> Result<String, String> {
        match name {
            "session.open" => {
                let session_id = args["session_id"]
                    .as_str()
                    .unwrap_or(DEFAULT_SESSION_ID)
                    .to_string();
                store::validate_session_id(&session_id)?;
                let events = self.store.replay(&session_id)?;
                let session = Session::from_events(events);
                let mut guard = self.active.lock().map_err(|e| format!("lock: {e}"))?;
                *guard = Some(ActiveSession {
                    session_id,
                    session,
                });
                Ok(r#"{"status":"ok","message":"session opened"}"#.to_string())
            }
            "append" => {
                let variant = args["variant"].as_str().unwrap_or("").to_string();
                let id = args["id"].as_str().map(str::to_string);
                let statement = args["statement"].as_str().map(str::to_string);
                let hypothesis_id = args["hypothesis_id"].as_str().map(str::to_string);
                let experiment_id = args["experiment_id"].as_str().map(str::to_string);
                let design = args["design"].as_str().map(str::to_string);
                let prediction = args["prediction"].as_str().map(str::to_string);
                let result_field = args["result"].as_str().map(str::to_string);
                let supports = args["supports"].as_bool();
                let evidence = args["evidence"].as_str().map(str::to_string);
                let verdict = args["verdict"].as_str().map(str::to_string);
                let rationale = args["rationale"].as_str().map(str::to_string);

                let mut guard = self.active.lock().map_err(|e| format!("lock: {e}"))?;
                let active = guard
                    .as_mut()
                    .ok_or("no session open; call session.open first")?;

                let event = match variant.as_str() {
                    "add_hypothesis" => {
                        let id = id.ok_or("missing 'id'")?;
                        let statement = statement.ok_or("missing 'statement'")?;
                        Event::AddHypothesis(Hypothesis {
                            id,
                            statement,
                            status: Status::Open,
                        })
                    }
                    "add_experiment" => {
                        let id = id.ok_or("missing 'id'")?;
                        let hypothesis_id = hypothesis_id.ok_or("missing 'hypothesis_id'")?;
                        let design = design.ok_or("missing 'design'")?;
                        let prediction = prediction.ok_or("missing 'prediction'")?;
                        Event::AddExperiment(Experiment {
                            id,
                            hypothesis_id,
                            design,
                            prediction,
                        })
                    }
                    "add_observation" => {
                        let id = id.ok_or("missing 'id'")?;
                        let experiment_id = experiment_id.ok_or("missing 'experiment_id'")?;
                        let result = result_field.ok_or("missing 'result'")?;
                        let supports = supports.ok_or("missing 'supports'")?;
                        let evidence = evidence.ok_or("missing 'evidence'")?;
                        Event::AddObservation(Observation {
                            id,
                            experiment_id,
                            result,
                            supports,
                            evidence,
                        })
                    }
                    "conclude" => {
                        let hypothesis_id = hypothesis_id.ok_or("missing 'hypothesis_id'")?;
                        let verdict_str = verdict.ok_or("missing 'verdict'")?;
                        let rationale = rationale.ok_or("missing 'rationale'")?;
                        let verdict = match verdict_str.as_str() {
                            "Supported" => Verdict::Supported,
                            "Refuted" => Verdict::Refuted,
                            "Inconclusive" => Verdict::Inconclusive,
                            other => return Err(format!("invalid verdict: {other}")),
                        };
                        Event::Conclude(Conclusion {
                            hypothesis_id,
                            verdict,
                            rationale,
                        })
                    }
                    other => return Err(format!("unknown variant: {other}")),
                };

                // Validate against the current log by applying to a clone, then
                // persist durably, then commit in memory. An invalid event is
                // rejected before anything is written.
                let mut next = active.session.clone();
                match &event {
                    Event::AddHypothesis(h) => next.add_hypothesis(h.clone())?,
                    Event::AddExperiment(e) => next.add_experiment(e.clone())?,
                    Event::AddObservation(o) => next.add_observation(o.clone())?,
                    Event::Conclude(c) => next.conclude(c.clone())?,
                }
                self.store.append(&active.session_id, &event)?;
                active.session = next;
                Ok(r#"{"status":"ok"}"#.to_string())
            }
            "state.get" => {
                let guard = self.active.lock().map_err(|e| format!("lock: {e}"))?;
                let active = guard
                    .as_ref()
                    .ok_or("no session open; call session.open first")?;
                serde_json::to_string_pretty(&active.session.standing())
                    .map_err(|e| format!("json: {e}"))
            }
            "experiment.next" => {
                let guard = self.active.lock().map_err(|e| format!("lock: {e}"))?;
                let active = guard
                    .as_ref()
                    .ok_or("no session open; call session.open first")?;
                match active.session.next_experiment() {
                    Some(h) => serde_json::to_string_pretty(&h).map_err(|e| format!("json: {e}")),
                    None => Ok(serde_json::json!({ "next": null }).to_string()),
                }
            }
            "report.export" => {
                let guard = self.active.lock().map_err(|e| format!("lock: {e}"))?;
                let active = guard
                    .as_ref()
                    .ok_or("no session open; call session.open first")?;
                serde_json::to_string_pretty(&active.session.export_report())
                    .map_err(|e| format!("json: {e}"))
            }
            "adjudicate" => {
                let cases = args["cases"]
                    .as_array()
                    .ok_or("missing 'cases' array")?
                    .clone();
                let mut s = Session::new();
                let mut refuted: Vec<String> = Vec::new();
                for (i, c) in cases.iter().enumerate() {
                    let uc = c["use_case_id"]
                        .as_str()
                        .ok_or("case missing 'use_case_id'")?
                        .to_string();
                    let supports = c["supports"].as_bool().ok_or("case missing 'supports'")?;
                    let evidence = c["evidence"].as_str().unwrap_or("").to_string();
                    let rationale = c["rationale"].as_str().unwrap_or("").to_string();
                    let hid = format!("h_{i}_{uc}");
                    let eid = format!("e_{i}_{uc}");
                    let oid = format!("o_{i}_{uc}");
                    s.add_hypothesis(Hypothesis {
                        id: hid.clone(),
                        statement: format!(
                            "The design is adequate to fully represent/handle use case {uc}"
                        ),
                        status: Status::Open,
                    })?;
                    s.add_experiment(Experiment {
                        id: eid.clone(),
                        hypothesis_id: hid.clone(),
                        design: format!("Attempt to express use case {uc} entirely in the design"),
                        prediction: "Fully representable with no missing node-kind, edge, or field"
                            .to_string(),
                    })?;
                    s.add_observation(Observation {
                        id: oid,
                        experiment_id: eid,
                        result: if supports {
                            "representable".to_string()
                        } else {
                            "GAP: not representable".to_string()
                        },
                        supports,
                        evidence,
                    })?;
                    let verdict = if supports {
                        Verdict::Supported
                    } else {
                        Verdict::Refuted
                    };
                    s.conclude(Conclusion {
                        hypothesis_id: hid,
                        verdict,
                        rationale,
                    })?;
                    if !supports {
                        refuted.push(uc);
                    }
                }
                let report = s.export_report();
                let adequate = refuted.is_empty();
                serde_json::to_string_pretty(&serde_json::json!({
                    "adequate": adequate,
                    "refuted": refuted,
                    "report": report,
                }))
                .map_err(|e| format!("json: {e}"))
            }
            other => Err(format!("unknown tool: {other}")),
        }
    }

    /// Async dispatch: advisory assist tools go through the provider-neutral
    /// dispatcher with a cloned session (never mutating the live log), and the
    /// six core tools fall through to the synchronous path.
    async fn call_tool_async(&self, name: &str, args: Value) -> Result<String, String> {
        if scientific_process::assist::tools::is_assist_tool(name) {
            let session = {
                let guard = self.active.lock().map_err(|e| format!("lock: {e}"))?;
                let active = guard
                    .as_ref()
                    .ok_or("no session open; call session.open first")?;
                active.session.clone()
            };
            let value = scientific_process::assist::tools::dispatch(
                name,
                &args,
                &session,
                self.judge.as_deref(),
            )
            .await
            .expect("assist tool name was checked");
            return serde_json::to_string(&value).map_err(|e| format!("json: {e}"));
        }
        self.call_tool_impl(name, args)
    }
}

/// Resolves the Jev judge from the process environment. Assist is opt-in: a
/// missing `SCIENTIFIC_PROCESS_ASSIST` entry or a missing key simply leaves
/// the judge unset and every assist tool reports ASSIST_UNAVAILABLE.
#[cfg(feature = "jev")]
fn resolve_judge_from_env() -> Option<Arc<dyn Judge>> {
    JevConfig::from_env()
        .ok()
        .map(|config| Arc::new(HttpJudge::new(config)) as Arc<dyn Judge>)
}

#[cfg(not(feature = "jev"))]
fn resolve_judge_from_env() -> Option<Arc<dyn Judge>> {
    None
}

// ─── ServerHandler implementation (rmcp 1.8 idioms — mirror log-mcp) ───────────

impl ServerHandler for S {
    fn get_info(&self) -> ServerInfo {
        let mut result = InitializeResult::default();
        result.capabilities = ServerCapabilities::builder().enable_tools().build();
        result.instructions = Some(
            "scientific-process: append-only scientific-process MCP server.\n\
            Standing is COMPUTED as a fold over the append-only event log — never stored.\n\n\
            Tools:\n\
              session.open    – open or resume a session by session_id (replays its log)\n\
              append          – append an event:\n\
                variant add_hypothesis : id, statement\n\
                variant add_experiment : id, hypothesis_id, design, prediction\n\
                variant add_observation: id, experiment_id, result, supports(bool), evidence\n\
                variant conclude       : hypothesis_id, verdict, rationale\n\
              state.get       – return the computed standing projection\n\
              experiment.next – return the highest-leverage open hypothesis (fewest observations), or null\n\
              report.export   – return the structured report\n\
              adjudicate      – stateless batch falsification adjudicator (does not touch the live session)\n\
            Advisory assist tools (opt-in Jev layer; never append events):\n\
              assist.experiment_check  – falsifiability + measurability of an experiment, with fixes\n\
              assist.observation_support – P(supports/refutes/irrelevant) for an observation\n\
              assist.verdict_check     – warnings when a verdict is not backed by observations\n\
              assist.hypothesis_check  – testability + near-duplicates of a candidate hypothesis\n\
            Every assist tool returns ASSIST_UNAVAILABLE when no key is configured."
                .to_string(),
        );
        result
    }

    #[allow(deprecated)]
    async fn list_tools(
        &self,
        _params: Option<PaginatedRequestParam>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(Self::tool_list()))
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = req
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Default::default()));
        match self.call_tool_async(req.name.as_ref(), args).await {
            Ok(text) => {
                // Populate BOTH text content (human/audit) AND structured_content
                // (machine). Praxec's mcp executor maps `$.output` from
                // structured_content when present, else from the text envelope
                // ({content:[...]}) — so without this, `$.output.adequate` etc.
                // resolve to null in capability output mappings.
                let mut r = CallToolResult::success(vec![Content::text(text.clone())]);
                r.structured_content = serde_json::from_str::<Value>(&text).ok();
                Ok(r)
            }
            // Kernel-level errors (DUPLICATE_ID, *_NOT_FOUND, missing args) come
            // back as a tool error result so the caller sees the stable prefix.
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }
}

// ─── Entry point + CLI ────────────────────────────────────────────────────────

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
scientific-process: append-only scientific-process MCP server.

Usage:
  scientific-process                 Run the MCP server on stdio (how MCP clients start it)
  scientific-process skills <cmd>    Install, uninstall or list the agent skills
  scientific-process --version       Print the version
  scientific-process --help          Print this help
";

/// `scientific-process <version>`, the line `--version` prints.
fn version_line() -> String {
    format!("scientific-process {VERSION}")
}

/// The command line, after UTF-8 validation, before any IO.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Server,
    Version,
    Help,
    Skills(Vec<String>),
    Usage(String),
}

fn parse_command(args: &[String]) -> Command {
    if args.is_empty() {
        return Command::Server;
    }
    match args[0].as_str() {
        "--version" | "-V" if args.len() == 1 => Command::Version,
        "--help" | "-h" | "help" if args.len() == 1 => Command::Help,
        "skills" => Command::Skills(args[1..].to_vec()),
        _ => Command::Usage(args.join(" ")),
    }
}

fn main() -> ExitCode {
    let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // No arguments: the MCP stdio server, exactly as every MCP client runs it.
    // Nothing else may reach stdout in this mode.
    if raw.is_empty() {
        return run_server();
    }
    let Some(args) = raw
        .into_iter()
        .map(|a| a.into_string().ok())
        .collect::<Option<Vec<String>>>()
    else {
        eprintln!("scientific-process: arguments must be valid UTF-8");
        return ExitCode::from(scientific_process::skills::EXIT_USAGE);
    };
    match parse_command(&args) {
        Command::Server => run_server(),
        Command::Version => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        Command::Help => {
            print!("{USAGE}\n{}", scientific_process::skills::USAGE);
            ExitCode::SUCCESS
        }
        Command::Skills(rest) => ExitCode::from(scientific_process::skills::run(&rest)),
        Command::Usage(joined) => {
            eprintln!("scientific-process: unexpected arguments `{joined}`\n\n{USAGE}");
            ExitCode::from(scientific_process::skills::EXIT_USAGE)
        }
    }
}

fn run_server() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("Error: cannot start the async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:?}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> anyhow::Result<()> {
    eprintln!("scientific-process: serving on stdio");
    let svc = S::new().serve(stdio()).await?;
    svc.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_open_without_session_id_opens_the_default_session() {
        let dir = tempfile::tempdir().unwrap();
        let server = S::with_state_dir(dir.path());
        call(&server, "session.open", Value::Object(Default::default())).unwrap();
        call(
            &server,
            "append",
            serde_json::json!({"variant": "add_hypothesis", "id": "h1", "statement": "s"}),
        )
        .unwrap();
        assert!(dir.path().join("default.jsonl").exists());
    }

    fn call(s: &S, name: &str, args: Value) -> Result<String, String> {
        s.call_tool_impl(name, args)
    }

    #[test]
    fn session_open_replays_appended_events_in_a_new_instance() {
        let dir = tempfile::tempdir().unwrap();
        let first = S::with_state_dir(dir.path());
        call(
            &first,
            "session.open",
            serde_json::json!({ "session_id": "s1" }),
        )
        .unwrap();
        call(
            &first,
            "append",
            serde_json::json!({
                "variant": "add_hypothesis",
                "id": "h1",
                "statement": "water boils at 100C"
            }),
        )
        .unwrap();

        let second = S::with_state_dir(dir.path());
        call(
            &second,
            "session.open",
            serde_json::json!({ "session_id": "s1" }),
        )
        .unwrap();
        let standing: Value = serde_json::from_str(
            &call(&second, "state.get", Value::Object(Default::default())).unwrap(),
        )
        .unwrap();

        assert_eq!(
            standing["hypotheses"],
            serde_json::json!([{
                "id": "h1",
                "statement": "water boils at 100C",
                "status": "Open"
            }])
        );
    }

    #[test]
    fn duplicate_append_leaves_log_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let s = S::with_state_dir(dir.path());
        call(
            &s,
            "session.open",
            serde_json::json!({ "session_id": "s1" }),
        )
        .unwrap();
        call(
            &s,
            "append",
            serde_json::json!({
                "variant": "add_hypothesis",
                "id": "h1",
                "statement": "first"
            }),
        )
        .unwrap();
        let _ = call(
            &s,
            "append",
            serde_json::json!({
                "variant": "add_hypothesis",
                "id": "h1",
                "statement": "second"
            }),
        );

        let log = std::fs::read_to_string(dir.path().join("s1.jsonl")).unwrap();
        assert_eq!(log.lines().count(), 1);
    }

    #[test]
    fn unknown_reference_append_leaves_log_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let s = S::with_state_dir(dir.path());
        call(
            &s,
            "session.open",
            serde_json::json!({ "session_id": "s1" }),
        )
        .unwrap();
        let _ = call(
            &s,
            "append",
            serde_json::json!({
                "variant": "add_experiment",
                "id": "e1",
                "hypothesis_id": "missing",
                "design": "d",
                "prediction": "p"
            }),
        );

        assert!(!dir.path().join("s1.jsonl").exists());
    }

    #[test]
    fn experiment_next_without_open_hypothesis_reports_null_object() {
        let dir = tempfile::tempdir().unwrap();
        let s = S::with_state_dir(dir.path());
        call(
            &s,
            "session.open",
            serde_json::json!({ "session_id": "s1" }),
        )
        .unwrap();

        let out: Value = serde_json::from_str(
            &call(&s, "experiment.next", Value::Object(Default::default())).unwrap(),
        )
        .unwrap();

        assert_eq!(out, serde_json::json!({ "next": null }));
    }

    #[test]
    fn server_instructions_mention_adjudicate() {
        let dir = tempfile::tempdir().unwrap();
        let instructions = S::with_state_dir(dir.path())
            .get_info()
            .instructions
            .unwrap();

        assert!(instructions.contains("adjudicate"));
    }

    #[test]
    fn version_line_names_the_binary_and_package_version() {
        assert_eq!(version_line(), "scientific-process 0.1.0");
    }

    #[test]
    fn no_arguments_selects_the_server_command() {
        assert_eq!(parse_command(&[]), Command::Server);
    }

    #[test]
    fn version_flag_selects_the_version_command() {
        assert_eq!(parse_command(&["--version".to_string()]), Command::Version);
    }

    #[test]
    fn skills_subcommand_forwards_its_arguments() {
        let args = ["skills", "install", "--target", "claude"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            parse_command(&args),
            Command::Skills(vec!["install".into(), "--target".into(), "claude".into()])
        );
    }

    #[cfg(feature = "jev")]
    mod assist_dispatch_tests {
        use super::*;
        use async_trait::async_trait;
        use scientific_process::assist::Usage;
        use scientific_process::assist::jev::{JevAnswer, JevQuestion, JevReply};
        use std::collections::BTreeMap;

        struct RefutesJudge;

        #[async_trait]
        impl Judge for RefutesJudge {
            async fn ask(
                &self,
                _state: Value,
                questions: BTreeMap<String, JevQuestion>,
            ) -> Result<JevReply, scientific_process::assist::AssistUnavailable> {
                let answers = questions
                    .keys()
                    .map(|id| {
                        (
                            id.clone(),
                            JevAnswer::Choice {
                                choice: "refutes".to_string(),
                                probabilities: BTreeMap::from([
                                    ("refutes".to_string(), 0.8),
                                    ("supports".to_string(), 0.1),
                                    ("irrelevant".to_string(), 0.1),
                                ]),
                                confidence: 0.8,
                            },
                        )
                    })
                    .collect();
                Ok(JevReply {
                    model: "fake/jev".to_string(),
                    answers,
                    usage: Usage::default(),
                })
            }
        }

        fn open_with_observation(s: &S) {
            call(s, "session.open", serde_json::json!({ "session_id": "s1" })).unwrap();
            call(
                s,
                "append",
                serde_json::json!({
                    "variant": "add_hypothesis", "id": "h1", "statement": "water boils"
                }),
            )
            .unwrap();
            call(
                s,
                "append",
                serde_json::json!({
                    "variant": "add_experiment", "id": "e1", "hypothesis_id": "h1",
                    "design": "boil", "prediction": "100C"
                }),
            )
            .unwrap();
            call(
                s,
                "append",
                serde_json::json!({
                    "variant": "add_observation", "id": "o1", "experiment_id": "e1",
                    "result": "80C", "supports": true, "evidence": "thermometer"
                }),
            )
            .unwrap();
        }

        #[tokio::test]
        async fn assist_call_leaves_state_get_unchanged() {
            let dir = tempfile::tempdir().unwrap();
            let s = S::with_state_dir_and_judge(dir.path(), Some(Arc::new(RefutesJudge)));
            open_with_observation(&s);
            let before = s
                .call_tool_async("state.get", Value::Object(Default::default()))
                .await
                .unwrap();
            let _ = s
                .call_tool_async(
                    "assist.observation_support",
                    serde_json::json!({ "observation_id": "o1" }),
                )
                .await
                .unwrap();
            let after = s
                .call_tool_async("state.get", Value::Object(Default::default()))
                .await
                .unwrap();

            assert_eq!(before, after);
        }

        #[tokio::test]
        async fn assist_experiment_check_without_judge_reports_assist_unavailable() {
            let dir = tempfile::tempdir().unwrap();
            let s = S::with_state_dir_and_judge(dir.path(), None);
            open_with_observation(&s);
            let text = s
                .call_tool_async(
                    "assist.experiment_check",
                    serde_json::json!({ "experiment_id": "e1" }),
                )
                .await
                .unwrap();

            assert!(text.contains("ASSIST_UNAVAILABLE"));
        }

        #[tokio::test]
        async fn session_open_still_works_after_an_assist_unavailable_call() {
            let dir = tempfile::tempdir().unwrap();
            let s = S::with_state_dir_and_judge(dir.path(), None);
            open_with_observation(&s);
            let _ = s
                .call_tool_async(
                    "assist.experiment_check",
                    serde_json::json!({ "experiment_id": "e1" }),
                )
                .await;
            let reopened = s
                .call_tool_async("session.open", serde_json::json!({ "session_id": "s1" }))
                .await;

            assert!(reopened.is_ok());
        }
    }
}
