//! The four advisory assist tools. Each one borrows the session, asks the
//! judge, and returns a proposal. None of them appends an event: the session
//! is only ever read, so the append-only log and the pure fold are untouched.
//!
//! Every response carries the question text sent to the model, per-option
//! probabilities (or the `Noul` probability), the model name and token usage.
//! An unavailable backend or a bad request is reported as a structured value
//! with `ok: false`, never as a mutation.

#[cfg(feature = "jev")]
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
#[cfg(feature = "jev")]
use serde_json::json;

use crate::Session;
use crate::assist::{AssistUnavailable, UnavailableReason};

#[cfg(feature = "jev")]
use crate::assist::Usage;

#[cfg(feature = "jev")]
pub use crate::assist::jev::Judge;

/// Placeholder judge trait for builds without the `jev` feature. It is never
/// implemented because every assist kind returns `FeatureNotBuilt` without
/// inspecting the judge.
#[cfg(not(feature = "jev"))]
pub trait Judge: Send + Sync {}

pub const EXPERIMENT_CHECK: &str = "assist.experiment_check";
pub const OBSERVATION_SUPPORT: &str = "assist.observation_support";
pub const VERDICT_CHECK: &str = "assist.verdict_check";
pub const HYPOTHESIS_CHECK: &str = "assist.hypothesis_check";

const EXPERIMENT_LINKS: &[&str] = &["append", "state.get", "experiment.next"];
const OBSERVATION_LINKS: &[&str] = &["append", "state.get", "assist.verdict_check"];
const VERDICT_LINKS: &[&str] = &["state.get", "report.export", "assist.hypothesis_check"];
const HYPOTHESIS_LINKS: &[&str] = &["append", "state.get", "assist.experiment_check"];

/// True when `name` names one of the four advisory assist tools.
pub fn is_assist_tool(name: &str) -> bool {
    links_for(name).is_some()
}

fn links_for(name: &str) -> Option<&'static [&'static str]> {
    match name {
        EXPERIMENT_CHECK => Some(EXPERIMENT_LINKS),
        OBSERVATION_SUPPORT => Some(OBSERVATION_LINKS),
        VERDICT_CHECK => Some(VERDICT_LINKS),
        HYPOTHESIS_CHECK => Some(HYPOTHESIS_LINKS),
        _ => None,
    }
}

/// One tool failure. `Unavailable` becomes an `assist_unavailable` value;
/// `BadRequest` becomes a `bad_request` value. Neither mutates the session.
enum ToolError {
    Unavailable(AssistUnavailable),
    #[cfg(feature = "jev")]
    BadRequest(String),
}

fn unavailable_value(error: &AssistUnavailable, links: &[&str]) -> Value {
    error.to_value(links)
}

#[cfg(feature = "jev")]
fn bad_request_value(detail: &str, links: &[&str]) -> Value {
    json!({
        "ok": false,
        "status": "bad_request",
        "error": "BAD_REQUEST",
        "detail": detail,
        "links": links,
    })
}

/// Dispatches one advisory tool. Returns `None` when `name` is not an assist
/// tool (the caller then uses the synchronous core path). The returned value
/// is always a report; the session is never mutated.
pub async fn dispatch(
    name: &str,
    args: &Value,
    session: &Session,
    judge: Option<&dyn Judge>,
) -> Option<Value> {
    let links = links_for(name)?;
    let result = match name {
        EXPERIMENT_CHECK => experiment_check(args, session, judge).await,
        OBSERVATION_SUPPORT => observation_support(args, session, judge).await,
        VERDICT_CHECK => verdict_check(args, session, judge).await,
        HYPOTHESIS_CHECK => hypothesis_check(args, session, judge).await,
        _ => return None,
    };
    Some(match result {
        Ok(value) => value,
        Err(ToolError::Unavailable(error)) => unavailable_value(&error, links),
        #[cfg(feature = "jev")]
        Err(ToolError::BadRequest(detail)) => bad_request_value(&detail, links),
    })
}

#[cfg(feature = "jev")]
fn required_str(args: &Value, name: &str) -> Result<String, ToolError> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::BadRequest(format!("missing '{name}'")))
}

