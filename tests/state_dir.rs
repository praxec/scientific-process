use scientific_process::store::{STATE_DIR_ENV, resolve_state_dir};

#[test]
fn scientific_process_state_dir_env_selects_state_directory() {
    let chosen = std::env::temp_dir().join("sp-state-dir-env-test");
    // Safety: this integration test binary contains only this test, so no
    // other thread reads the environment concurrently.
    unsafe { std::env::set_var(STATE_DIR_ENV, &chosen) };

    assert_eq!(resolve_state_dir(), chosen);
}
