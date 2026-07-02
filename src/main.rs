use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::transport::stdio;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};

use scientific_process::{
    Conclusion, Experiment, Hypothesis, Observation, Session, Status, Verdict,
};

type McpError = rmcp::ErrorData;

// ─── Server struct ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct S {
    session: Arc<Mutex<Session>>,
}

impl S {
    fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(Session::new())),
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
                "Reset the session to a fresh state. No arguments required.",
                serde_json::json!({ "type": "object", "properties": {} }),
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
        ]
    }

    fn call_tool_impl(&self, name: &str, args: Value) -> Result<String, String> {
        match name {
            "session.open" => {
                let mut guard = self.session.lock().map_err(|e| format!("lock: {e}"))?;
                *guard = Session::new();
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

                let mut guard = self.session.lock().map_err(|e| format!("lock: {e}"))?;

                match variant.as_str() {
                    "add_hypothesis" => {
                        let id = id.ok_or("missing 'id'")?;
                        let statement = statement.ok_or("missing 'statement'")?;
                        guard.add_hypothesis(Hypothesis {
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
                        guard.add_experiment(Experiment {
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
                        guard.add_observation(Observation {
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
                        guard.conclude(Conclusion {
                            hypothesis_id,
                            verdict,
                            rationale,
                        })
                    }
                    other => Err(format!("unknown variant: {other}")),
                }
                .map(|()| r#"{"status":"ok"}"#.to_string())
            }
            "state.get" => {
                let guard = self.session.lock().map_err(|e| format!("lock: {e}"))?;
                serde_json::to_string_pretty(&guard.standing()).map_err(|e| format!("json: {e}"))
            }
            "experiment.next" => {
                let guard = self.session.lock().map_err(|e| format!("lock: {e}"))?;
                match guard.next_experiment() {
                    Some(h) => serde_json::to_string_pretty(&h).map_err(|e| format!("json: {e}")),
                    None => Ok("null".to_string()),
                }
            }
            "report.export" => {
                let guard = self.session.lock().map_err(|e| format!("lock: {e}"))?;
                serde_json::to_string_pretty(&guard.export_report())
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
              session.open    – reset the session (start fresh)\n\
              append          – append an event:\n\
                variant add_hypothesis : id, statement\n\
                variant add_experiment : id, hypothesis_id, design, prediction\n\
                variant add_observation: id, experiment_id, result, supports(bool), evidence\n\
                variant conclude       : hypothesis_id, verdict, rationale\n\
              state.get       – return the computed standing projection\n\
              experiment.next – return the highest-leverage open hypothesis (fewest observations), or null\n\
              report.export   – return the structured report"
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
        match self.call_tool_impl(req.name.as_ref(), args) {
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

// ─── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    eprintln!("scientific-process: serving on stdio");
    let svc = S::new().serve(stdio()).await?;
    svc.waiting().await?;
    Ok(())
}