#[cfg(feature = "jev")]
fn usage_value(usage: &Usage) -> Value {
    json!({
        "inputTokens": usage.input_tokens,
        "outputTokens": usage.output_tokens,
        "costUsd": usage.cost_usd,
    })
}

// ── feature-off stubs ────────────────────────────────────────────────

#[cfg(not(feature = "jev"))]
fn feature_not_built(_links: &[&str]) -> ToolError {
    ToolError::Unavailable(AssistUnavailable::new(
        UnavailableReason::FeatureNotBuilt,
        "the jev feature is not compiled",
    ))
}

#[cfg(not(feature = "jev"))]
async fn experiment_check(
    _args: &Value,
    _session: &Session,
    _judge: Option<&dyn Judge>,
) -> Result<Value, ToolError> {
    Err(feature_not_built(EXPERIMENT_LINKS))
}

#[cfg(not(feature = "jev"))]
async fn observation_support(
    _args: &Value,
    _session: &Session,
    _judge: Option<&dyn Judge>,
) -> Result<Value, ToolError> {
    Err(feature_not_built(OBSERVATION_LINKS))
}

#[cfg(not(feature = "jev"))]
async fn verdict_check(
    _args: &Value,
    _session: &Session,
    _judge: Option<&dyn Judge>,
) -> Result<Value, ToolError> {
    Err(feature_not_built(VERDICT_LINKS))
}

#[cfg(not(feature = "jev"))]
async fn hypothesis_check(
    _args: &Value,
    _session: &Session,
    _judge: Option<&dyn Judge>,
) -> Result<Value, ToolError> {
    Err(feature_not_built(HYPOTHESIS_LINKS))
}

// ── jev-backed handlers ──────────────────────────────────────────────

#[cfg(feature = "jev")]
use crate::assist::jev::{JevAnswer, JevQuestion, JevReply, MAX_QUESTIONS_PER_CALL};

#[cfg(feature = "jev")]
fn require_judge(judge: Option<&dyn Judge>) -> Result<&dyn Judge, ToolError> {
    judge.ok_or_else(|| {
        ToolError::Unavailable(AssistUnavailable::new(
            UnavailableReason::NoKey,
            "assist judge is not configured; set SCIENTIFIC_PROCESS_ASSIST=jev and \
             TYPESAFE_JEV_API_KEY or OPENROUTER_API_KEY",
        ))
    })
}

#[cfg(feature = "jev")]
fn noul_of(reply: &JevReply, id: &str) -> Result<f64, ToolError> {
    match reply.answers.get(id) {
        Some(JevAnswer::Noul { noul }) => Ok(*noul),
        _ => Err(ToolError::Unavailable(AssistUnavailable::new(
            UnavailableReason::BadResponse,
            format!("judge did not return a noul answer for '{id}'"),
        ))),
    }
}

#[cfg(feature = "jev")]
fn choice_of(
    reply: &JevReply,
    id: &str,
) -> Result<(String, BTreeMap<String, f64>, f64), ToolError> {
    match reply.answers.get(id) {
        Some(JevAnswer::Choice {
            choice,
            probabilities,
            confidence,
        }) => Ok((choice.clone(), probabilities.clone(), *confidence)),
        _ => Err(ToolError::Unavailable(AssistUnavailable::new(
            UnavailableReason::BadResponse,
            format!("judge did not return a choice answer for '{id}'"),
        ))),
    }
}

#[cfg(feature = "jev")]
fn noul_question(instructions: String) -> JevQuestion {
    JevQuestion::Noul {
        instructions,
        criteria: None,
    }
}

