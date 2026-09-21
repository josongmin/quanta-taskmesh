//! Validation for raw task plans.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::task::{StageDescriptor, TaskScope, TaskSpec, TaskStage};

/// Maximum UTF-8 byte length of any task-plan identifier.
pub const MAX_TASK_IDENTIFIER_LEN: usize = 128;

/// Identifies the exact field rejected by [`TaskPlanError::InvalidIdentifier`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskIdentifierField {
    PlanSource,
    Class,
    Operation,
    RootOperationId,
    ParentOperationId,
    ParentStage,
    Stage { index: usize },
    ReduceKey { index: usize },
}

impl fmt::Display for TaskIdentifierField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlanSource => f.write_str("source"),
            Self::Class => f.write_str("class"),
            Self::Operation => f.write_str("operation"),
            Self::RootOperationId => f.write_str("root_operation_id"),
            Self::ParentOperationId => f.write_str("parent_operation_id"),
            Self::ParentStage => f.write_str("parent_stage"),
            Self::Stage { index } => write!(f, "stages[{index}].stage"),
            Self::ReduceKey { index } => write!(f, "stages[{index}].reduce_policy.stable_sort_key"),
        }
    }
}

/// Why an identifier is not canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentifierViolation {
    Empty,
    SurroundingWhitespace,
    TooLong { actual: usize, max: usize },
    InvalidCharacter { byte_offset: usize, character: char },
}

impl fmt::Display for IdentifierViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("must not be empty"),
            Self::SurroundingWhitespace => f.write_str("must not have surrounding whitespace"),
            Self::TooLong { actual, max } => {
                write!(f, "is {actual} bytes, above the {max}-byte maximum")
            }
            Self::InvalidCharacter {
                byte_offset,
                character,
            } => write!(
                f,
                "contains invalid character {character:?} at byte offset {byte_offset}"
            ),
        }
    }
}

/// Deterministic rejection returned when a raw [`TaskSpec`] is not admissible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskPlanError {
    InvalidIdentifier {
        field: TaskIdentifierField,
        violation: IdentifierViolation,
    },
    NoStages,
    RootIdentityMismatch,
    ChildOperationMatchesRoot,
    ParentOperationMatchesChild,
    DuplicateStage {
        stage: TaskStage,
    },
    ConflictingStage {
        stage: TaskStage,
    },
    StageClassMismatch {
        stage: TaskStage,
    },
    MissingReducePolicy {
        stage: TaskStage,
    },
    UnexpectedReducePolicy {
        stage: TaskStage,
    },
}

impl fmt::Display for TaskPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field, violation } => {
                write!(f, "invalid task-plan identifier {field}: {violation}")
            }
            Self::NoStages => f.write_str("task plan must declare at least one stage"),
            Self::RootIdentityMismatch => {
                f.write_str("root task operation must equal root_operation_id")
            }
            Self::ChildOperationMatchesRoot => {
                f.write_str("child operation must differ from root_operation_id")
            }
            Self::ParentOperationMatchesChild => {
                f.write_str("parent_operation_id must differ from the child operation")
            }
            Self::DuplicateStage { stage } => write!(f, "duplicate stage descriptor: {stage}"),
            Self::ConflictingStage { stage } => {
                write!(f, "conflicting descriptors for stage: {stage}")
            }
            Self::StageClassMismatch { stage } => {
                write!(f, "stage {stage} does not carry the task class")
            }
            Self::MissingReducePolicy { stage } => {
                write!(
                    f,
                    "fan-out stage {stage} has no deterministic reduce policy"
                )
            }
            Self::UnexpectedReducePolicy { stage } => {
                write!(f, "non-fan-out stage {stage} carries a reduce policy")
            }
        }
    }
}

impl Error for TaskPlanError {}

/// Admission-ready task plan. Construction always runs the contract validator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ValidatedTaskPlan(TaskSpec);

impl ValidatedTaskPlan {
    pub fn as_spec(&self) -> &TaskSpec {
        &self.0
    }

    pub fn into_spec(self) -> TaskSpec {
        self.0
    }
}

impl AsRef<TaskSpec> for ValidatedTaskPlan {
    fn as_ref(&self) -> &TaskSpec {
        self.as_spec()
    }
}

