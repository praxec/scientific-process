use scientific_process::store::{FilesystemStore, validate_session_id};
use scientific_process::{Event, Hypothesis, Session, Status};

fn hypothesis(id: &str) -> Hypothesis {
    Hypothesis {
        id: id.to_string(),
        statement: format!("statement {id}"),
        status: Status::Open,
    }
}

#[test]
fn appended_event_is_restored_by_replaying_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(dir.path()).unwrap();
    store
        .append("s1", &Event::AddHypothesis(hypothesis("h1")))
        .unwrap();

    let session = Session::from_events(store.replay("s1").unwrap());

    assert_eq!(session.standing().hypotheses.len(), 1);
}

#[test]
fn replay_of_a_never_written_session_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(dir.path()).unwrap();

    assert!(store.replay("s1").unwrap().is_empty());
}

#[test]
fn session_id_with_path_separator_is_rejected() {
    assert!(validate_session_id("a/b").is_err());
}

#[test]
fn session_id_with_parent_traversal_is_rejected() {
    assert!(validate_session_id("..").is_err());
}

#[test]
fn empty_session_id_is_rejected() {
    assert!(validate_session_id("").is_err());
}

#[test]
fn well_formed_session_id_is_accepted() {
    assert!(validate_session_id("session-2026.1").is_ok());
}