/// `assist.experiment_check`: is the design falsifiable and the prediction
/// specific and measurable, with concrete fixes for the weak half.
#[cfg(feature = "jev")]
async fn experiment_check(
    args: &Value,
    session: &Session,
    judge: Option<&dyn Judge>,
) -> Result<Value, ToolError> {
    let judge = require_judge(judge)?;
    let experiment_id = required_str(args, "experiment_id")?;
    let standing = session.standing();
    let experiment = standing
        .experiments
        .iter()
        .find(|experiment| experiment.id == experiment_id)
        .ok_or_else(|| ToolError::BadRequest(format!("unknown experiment: {experiment_id}")))?;
    let hypothesis = standing
        .hypotheses
        .iter()
        .find(|hypothesis| hypothesis.id == experiment.hypothesis_id);

    let falsifiable_instructions = format!(
        "Hypothesis: {}. Experiment design: {}. Is the design falsifiable — would some \
         observable outcome refute the hypothesis?",
        hypothesis
            .map(|h| h.statement.as_str())
            .unwrap_or("<unknown>"),
        experiment.design
    );
    let measurable_instructions = format!(
        "Hypothesis: {}. Experiment prediction: {}. Is the prediction specific and \
         measurable (named quantity, threshold and units)?",
        hypothesis
            .map(|h| h.statement.as_str())
            .unwrap_or("<unknown>"),
        experiment.prediction
    );

    let mut questions = BTreeMap::new();
    questions.insert(
        "design_falsifiable".to_string(),
        noul_question(falsifiable_instructions.clone()),
    );
    questions.insert(
        "prediction_measurable".to_string(),
        noul_question(measurable_instructions.clone()),
    );
    if questions.len() > MAX_QUESTIONS_PER_CALL {
        return Err(ToolError::Unavailable(AssistUnavailable::new(
            UnavailableReason::BadConfig,
            "experiment_check exceeded the question limit",
        )));
    }

    let state = json!({
        "experiment": {
            "id": experiment.id,
            "design": experiment.design,
            "prediction": experiment.prediction,
        },
        "hypothesis": hypothesis.map(|h| json!({ "id": h.id, "statement": h.statement })),
    });

    let reply = judge
        .ask(state, questions)
        .await
        .map_err(ToolError::Unavailable)?;
    let design_falsifiable = noul_of(&reply, "design_falsifiable")?;
    let prediction_measurable = noul_of(&reply, "prediction_measurable")?;

    let mut fixes: Vec<String> = Vec::new();
    if design_falsifiable < 0.5 {
        fixes.push(
            "Make the design falsifiable: state the observable outcome that would refute the \
             hypothesis, and record it before running the experiment."
                .to_string(),
        );
    }
    if prediction_measurable < 0.5 {
        fixes.push(
            "Quantify the prediction: name the measured quantity, the decision threshold and \
             the units."
                .to_string(),
        );
    }

    Ok(json!({
        "ok": true,
        "tool": EXPERIMENT_CHECK,
        "experiment_id": experiment_id,
        "hypothesis_id": experiment.hypothesis_id,
        "question": {
            "design_falsifiable": falsifiable_instructions,
            "prediction_measurable": measurable_instructions,
        },
        "design_falsifiable": { "probability": design_falsifiable },
        "prediction_measurable": { "probability": prediction_measurable },
        "fixes": fixes,
        "model": reply.model,
        "usage": usage_value(&reply.usage),
        "links": EXPERIMENT_LINKS,
    }))
}

