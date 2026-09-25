//! SEP21-C01 adversarial tests for the raw/validated task-plan boundary.

use taskmesh_contract::*;

fn root() -> TaskSpec {
    TaskSpec::io(TaskClass::new("worker")).operation("root-1")
}

fn assert_invalid(raw: TaskSpec, expected: TaskPlanError) {
    assert_eq!(raw.validate_borrowed(), Err(expected.clone()));
    assert_eq!(raw.validate(), Err(expected.clone()));
    assert_eq!(ValidatedTaskPlan::try_from(raw), Err(expected));
}

#[test]
fn valid_child_preserves_immediate_parent_identity() {
    let raw = TaskSpec::cpu(TaskClass::new("worker"))
        .awaited_child_of("root-1", "parent-2", TaskStage::new("fanout"))
        .operation("child-3");
    assert_eq!(raw.validate_borrowed(), Ok(()));
    let plan = raw.validate().expect("complete lineage is admissible");
    assert_eq!(
        plan.as_spec().scope,
        TaskScope::Child {
            parent_operation_id: "parent-2".to_string(),
            parent_stage: TaskStage::new("fanout"),
            parent_awaits: true,
        }
    );
}

#[test]
fn parent_stage_membership_is_explicitly_outside_a_child_plan() {
    let child = TaskSpec::cpu(TaskClass::new("worker"))
        .awaited_child_of("root-1", "parent-2", TaskStage::new("not-in-child-plan"))
        .operation("child-3")
        .validate()
        .expect("the child plan validates identity shape, not a separate parent plan");
    assert!(matches!(
        &child.as_spec().scope,
        TaskScope::Child { parent_stage, .. } if parent_stage.as_str() == "not-in-child-plan"
    ));
}

#[test]
fn zero_stage_is_rejected() {
    let mut raw = root();
    raw.stages.clear();
    assert_invalid(raw, TaskPlanError::NoStages);
}

#[test]
fn duplicate_identical_stage_is_not_silently_deduplicated() {
    let mut raw = root();
    raw.stages.push(raw.stages[0].clone());
    assert_invalid(
        raw,
        TaskPlanError::DuplicateStage {
            stage: TaskStage::new("io"),
        },
    );
}

#[test]
fn duplicate_name_with_different_descriptor_is_conflicting() {
    let raw = root().stage(TaskStage::new("io"), SubstrateHint::BlockingPool);
    assert_invalid(
        raw,
        TaskPlanError::ConflictingStage {
            stage: TaskStage::new("io"),
        },
    );
}

#[test]
fn class_mismatch_is_rejected_by_the_same_validator() {
    let mut raw = root();
    raw.stages[0].class = TaskClass::new("other");
    assert_invalid(
        raw,
        TaskPlanError::StageClassMismatch {
            stage: TaskStage::new("io"),
        },
    );
}

#[test]
fn incomplete_and_spurious_reduce_contracts_are_rejected() {
    assert_invalid(
        root().fan_out_stage(TaskStage::new("fanout"), SubstrateHint::SharedCpuExecutor),
        TaskPlanError::MissingReducePolicy {
            stage: TaskStage::new("fanout"),
        },
    );
    let mut raw = root();
    raw.stages[0].reduce_policy = Some(DeterministicReducePolicy::keyed("id"));
    assert_invalid(
        raw,
        TaskPlanError::UnexpectedReducePolicy {
            stage: TaskStage::new("io"),
        },
    );
}

#[test]
fn identifiers_reject_empty_whitespace_oversize_and_noncanonical_characters() {
    let mut empty_class = root();
    empty_class.class = TaskClass::new("");
    assert_invalid(
        empty_class,
        TaskPlanError::InvalidIdentifier {
            field: TaskIdentifierField::Class,
            violation: IdentifierViolation::Empty,
        },
    );

    let mut padded_operation = root();
    padded_operation.operation = " root-1".to_string();
    assert_invalid(
        padded_operation,
        TaskPlanError::InvalidIdentifier {
            field: TaskIdentifierField::Operation,
            violation: IdentifierViolation::SurroundingWhitespace,
        },
    );

    let mut oversize_stage = root();
    oversize_stage.stages[0].stage = TaskStage::new("x".repeat(MAX_TASK_IDENTIFIER_LEN + 1));
    assert_invalid(
        oversize_stage,
        TaskPlanError::InvalidIdentifier {
            field: TaskIdentifierField::Stage { index: 0 },
            violation: IdentifierViolation::TooLong {
                actual: MAX_TASK_IDENTIFIER_LEN + 1,
                max: MAX_TASK_IDENTIFIER_LEN,
            },
        },
    );

    let mut bad_stage = root();
    bad_stage.stages[0].stage = TaskStage::new("io?");
    assert_invalid(
        bad_stage,
        TaskPlanError::InvalidIdentifier {
            field: TaskIdentifierField::Stage { index: 0 },
            violation: IdentifierViolation::InvalidCharacter {
                byte_offset: 2,
                character: '?',
            },
        },
    );
}