impl TryFrom<TaskSpec> for ValidatedTaskPlan {
    type Error = TaskPlanError;

    fn try_from(spec: TaskSpec) -> Result<Self, Self::Error> {
        validate_task_spec(&spec)?;
        Ok(Self(spec))
    }
}

impl From<ValidatedTaskPlan> for TaskSpec {
    fn from(plan: ValidatedTaskPlan) -> Self {
        plan.into_spec()
    }
}

impl<'de> Deserialize<'de> for ValidatedTaskPlan {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = TaskSpec::deserialize(deserializer)?;
        Self::try_from(raw).map_err(serde::de::Error::custom)
    }
}

pub fn validate_identifier(field: TaskIdentifierField, value: &str) -> Result<(), TaskPlanError> {
    let violation = if value.is_empty() {
        Some(IdentifierViolation::Empty)
    } else if value.trim() != value {
        Some(IdentifierViolation::SurroundingWhitespace)
    } else if value.len() > MAX_TASK_IDENTIFIER_LEN {
        Some(IdentifierViolation::TooLong {
            actual: value.len(),
            max: MAX_TASK_IDENTIFIER_LEN,
        })
    } else {
        value.char_indices().find_map(|(byte_offset, character)| {
            (!is_identifier_character(character)).then_some(IdentifierViolation::InvalidCharacter {
                byte_offset,
                character,
            })
        })
    };

    match violation {
        Some(violation) => Err(TaskPlanError::InvalidIdentifier { field, violation }),
        None => Ok(()),
    }
}

fn is_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':' | '/')
}

pub fn validate_task_spec(spec: &TaskSpec) -> Result<(), TaskPlanError> {
    validate_identifier(TaskIdentifierField::Class, spec.class.as_str())?;
    validate_identifier(TaskIdentifierField::Operation, &spec.operation)?;
    validate_identifier(
        TaskIdentifierField::RootOperationId,
        &spec.root_operation_id,
    )?;

    match &spec.scope {
        TaskScope::Root => {
            if spec.operation != spec.root_operation_id {
                return Err(TaskPlanError::RootIdentityMismatch);
            }
        }
        TaskScope::Child {
            parent_operation_id,
            parent_stage,
            ..
        } => {
            validate_identifier(TaskIdentifierField::ParentOperationId, parent_operation_id)?;
            validate_identifier(TaskIdentifierField::ParentStage, parent_stage.as_str())?;
            if spec.operation == spec.root_operation_id {
                return Err(TaskPlanError::ChildOperationMatchesRoot);
            }
            if spec.operation == *parent_operation_id {
                return Err(TaskPlanError::ParentOperationMatchesChild);
            }
        }
    }

    if spec.stages.is_empty() {
        return Err(TaskPlanError::NoStages);
    }

    let mut stages: BTreeMap<&TaskStage, &StageDescriptor> = BTreeMap::new();
    for (index, descriptor) in spec.stages.iter().enumerate() {
        validate_identifier(
            TaskIdentifierField::Stage { index },
            descriptor.stage.as_str(),
        )?;

        if let Some(previous) = stages.insert(&descriptor.stage, descriptor) {
            return if previous == descriptor {
                Err(TaskPlanError::DuplicateStage {
                    stage: descriptor.stage.clone(),
                })
            } else {
                Err(TaskPlanError::ConflictingStage {
                    stage: descriptor.stage.clone(),
                })
            };
        }

        if descriptor.class != spec.class {
            return Err(TaskPlanError::StageClassMismatch {
                stage: descriptor.stage.clone(),
            });
        }

        match (descriptor.fan_out, &descriptor.reduce_policy) {
            (true, None) => {
                return Err(TaskPlanError::MissingReducePolicy {
                    stage: descriptor.stage.clone(),
                });
            }
            (false, Some(_)) => {
                return Err(TaskPlanError::UnexpectedReducePolicy {
                    stage: descriptor.stage.clone(),
                });
            }
            (_, Some(policy)) => validate_identifier(
                TaskIdentifierField::ReduceKey { index },
                policy.stable_sort_key.as_ref(),
            )?,
            (false, None) => {}
        }
    }

    Ok(())
}