/// `assist.observation_support`: P(supports / refutes / irrelevant) for the
/// observation's hypothesis, flagging disagreement with the caller's flag.
#[cfg(feature = "jev")]
async fn observation_support(
    args: &Value,
    session: &Session,
    judge: Option<&dyn Judge>,
) -> Result<Value, ToolError> {
    let judge = require_judge(judge)?;
    let observation_id = required_str(args, "observation_id")?;
    let standing = session.standing();
    let observation = standing
        .observations
        .iter()
        .find(|observation| observation.id == observation_id)
        .ok_or_else(|| ToolError::BadRequest(format!("unknown observation: {observation_id}")))?;
    let experiment = standing
        .experiments
        .iter()
        .find(|experiment| experiment.id == observation.experiment_id)
        .ok_or_else(|| ToolError::BadRequest("observation references unknown experiment".into()))?;
    let hypothesis = standing
        .hypotheses
        .iter()
        .find(|hypothesis| hypothesis.id == experiment.hypothesis_id)
        .ok_or_else(|| ToolError::BadRequest("experiment references unknown hypothesis".into()))?;

    let instructions = format!(
        "Hypothesis: {}. Experiment design: {}. Prediction: {}. Observation result: {}. \
         Evidence: {}. Does this observation support, refute, or is it irrelevant to the \
         hypothesis? Choose exactly one option.",
        hypothesis.statement,
        experiment.design,
        experiment.prediction,
        observation.result,
        observation.evidence
    );
    let criteria = BTreeMap::from([
        (
            "supports".to_string(),
            "The observation is consistent with the hypothesis and increases belief in it."
                .to_string(),
        ),
        (
            "refutes".to_string(),
            "The observation contradicts the hypothesis or its prediction.".to_string(),
        ),
        (
            "irrelevant".to_string(),
            "The observation neither supports nor refutes the hypothesis.".to_string(),
        ),
    ]);
    let question =
        JevQuestion::choice(instructions.clone(), criteria).map_err(ToolError::Unavailable)?;

    let state = json!({
        "hypothesis": { "id": hypothesis.id, "statement": hypothesis.statement },
        "experiment": {
            "id": experiment.id,
            "design": experiment.design,
            "prediction": experiment.prediction,
        },
        "observation": {
            "id": observation.id,
            "result": observation.result,
            "supports": observation.supports,
            "evidence": observation.evidence,
        },
    });

    let mut questions = BTreeMap::new();
    questions.insert("support".to_string(), question);
    let reply = judge
        .ask(state, questions)
        .await
        .map_err(ToolError::Unavailable)?;
    let (choice, probabilities, confidence) = choice_of(&reply, "support")?;

    let disagrees = (observation.supports && choice == "refutes")
        || (!observation.supports && choice == "supports");

    Ok(json!({
        "ok": true,
        "tool": OBSERVATION_SUPPORT,
        "observation_id": observation_id,
        "hypothesis_id": hypothesis.id,
        "question": instructions,
        "answer": {
            "choice": choice,
            "probabilities": probabilities,
            "confidence": confidence,
        },
        "caller_supports": observation.supports,
        "disagrees": disagrees,
        "model": reply.model,
        "usage": usage_value(&reply.usage),
        "links": OBSERVATION_LINKS,
    }))
}