#[test]
fn identifier_maximum_is_inclusive_and_field_display_is_exact() {
    let below_maximum = "x".repeat(MAX_TASK_IDENTIFIER_LEN - 1);
    assert_eq!(
        PlanSource::new(below_maximum)
            .expect("one byte below the maximum is valid")
            .as_str()
            .len(),
        MAX_TASK_IDENTIFIER_LEN - 1
    );
    let maximum = "x".repeat(MAX_TASK_IDENTIFIER_LEN);
    assert_eq!(
        PlanSource::new(maximum)
            .expect("maximum is valid")
            .as_str()
            .len(),
        MAX_TASK_IDENTIFIER_LEN
    );
    assert_eq!(TaskIdentifierField::PlanSource.to_string(), "source");
    assert_eq!(TaskIdentifierField::Class.to_string(), "class");
    assert_eq!(TaskIdentifierField::Operation.to_string(), "operation");
    assert_eq!(
        TaskIdentifierField::RootOperationId.to_string(),
        "root_operation_id"
    );
    assert_eq!(
        TaskIdentifierField::ParentOperationId.to_string(),
        "parent_operation_id"
    );
    assert_eq!(TaskIdentifierField::ParentStage.to_string(), "parent_stage");
    assert_eq!(
        TaskIdentifierField::Stage { index: 7 }.to_string(),
        "stages[7].stage"
    );
    assert_eq!(
        TaskIdentifierField::ReduceKey { index: 9 }.to_string(),
        "stages[9].reduce_policy.stable_sort_key"
    );
}

#[test]
fn class_identifiers_reject_nul_non_ascii_and_over_limit_values() {
    for (value, violation) in [
        (
            "nul\0class".to_owned(),
            IdentifierViolation::InvalidCharacter {
                byte_offset: 3,
                character: '\0',
            },
        ),
        (
            "café".to_owned(),
            IdentifierViolation::InvalidCharacter {
                byte_offset: 3,
                character: 'é',
            },
        ),
        (
            "x".repeat(MAX_TASK_IDENTIFIER_LEN + 1),
            IdentifierViolation::TooLong {
                actual: MAX_TASK_IDENTIFIER_LEN + 1,
                max: MAX_TASK_IDENTIFIER_LEN,
            },
        ),
    ] {
        let mut raw = root();
        raw.class = TaskClass::new(value);
        assert_invalid(
            raw,
            TaskPlanError::InvalidIdentifier {
                field: TaskIdentifierField::Class,
                violation,
            },
        );
    }
}

#[test]
fn reduce_key_is_validated_as_an_identifier() {
    let raw = root().reduce_stage(
        TaskStage::new("merge"),
        SubstrateHint::SharedCpuExecutor,
        DeterministicReducePolicy::keyed(""),
    );
    assert_invalid(
        raw,
        TaskPlanError::InvalidIdentifier {
            field: TaskIdentifierField::ReduceKey { index: 1 },
            violation: IdentifierViolation::Empty,
        },
    );
}

#[test]
fn root_and_parent_identity_ambiguity_is_rejected() {
    let mut root_mismatch = root();
    root_mismatch.root_operation_id = "other-root".to_string();
    assert_invalid(root_mismatch, TaskPlanError::RootIdentityMismatch);

    let child_is_root = TaskSpec::io(TaskClass::new("worker"))
        .child_of("root-1", "root-1", TaskStage::new("fanout"))
        .operation("root-1");
    assert_invalid(child_is_root, TaskPlanError::ChildOperationMatchesRoot);

    let child_is_parent = TaskSpec::io(TaskClass::new("worker"))
        .child_of("root-1", "parent-2", TaskStage::new("fanout"))
        .operation("parent-2");
    assert_invalid(child_is_parent, TaskPlanError::ParentOperationMatchesChild);

    let empty_parent = TaskSpec::io(TaskClass::new("worker"))
        .child_of("root-1", "", TaskStage::new("fanout"))
        .operation("child-3");
    assert_invalid(
        empty_parent,
        TaskPlanError::InvalidIdentifier {
            field: TaskIdentifierField::ParentOperationId,
            violation: IdentifierViolation::Empty,
        },
    );
}

#[test]
fn serde_cannot_construct_a_validated_plan_from_invalid_raw_input() {
    let mut raw = root();
    raw.stages.push(raw.stages[0].clone());
    let json = serde_json::to_string(&raw).unwrap();
    let decoded_raw: TaskSpec = serde_json::from_str(&json).expect("raw wire data remains raw");
    assert_eq!(decoded_raw, raw);
    let error = serde_json::from_str::<ValidatedTaskPlan>(&json).unwrap_err();
    assert!(error.to_string().contains("duplicate stage descriptor: io"));
}

#[test]
fn serde_rejects_missing_immediate_parent_and_invalid_provenance() {
    let child_without_parent = r#"{
        "class":"worker",
        "operation":"child-3",
        "root_operation_id":"root-1",
        "source":"internal",
        "reason":"ExplicitMapping",
        "scope":{"Child":{"parent_stage":"fanout","parent_awaits":true}},
        "stages":[{"class":"worker","stage":"io","substrate_hint":"AsyncIo","fan_out":false,"reduce_policy":null}]
    }"#;
    let error = serde_json::from_str::<TaskSpec>(child_without_parent)
        .expect_err("a child scope without its immediate parent identity must be rejected");
    assert!(
        error.to_string().contains("parent_operation_id"),
        "the wire error must identify the missing parent operation: {error}"
    );
    let error = serde_json::from_str::<PlanSource>(r#""source with spaces""#).unwrap_err();
    assert!(error.to_string().contains("invalid character"));
}
