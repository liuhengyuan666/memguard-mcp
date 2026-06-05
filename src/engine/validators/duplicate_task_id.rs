//! Rejects `TaskCreated` events when the task ID already exists in
//! `RuntimeState::active_tasks`.
//!
//! Duplicate task IDs break the invariant that every active task has
//! a unique identifier.  The caller should use `TaskUpdated` instead.

use crate::engine::validator::{ValidationError, Validator};
use crate::models::{ADR, RuntimeEvent, RuntimeState, Trap};

pub struct DuplicateTaskId;

impl Validator for DuplicateTaskId {
    fn validate(
        &self,
        event: &RuntimeEvent,
        state: &RuntimeState,
        _decisions: &[ADR],
        _traps: &[Trap],
    ) -> Result<(), ValidationError> {
        if let RuntimeEvent::TaskCreated(task) = event
            && state.active_tasks.iter().any(|t| t.id == task.id)
        {
            return Err(ValidationError::new(
                    self.name(),
                    &format!("Task with id '{}' already exists.", task.id),
                    "Use TaskUpdated to modify the existing task.",
                ));
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "duplicate_task_id"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Task, TaskStatus};

    fn state_with_task(id: &str) -> RuntimeState {
        RuntimeState {
            current_phase: String::new(),
            active_tasks: vec![Task {
            id: id.to_string(),
            description: "existing".into(),
            status: TaskStatus::Todo,
            superseded_by: None,
        }],
            done_tasks: vec![],
            constraints: vec![],
        }
    }

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
        let validator = DuplicateTaskId;
        let event = RuntimeEvent::PhaseChanged("plan".into());
        let result = validator.validate(&event, &empty_state(), &empty_decisions(), &empty_traps());
        assert!(result.is_ok(), "PhaseChanged should pass");
    }

    #[test]
    fn passes_on_unique_task_id() {
        let validator = DuplicateTaskId;
        let state = state_with_task("TASK-001");
        let event = RuntimeEvent::TaskCreated(Task {
            id: "TASK-002".into(),
            description: "unique".into(),
            status: TaskStatus::Todo,
            superseded_by: None,
        });
        let result = validator.validate(&event, &state, &empty_decisions(), &empty_traps());
        assert!(result.is_ok(), "unique ID should pass");
    }

    #[test]
    fn passes_on_empty_state() {
        let validator = DuplicateTaskId;
        let event = RuntimeEvent::TaskCreated(Task {
            id: "TASK-001".into(),
            description: "first task".into(),
            status: TaskStatus::Todo,
            superseded_by: None,
        });
        let result = validator.validate(&event, &empty_state(), &empty_decisions(), &empty_traps());
        assert!(result.is_ok(), "empty state should always pass");
    }

    #[test]
    fn rejects_duplicate_task_id() {
        let validator = DuplicateTaskId;
        let state = state_with_task("TASK-001");
        let event = RuntimeEvent::TaskCreated(Task {
            id: "TASK-001".into(),
            description: "duplicate".into(),
            status: TaskStatus::Todo,
            superseded_by: None,
        });
        let result = validator.validate(&event, &state, &empty_decisions(), &empty_traps());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.validator_name, "duplicate_task_id");
        assert!(err.message.contains("already exists"));
        assert!(err.message.contains("TASK-001"));
        assert!(err.suggestion.contains("TaskUpdated"));
    }

    #[test]
    fn name_returns_expected() {
        assert_eq!(DuplicateTaskId.name(), "duplicate_task_id");
    }
}