/// `assist.verdict_check`: deterministic warnings when a verdict is not backed
/// by the observations, plus the judge's confidence in the verdict.
#[cfg(feature = "jev")]
async fn verdict_check(
    args: &Value,
    session: &Session,
    judge: Option<&dyn Judge>,
) -> Result<Value, ToolError> {
    let judge = require_judge(judge)?;
    let hypothesis_id = required_str(args, "hypothesis_id")?;
    let verdict = required_str(args, "verdict")?;
    if !matches!(verdict.as_str(), "Supported" | "Refuted" | "Inconclusive") {
        return Err(ToolError::BadRequest(format!(
            "verdict must be Supported, Refuted or Inconclusive; got {verdict}"
        )));
    }

    let standing = session.standing();
    let hypothesis = standing
        .hypotheses
        .iter()
        .find(|hypothesis| hypothesis.id == hypothesis_id)
        .ok_or_else(|| ToolError::BadRequest(format!("unknown hypothesis: {hypothesis_id}")))?;

    let experiment_ids: BTreeSet<&str> = standing
        .experiments
        .iter()
        .filter(|experiment| experiment.hypothesis_id == hypothesis_id)
        .map(|experiment| experiment.id.as_str())
        .collect();
    let observations: Vec<&crate::Observation> = standing
        .observations
        .iter()
        .filter(|observation| experiment_ids.contains(observation.experiment_id.as_str()))
        .collect();
    let supporting = observations
        .iter()
        .filter(|observation| observation.supports)
        .count();
    let refuting = observations.len() - supporting;

    let mut warnings: Vec<String> = Vec::new();
    match verdict.as_str() {
        "Supported" => {
            if observations.is_empty() {
                warnings.push(
                    "Verdict Supported has zero observations backing it; append an experiment \
                     and at least one supporting observation."
                        .to_string(),
                );
            } else if supporting == 0 {
                warnings.push(format!(
                    "Verdict Supported has no supporting observations ({refuting} refuting)."
                ));
            }
            if supporting > 0 && refuting > 0 {
                warnings.push(format!(
                    "Verdict Supported is contested by {refuting} refuting observation(s)."
                ));
            }
        }
        "Refuted" => {
            if observations.is_empty() {
                warnings.push(
                    "Verdict Refuted has zero observations backing it; append an experiment \
                     and at least one observation."
                        .to_string(),
                );
            } else if refuting == 0 {
                warnings.push(format!(
                    "Verdict Refuted has no refuting observations ({supporting} supporting)."
                ));
            }
        }
        _ => {
            if !observations.is_empty() && (supporting == 0 || refuting == 0) {
                warnings.push(
                    "Verdict Inconclusive was given for one-sided evidence; consider whether \
                     the existing observations actually resolve the hypothesis."
                        .to_string(),
                );
            }
        }
    }

    let instructions = format!(
        "Hypothesis: {}. Stated verdict: {}. Observations: {} supporting, {} refuting. Are the \
         observations sufficient to back the stated verdict?",
        hypothesis.statement, verdict, supporting, refuting
    );
    let mut questions = BTreeMap::new();
    questions.insert("backed".to_string(), noul_question(instructions.clone()));
    let state = json!({
        "hypothesis": { "id": hypothesis.id, "statement": hypothesis.statement },
        "verdict": verdict,
        "observations": {
            "total": observations.len(),
            "supporting": supporting,
            "refuting": refuting,
        },
    });
    let reply = judge
        .ask(state, questions)
        .await
        .map_err(ToolError::Unavailable)?;
    let confidence = noul_of(&reply, "backed")?;

    Ok(json!({
        "ok": true,
        "tool": VERDICT_CHECK,
        "hypothesis_id": hypothesis_id,
        "verdict": verdict,
        "observation_count": observations.len(),
        "supporting": supporting,
        "refuting": refuting,
        "warnings": warnings,
        "confidence": confidence,
        "question": instructions,
        "model": reply.model,
        "usage": usage_value(&reply.usage),
        "links": VERDICT_LINKS,
    }))
}

/// `assist.hypothesis_check`: testability of a candidate statement and
/// deterministic near-duplicates among the existing hypotheses.
#[cfg(feature = "jev")]
async fn hypothesis_check(
    args: &Value,
    session: &Session,
    judge: Option<&dyn Judge>,
) -> Result<Value, ToolError> {
    let judge = require_judge(judge)?;
    let statement = required_str(args, "statement")?;
    let standing = session.standing();

    let instructions = format!(
        "Hypothesis: {}. Is this hypothesis testable and falsifiable by a concrete experiment \
         or observation?",
        statement
    );
    let mut questions = BTreeMap::new();
    questions.insert("testable".to_string(), noul_question(instructions.clone()));
    let state = json!({ "candidate": { "statement": statement } });
    let reply = judge
        .ask(state, questions)
        .await
        .map_err(ToolError::Unavailable)?;
    let testable = noul_of(&reply, "testable")?;

    let candidate = words(&statement);
    let mut near_duplicates: Vec<Value> = standing
        .hypotheses
        .iter()
        .filter_map(|hypothesis| {
            let similarity = jaccard(&candidate, &words(&hypothesis.statement));
            (similarity >= 0.4).then(|| {
                json!({
                    "id": hypothesis.id,
                    "statement": hypothesis.statement,
                    "similarity": similarity,
                })
            })
        })
        .collect();
    near_duplicates.sort_by(|a, b| {
        b["similarity"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["similarity"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(json!({
        "ok": true,
        "tool": HYPOTHESIS_CHECK,
        "statement": statement,
        "testable_probability": testable,
        "near_duplicates": near_duplicates,
        "question": instructions,
        "model": reply.model,
        "usage": usage_value(&reply.usage),
        "links": HYPOTHESIS_LINKS,
    }))
}

#[cfg(feature = "jev")]
fn words(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() > 2)
        .map(str::to_string)
        .collect()
}

#[cfg(feature = "jev")]
fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    intersection / union
}
