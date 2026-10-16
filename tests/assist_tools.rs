//! Behavioral tests for the four advisory assist tools. Each test drives the
//! public `assist::tools::dispatch` with an in-process fake Judge and asserts
//! exactly one observable outcome. No network, no real provider.

#![cfg(feature = "jev")]

use std::collections::BTreeMap;

use async_trait::async_trait;
use scientific_process::assist::jev::{JevAnswer, JevQuestion, JevReply, Judge};
use scientific_process::assist::tools;
use scientific_process::assist::{AssistUnavailable, Usage};
use scientific_process::{Experiment, Hypothesis, Observation, Session, Status};
use serde_json::{Value, json};

struct NoulJudge(f64);

#[async_trait]
impl Judge for NoulJudge {
    async fn ask(
        &self,
        _state: Value,
        questions: BTreeMap<String, JevQuestion>,
    ) -> Result<JevReply, AssistUnavailable> {
        let answers = questions
            .keys()
            .map(|id| (id.clone(), JevAnswer::Noul { noul: self.0 }))
            .collect();
        Ok(JevReply {
            model: "fake/jev".to_string(),
            answers,
            usage: Usage::default(),
        })
    }
}

struct RefutesJudge;

#[async_trait]
impl Judge for RefutesJudge {
    async fn ask(
        &self,
        _state: Value,
        questions: BTreeMap<String, JevQuestion>,
    ) -> Result<JevReply, AssistUnavailable> {
        let answers = questions
            .keys()
            .map(|id| {
                (
                    id.clone(),
                    JevAnswer::Choice {
                        choice: "refutes".to_string(),
                        probabilities: BTreeMap::from([
                            ("supports".to_string(), 0.1),
                            ("refutes".to_string(), 0.8),
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

fn session_with_observation(supports: bool) -> Session {
    let mut session = Session::new();
    session
        .add_hypothesis(Hypothesis {
            id: "h1".to_string(),
            statement: "water boils at 100C".to_string(),
            status: Status::Open,
        })
        .expect("hypothesis");
    session
        .add_experiment(Experiment {
            id: "e1".to_string(),
            hypothesis_id: "h1".to_string(),
            design: "boil water and read the thermometer".to_string(),
            prediction: "temperature is 100C".to_string(),
        })
        .expect("experiment");
    session
        .add_observation(Observation {
            id: "o1".to_string(),
            experiment_id: "e1".to_string(),
            result: "measured 80C".to_string(),
            supports,
            evidence: "thermometer reading".to_string(),
        })
        .expect("observation");
    session
}

fn session_with_hypothesis_only() -> Session {
    let mut session = Session::new();
    session
        .add_hypothesis(Hypothesis {
            id: "h1".to_string(),
            statement: "water boils at 100C at sea level".to_string(),
            status: Status::Open,
        })
        .expect("hypothesis");
    session
}

#[tokio::test]
async fn observation_support_reports_refutes() {
    let session = session_with_observation(true);
    let output = tools::dispatch(
        tools::OBSERVATION_SUPPORT,
        &json!({"observation_id": "o1"}),
        &session,
        Some(&RefutesJudge),
    )
    .await
    .expect("assist tool");

    assert_eq!(output["answer"]["choice"], "refutes");
}

#[tokio::test]
async fn observation_support_reports_the_refutes_probability() {
    let session = session_with_observation(true);
    let output = tools::dispatch(
        tools::OBSERVATION_SUPPORT,
        &json!({"observation_id": "o1"}),
        &session,
        Some(&RefutesJudge),
    )
    .await
    .expect("assist tool");

    assert_eq!(output["answer"]["probabilities"]["refutes"], 0.8);
}

#[tokio::test]
async fn observation_support_flags_disagreement_when_caller_supports() {
    let session = session_with_observation(true);
    let output = tools::dispatch(
        tools::OBSERVATION_SUPPORT,
        &json!({"observation_id": "o1"}),
        &session,
        Some(&RefutesJudge),
    )
    .await
    .expect("assist tool");

    assert_eq!(output["disagrees"], true);
}

#[tokio::test]
async fn observation_support_returns_the_question_text_sent() {
    let session = session_with_observation(true);
    let output = tools::dispatch(
        tools::OBSERVATION_SUPPORT,
        &json!({"observation_id": "o1"}),
        &session,
        Some(&RefutesJudge),
    )
    .await
    .expect("assist tool");

    assert!(
        output["question"]
            .as_str()
            .is_some_and(|question| question.contains("Observation result"))
    );
}

#[tokio::test]
async fn verdict_check_warns_on_supported_with_zero_observations() {
    let session = session_with_hypothesis_only();
    let output = tools::dispatch(
        tools::VERDICT_CHECK,
        &json!({"hypothesis_id": "h1", "verdict": "Supported"}),
        &session,
        Some(&NoulJudge(0.2)),
    )
    .await
    .expect("assist tool");

    assert!(
        output["warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty())
    );
}

#[tokio::test]
async fn experiment_check_reports_the_design_probability() {
    let session = session_with_observation(true);
    let output = tools::dispatch(
        tools::EXPERIMENT_CHECK,
        &json!({"experiment_id": "e1"}),
        &session,
        Some(&NoulJudge(0.2)),
    )
    .await
    .expect("assist tool");

    assert_eq!(output["design_falsifiable"]["probability"], 0.2);
}

#[tokio::test]
async fn experiment_check_without_a_judge_is_assist_unavailable() {
    let session = session_with_observation(true);
    let output = tools::dispatch(
        tools::EXPERIMENT_CHECK,
        &json!({"experiment_id": "e1"}),
        &session,
        None,
    )
    .await
    .expect("assist tool");

    assert_eq!(output["error"], "ASSIST_UNAVAILABLE");
}

#[tokio::test]
async fn hypothesis_check_reports_a_near_duplicate() {
    let session = session_with_hypothesis_only();
    let output = tools::dispatch(
        tools::HYPOTHESIS_CHECK,
        &json!({"statement": "water boils at 100C"}),
        &session,
        Some(&NoulJudge(0.9)),
    )
    .await
    .expect("assist tool");

    assert!(
        output["near_duplicates"]
            .as_array()
            .is_some_and(|duplicates| !duplicates.is_empty())
    );
}

#[tokio::test]
async fn assist_tools_leave_the_event_log_unchanged() {
    let session = session_with_observation(true);
    let before = serde_json::to_value(session.events()).expect("serialize events");
    for (name, args) in [
        (tools::EXPERIMENT_CHECK, json!({"experiment_id": "e1"})),
        (tools::OBSERVATION_SUPPORT, json!({"observation_id": "o1"})),
        (
            tools::VERDICT_CHECK,
            json!({"hypothesis_id": "h1", "verdict": "Supported"}),
        ),
        (
            tools::HYPOTHESIS_CHECK,
            json!({"statement": "a new candidate"}),
        ),
    ] {
        let _ = tools::dispatch(name, &args, &session, Some(&NoulJudge(0.5))).await;
    }
    let after = serde_json::to_value(session.events()).expect("serialize events");

    assert_eq!(before, after);
}
