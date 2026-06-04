//! Audit M2: GovernorError/RunError expose Display + std::error::Error.
use std::error::Error;
use taskmesh_contract::*;

#[test]
fn governor_error_is_display_and_error() {
    let e = GovernorError::Rejected(AdmissionVerdict::UnknownClass {
        class: TaskClass::new("x"),
    });
    assert!(e.to_string().contains("unknown class"));
    let _dyn: &dyn Error = &e;
}

#[test]
fn run_error_display_and_source() {
    #[derive(Debug)]
    struct MyErr;
    impl std::fmt::Display for MyErr {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("boom")
        }
    }
    impl Error for MyErr {}

    let task: RunError<MyErr> = RunError::Task(MyErr);
    assert!(task.to_string().contains("boom"));
    assert!(task.source().is_some());

    let gov: RunError<MyErr> = RunError::Governor(GovernorError::LocalRuntimeUnavailable);
    assert!(gov.to_string().contains("local runtime"));
}
