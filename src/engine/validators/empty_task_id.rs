//! Rejects `TaskCreated` events with an empty task ID.
//!
//! An empty task ID is meaningless — the task cannot be referenced for
//! updates, prioritisation, or archival.

use crate::engine::validator::{ValidationError, Validator};
use crate::models::{ADR, RuntimeEvent, RuntimeState, Trap};

pub struct EmptyTaskId;

impl Validator for EmptyTaskId {
    fn validate(
        &self,
        event: &RuntimeEvent,
        _state: &RuntimeState,
        _decisions: &[ADR],
        _traps: &[Trap],
    ) -> Result<(), ValidationError> {
        if let RuntimeEvent::TaskCreated(task) = event
            && task.id.is_empty()
        {
            return Err(ValidationError::new(
                    self.name(),
                    "Task ID cannot be empty.",
                    "Provide a non-empty task ID.",
                ));
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "empty_task_id"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Task, TaskStatus};

    fn empty_state() -> RuntimeState {
        RuntimeState {
            current_phase: String::new(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        }
    }

    fn empty_decisions() -> Vec<ADR> {
        vec![]
    }

    fn empty_traps() -> Vec<Trap> {
        vec![]
    }

    #[test]
    fn passes_on_non_task_created_event() {
        let validator = EmptyTaskId;
        let event = RuntimeEvent::PhaseChanged("plan".into());
        let result = validator.validate(&event, &empty_state(), &empty_decisions(), &empty_traps());
        assert!(result.is_ok(), "PhaseChanged should pass");
    }

    #[test]
    fn passes_on_valid_task_id() {
        let validator = EmptyTaskId;
        let event = RuntimeEvent::TaskCreated(Task {
            id: "TASK-001".into(),
            description: "valid".into(),
            status: TaskStatus::Todo,
        });
        let result = validator.validate(&event, &empty_state(), &empty_decisions(), &empty_traps());
        assert!(result.is_ok(), "non-empty ID should pass");
    }

    #[test]
    fn rejects_empty_task_id() {
        let validator = EmptyTaskId;
        let event = RuntimeEvent::TaskCreated(Task {
            id: "".into(),
            description: "invalid".into(),
            status: TaskStatus::Todo,
        });
        let result = validator.validate(&event, &empty_state(), &empty_decisions(), &empty_traps());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.validator_name, "empty_task_id");
        assert!(err.message.contains("cannot be empty"));
        assert!(err.suggestion.contains("non-empty"));
    }

    #[test]
    fn name_returns_expected() {
        assert_eq!(EmptyTaskId.name(), "empty_task_id");
    }
}
