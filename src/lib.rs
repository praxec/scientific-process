use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── error strings ───────────────────────────────────────────────────

pub const ERR_DUPLICATE_ID: &str = "DUPLICATE_ID";
pub const ERR_HYPOTHESIS_NOT_FOUND: &str = "HYPOTHESIS_NOT_FOUND";
pub const ERR_EXPERIMENT_NOT_FOUND: &str = "EXPERIMENT_NOT_FOUND";

// ── types ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Open,
    Supported,
    Refuted,
    Inconclusive,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Hypothesis {
    pub id: String,
    pub statement: String,
    pub status: Status,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Experiment {
    pub id: String,
    pub hypothesis_id: String,
    pub design: String,
    pub prediction: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Observation {
    pub id: String,
    pub experiment_id: String,
    pub result: String,
    pub supports: bool,
    pub evidence: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Supported,
    Refuted,
    Inconclusive,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Conclusion {
    pub hypothesis_id: String,
    pub verdict: Verdict,
    pub rationale: String,
}

// ── events (append-only log) ────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum Event {
    AddHypothesis(Hypothesis),
    AddExperiment(Experiment),
    AddObservation(Observation),
    Conclude(Conclusion),
}

// ── session ─────────────────────────────────────────────────────────

pub struct Session {
    events: Vec<Event>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    // ── mutation (append-only, validated) ────────────────────────

    pub fn add_hypothesis(&mut self, h: Hypothesis) -> Result<(), String> {
        if self.has_hypothesis_id(&h.id) {
            return Err(ERR_DUPLICATE_ID.to_string());
        }
        self.events.push(Event::AddHypothesis(h));
        Ok(())
    }

    pub fn add_experiment(&mut self, e: Experiment) -> Result<(), String> {
        if self.has_experiment_id(&e.id) {
            return Err(ERR_DUPLICATE_ID.to_string());
        }
        if !self.has_hypothesis_id(&e.hypothesis_id) {
            return Err(ERR_HYPOTHESIS_NOT_FOUND.to_string());
        }
        self.events.push(Event::AddExperiment(e));
        Ok(())
    }

    pub fn add_observation(&mut self, o: Observation) -> Result<(), String> {
        if self.has_observation_id(&o.id) {
            return Err(ERR_DUPLICATE_ID.to_string());
        }
        if !self.has_experiment_id(&o.experiment_id) {
            return Err(ERR_EXPERIMENT_NOT_FOUND.to_string());
        }
        self.events.push(Event::AddObservation(o));
        Ok(())
    }

    pub fn conclude(&mut self, c: Conclusion) -> Result<(), String> {
        if !self.has_hypothesis_id(&c.hypothesis_id) {
            return Err(ERR_HYPOTHESIS_NOT_FOUND.to_string());
        }
        self.events.push(Event::Conclude(c));
        Ok(())
    }

    // ── id-existence helpers ────────────────────────────────────

    fn has_hypothesis_id(&self, id: &str) -> bool {
        self.events
            .iter()
            .any(|ev| matches!(ev, Event::AddHypothesis(h) if h.id == id))
    }

    fn has_experiment_id(&self, id: &str) -> bool {
        self.events
            .iter()
            .any(|ev| matches!(ev, Event::AddExperiment(e) if e.id == id))
    }

    fn has_observation_id(&self, id: &str) -> bool {
        self.events
            .iter()
            .any(|ev| matches!(ev, Event::AddObservation(o) if o.id == id))
    }

    // ── queries (pure fold over the log) ────────────────────────

    pub fn standing(&self) -> Standing {
        let mut hypotheses_map: HashMap<String, Hypothesis> = HashMap::new();
        let mut experiments: Vec<Experiment> = Vec::new();
        let mut observations: Vec<Observation> = Vec::new();

        for event in &self.events {
            match event {
                Event::AddHypothesis(h) => {
                    hypotheses_map.insert(h.id.clone(), h.clone());
                }
                Event::AddExperiment(e) => {
                    experiments.push(e.clone());
                }
                Event::AddObservation(o) => {
                    observations.push(o.clone());
                }
                Event::Conclude(c) => {
                    if let Some(h) = hypotheses_map.get_mut(&c.hypothesis_id) {
                        h.status = match c.verdict {
                            Verdict::Supported => Status::Supported,
                            Verdict::Refuted => Status::Refuted,
                            Verdict::Inconclusive => Status::Inconclusive,
                        };
                    }
                }
            }
        }

        let mut hypotheses: Vec<Hypothesis> = hypotheses_map.into_values().collect();
        hypotheses.sort_by(|a, b| a.id.cmp(&b.id));

        Standing {
            hypotheses,
            experiments,
            observations,
        }
    }

    /// Returns the open hypothesis with the fewest observations
    /// (highest leverage for next experiment). Returns `None` when
    /// no open hypothesis remains.
    pub fn next_experiment(&self) -> Option<Hypothesis> {
        let standing = self.standing();

        // Build experiment-id → hypothesis-id lookup
        let exp_to_hyp: HashMap<&str, &str> = standing
            .experiments
            .iter()
            .map(|e| (e.id.as_str(), e.hypothesis_id.as_str()))
            .collect();

        // Count observations per hypothesis
        let mut obs_count: HashMap<&str, usize> = HashMap::new();
        for o in &standing.observations {
            if let Some(hid) = exp_to_hyp.get(o.experiment_id.as_str()) {
                *obs_count.entry(hid).or_insert(0) += 1;
            }
        }

        standing
            .hypotheses
            .iter()
            .filter(|h| h.status == Status::Open)
            .min_by_key(|h| obs_count.get(h.id.as_str()).copied().unwrap_or(0))
            .cloned()
    }

    /// Export a structured report: each hypothesis with its
    /// experiments, observations, and verdict.
    pub fn export_report(&self) -> Report {
        let standing = self.standing();

        let mut hypothesis_reports: Vec<HypothesisReport> = Vec::new();

        for hyp in &standing.hypotheses {
            let exps: Vec<&Experiment> = standing
                .experiments
                .iter()
                .filter(|e| e.hypothesis_id == hyp.id)
                .collect();

            let mut exp_reports: Vec<ExperimentReport> = Vec::new();

            for exp in &exps {
                let obs: Vec<&Observation> = standing
                    .observations
                    .iter()
                    .filter(|o| o.experiment_id == exp.id)
                    .collect();

                let supporting = obs.iter().filter(|o| o.supports).count();
                let refuting = obs.iter().filter(|o| !o.supports).count();

                exp_reports.push(ExperimentReport {
                    experiment_id: exp.id.clone(),
                    design: exp.design.clone(),
                    prediction: exp.prediction.clone(),
                    observation_count: obs.len(),
                    supporting_observations: supporting,
                    refuting_observations: refuting,
                    observations: obs
                        .iter()
                        .map(|o| ObservationSummary {
                            id: o.id.clone(),
                            result: o.result.clone(),
                            supports: o.supports,
                            evidence: o.evidence.clone(),
                        })
                        .collect(),
                });
            }

            hypothesis_reports.push(HypothesisReport {
                hypothesis_id: hyp.id.clone(),
                statement: hyp.statement.clone(),
                status: hyp.status.clone(),
                experiments: exp_reports,
            });
        }

        Report {
            hypotheses: hypothesis_reports,
        }
    }
}

// ── standing (computed projection) ──────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Standing {
    pub hypotheses: Vec<Hypothesis>,
    pub experiments: Vec<Experiment>,
    pub observations: Vec<Observation>,
}

// ── report (structured export) ──────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Report {
    pub hypotheses: Vec<HypothesisReport>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HypothesisReport {
    pub hypothesis_id: String,
    pub statement: String,
    pub status: Status,
    pub experiments: Vec<ExperimentReport>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExperimentReport {
    pub experiment_id: String,
    pub design: String,
    pub prediction: String,
    pub observation_count: usize,
    pub supporting_observations: usize,
    pub refuting_observations: usize,
    pub observations: Vec<ObservationSummary>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ObservationSummary {
    pub id: String,
    pub result: String,
    pub supports: bool,
    pub evidence: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─ helpers ──────────────────────────────────────────────────

    fn h(id: &str, statement: &str) -> Hypothesis {
        Hypothesis {
            id: id.to_string(),
            statement: statement.to_string(),
            status: Status::Open,
        }
    }

    fn e(id: &str, hyp_id: &str) -> Experiment {
        Experiment {
            id: id.to_string(),
            hypothesis_id: hyp_id.to_string(),
            design: format!("design-{id}"),
            prediction: format!("prediction-{id}"),
        }
    }

    fn o(id: &str, exp_id: &str, supports: bool) -> Observation {
        Observation {
            id: id.to_string(),
            experiment_id: exp_id.to_string(),
            result: format!("result-{id}"),
            supports,
            evidence: format!("evidence-{id}"),
        }
    }

    fn c(hyp_id: &str, verdict: Verdict) -> Conclusion {
        Conclusion {
            hypothesis_id: hyp_id.to_string(),
            verdict,
            rationale: format!("rationale-{hyp_id}"),
        }
    }

    // ─ 1. empty session ─────────────────────────────────────────

    #[test]
    fn empty_session_has_empty_standing() {
        let s = Session::new();
        let st = s.standing();
        assert!(st.hypotheses.is_empty());
        assert!(st.experiments.is_empty());
        assert!(st.observations.is_empty());
    }

    #[test]
    fn empty_session_next_experiment_is_none() {
        let s = Session::new();
        assert!(s.next_experiment().is_none());
    }

    #[test]
    fn empty_session_report_is_empty() {
        let s = Session::new();
        let r = s.export_report();
        assert!(r.hypotheses.is_empty());
    }

    // ─ 2. add hypothesis + standing fold ────────────────────────

    #[test]
    fn add_hypothesis_appears_in_standing() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "water boils at 100C")).unwrap();
        let st = s.standing();
        assert_eq!(st.hypotheses.len(), 1);
        assert_eq!(st.hypotheses[0].id, "h1");
        assert_eq!(st.hypotheses[0].status, Status::Open);
    }

    #[test]
    fn add_two_hypotheses_appear_sorted() {
        let mut s = Session::new();
        s.add_hypothesis(h("h2", "second")).unwrap();
        s.add_hypothesis(h("h1", "first")).unwrap();
        let st = s.standing();
        assert_eq!(st.hypotheses.len(), 2);
        assert_eq!(st.hypotheses[0].id, "h1");
        assert_eq!(st.hypotheses[1].id, "h2");
    }

    // ─ 3. duplicate rejection ───────────────────────────────────

    #[test]
    fn reject_duplicate_hypothesis_id() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        let err = s.add_hypothesis(h("h1", "b")).unwrap_err();
        assert!(err.contains(ERR_DUPLICATE_ID));
    }

    #[test]
    fn reject_duplicate_experiment_id() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        let err = s.add_experiment(e("e1", "h1")).unwrap_err();
        assert!(err.contains(ERR_DUPLICATE_ID));
    }

    #[test]
    fn reject_duplicate_observation_id() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        let err = s.add_observation(o("o1", "e1", false)).unwrap_err();
        assert!(err.contains(ERR_DUPLICATE_ID));
    }

    // ─ 4. unknown-reference rejection ───────────────────────────

    #[test]
    fn reject_experiment_with_unknown_hypothesis() {
        let mut s = Session::new();
        let err = s.add_experiment(e("e1", "nonexistent")).unwrap_err();
        assert!(err.contains(ERR_HYPOTHESIS_NOT_FOUND));
    }

    #[test]
    fn reject_observation_with_unknown_experiment() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        let err = s.add_observation(o("o1", "nonexistent", true)).unwrap_err();
        assert!(err.contains(ERR_EXPERIMENT_NOT_FOUND));
    }

    #[test]
    fn reject_conclusion_with_unknown_hypothesis() {
        let mut s = Session::new();
        let err = s
            .conclude(c("nonexistent", Verdict::Supported))
            .unwrap_err();
        assert!(err.contains(ERR_HYPOTHESIS_NOT_FOUND));
    }

    // ─ 5. conclusion derives status ─────────────────────────────

    #[test]
    fn conclude_supported_sets_status() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        s.conclude(c("h1", Verdict::Supported)).unwrap();
        let st = s.standing();
        assert_eq!(st.hypotheses[0].status, Status::Supported);
    }

    #[test]
    fn conclude_refuted_sets_status() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        s.conclude(c("h1", Verdict::Refuted)).unwrap();
        let st = s.standing();
        assert_eq!(st.hypotheses[0].status, Status::Refuted);
    }

    #[test]
    fn conclude_inconclusive_sets_status() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        s.conclude(c("h1", Verdict::Inconclusive)).unwrap();
        let st = s.standing();
        assert_eq!(st.hypotheses[0].status, Status::Inconclusive);
    }

    // ─ 6. next_experiment selection ────────────────────────────

    #[test]
    fn next_experiment_returns_open_hypothesis() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        let next = s.next_experiment().unwrap();
        assert_eq!(next.id, "h1");
    }

    #[test]
    fn next_experiment_none_when_all_concluded() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        s.conclude(c("h1", Verdict::Supported)).unwrap();
        assert!(s.next_experiment().is_none());
    }

    #[test]
    fn next_experiment_picks_fewest_observations() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "first")).unwrap();
        s.add_hypothesis(h("h2", "second")).unwrap();
        // h1 has 2 observations via e1
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        s.add_observation(o("o2", "e1", false)).unwrap();
        // h2 has 1 observation via e2
        s.add_experiment(e("e2", "h2")).unwrap();
        s.add_observation(o("o3", "e2", true)).unwrap();
        // h2 has fewer observations → should be chosen
        let next = s.next_experiment().unwrap();
        assert_eq!(next.id, "h2");
    }

    #[test]
    fn next_experiment_zero_obs_beats_one() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "has-obs")).unwrap();
        s.add_hypothesis(h("h2", "no-obs")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        // h2 has zero observations
        let next = s.next_experiment().unwrap();
        assert_eq!(next.id, "h2");
    }

    #[test]
    fn next_experiment_tie_returns_first_by_id() {
        let mut s = Session::new();
        s.add_hypothesis(h("hB", "b")).unwrap();
        s.add_hypothesis(h("hA", "a")).unwrap();
        // Both have zero observations; standing sorts by id (hA, hB)
        let next = s.next_experiment().unwrap();
        assert_eq!(next.id, "hA");
    }

    // ─ 7. experiment / observation in standing ──────────────────

    #[test]
    fn experiment_appears_in_standing() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        let st = s.standing();
        assert_eq!(st.experiments.len(), 1);
        assert_eq!(st.experiments[0].id, "e1");
        assert_eq!(st.experiments[0].hypothesis_id, "h1");
    }

    #[test]
    fn observation_appears_in_standing() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "a")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        let st = s.standing();
        assert_eq!(st.observations.len(), 1);
        assert_eq!(st.observations[0].id, "o1");
        assert!(st.observations[0].supports);
    }

    // ─ 8. interleaved events ────────────────────────────────────

    #[test]
    fn interleaved_events_produce_correct_standing() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "first")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        s.add_hypothesis(h("h2", "second")).unwrap();
        s.add_experiment(e("e2", "h2")).unwrap();
        s.add_observation(o("o2", "e2", false)).unwrap();
        s.add_observation(o("o3", "e1", false)).unwrap();
        s.conclude(c("h1", Verdict::Refuted)).unwrap();

        let st = s.standing();
        assert_eq!(st.hypotheses.len(), 2);
        assert_eq!(st.experiments.len(), 2);
        assert_eq!(st.observations.len(), 3);

        let h1 = st.hypotheses.iter().find(|x| x.id == "h1").unwrap();
        assert_eq!(h1.status, Status::Refuted);

        let h2 = st.hypotheses.iter().find(|x| x.id == "h2").unwrap();
        assert_eq!(h2.status, Status::Open);
    }

    // ─ 9. report export ─────────────────────────────────────────

    #[test]
    fn report_includes_hypothesis_with_status() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        s.conclude(c("h1", Verdict::Supported)).unwrap();
        let r = s.export_report();
        assert_eq!(r.hypotheses.len(), 1);
        assert_eq!(r.hypotheses[0].hypothesis_id, "h1");
        assert_eq!(r.hypotheses[0].status, Status::Supported);
    }

    #[test]
    fn report_includes_experiments_and_observations() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        s.add_observation(o("o2", "e1", false)).unwrap();
        let r = s.export_report();
        let hr = &r.hypotheses[0];
        assert_eq!(hr.experiments.len(), 1);
        let er = &hr.experiments[0];
        assert_eq!(er.experiment_id, "e1");
        assert_eq!(er.observation_count, 2);
        assert_eq!(er.supporting_observations, 1);
        assert_eq!(er.refuting_observations, 1);
        assert_eq!(er.observations.len(), 2);
    }

    #[test]
    fn report_observation_summary_includes_details() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        let r = s.export_report();
        let obs = &r.hypotheses[0].experiments[0].observations[0];
        assert_eq!(obs.id, "o1");
        assert_eq!(obs.result, "result-o1");
        assert!(obs.supports);
        assert_eq!(obs.evidence, "evidence-o1");
    }

    // ─ 10. full workflow integration test ───────────────────────

    #[test]
    fn full_scientific_workflow() {
        let mut s = Session::new();

        // Open with two hypotheses
        s.add_hypothesis(h("h1", "water boils at 100C at sea level"))
            .unwrap();
        s.add_hypothesis(h("h2", "salt water boils above 100C"))
            .unwrap();

        // h1 has an experiment
        s.add_experiment(Experiment {
            id: "e1".into(),
            hypothesis_id: "h1".into(),
            design: "Boil pure water, measure temperature".into(),
            prediction: "Temperature will be 100C".into(),
        })
        .unwrap();
        s.add_observation(Observation {
            id: "o1".into(),
            experiment_id: "e1".into(),
            result: "Measured 99.8C".into(),
            supports: true,
            evidence: "Thermometer reading within error margin".into(),
        })
        .unwrap();

        // Conclude h1
        s.conclude(Conclusion {
            hypothesis_id: "h1".into(),
            verdict: Verdict::Supported,
            rationale: "Observation matches prediction within measurement error".into(),
        })
        .unwrap();

        // h1 should now be Supported, h2 still Open
        let st = s.standing();
        let h1 = st.hypotheses.iter().find(|x| x.id == "h1").unwrap();
        let h2 = st.hypotheses.iter().find(|x| x.id == "h2").unwrap();
        assert_eq!(h1.status, Status::Supported);
        assert_eq!(h2.status, Status::Open);

        // next_experiment should be h2
        let next = s.next_experiment().unwrap();
        assert_eq!(next.id, "h2");

        // Report should have both
        let r = s.export_report();
        assert_eq!(r.hypotheses.len(), 2);
    }

    // ─ 11. refutation / no-conclusion status preservation ───────

    #[test]
    fn refuting_observation_does_not_change_status_without_conclusion() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", false)).unwrap();
        // Without a conclusion, status stays Open
        let st = s.standing();
        assert_eq!(st.hypotheses[0].status, Status::Open);
    }

    #[test]
    fn multiple_experiments_per_hypothesis() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_experiment(e("e2", "h1")).unwrap();
        let st = s.standing();
        assert_eq!(st.experiments.len(), 2);
    }

    #[test]
    fn observation_without_experiment_rejected() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        // No experiment added → observation should be rejected
        let err = s.add_observation(o("o1", "e99", true)).unwrap_err();
        assert!(err.contains(ERR_EXPERIMENT_NOT_FOUND));
    }

    // ─ 12. serde round-trip ─────────────────────────────────────

    #[test]
    fn hypothesis_serde_roundtrip() {
        let hyp = h("h1", "test statement");
        let json = serde_json::to_string(&hyp).unwrap();
        let back: Hypothesis = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, hyp.id);
        assert_eq!(back.statement, hyp.statement);
        assert_eq!(back.status, hyp.status);
    }

    #[test]
    fn standing_serde_roundtrip() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        let st = s.standing();
        let json = serde_json::to_string(&st).unwrap();
        let back: Standing = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hypotheses.len(), 1);
        assert_eq!(back.experiments.len(), 1);
        assert_eq!(back.observations.len(), 1);
    }

    #[test]
    fn report_serde_roundtrip() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        s.add_experiment(e("e1", "h1")).unwrap();
        s.add_observation(o("o1", "e1", true)).unwrap();
        s.conclude(c("h1", Verdict::Supported)).unwrap();
        let r = s.export_report();
        let json = serde_json::to_string(&r).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hypotheses.len(), 1);
        assert_eq!(back.hypotheses[0].status, Status::Supported);
    }

    // ─ 13. conclusion overrides prior conclusion ─────────────────

    #[test]
    fn second_conclusion_overrides_first() {
        let mut s = Session::new();
        s.add_hypothesis(h("h1", "test")).unwrap();
        s.conclude(c("h1", Verdict::Supported)).unwrap();
        s.conclude(c("h1", Verdict::Refuted)).unwrap();
        let st = s.standing();
        // Last conclusion wins in fold order
        assert_eq!(st.hypotheses[0].status, Status::Refuted);
    }
}
