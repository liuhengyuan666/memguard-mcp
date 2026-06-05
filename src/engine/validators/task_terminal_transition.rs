//! Validates that terminal-status tasks (Done, Superseded, Cancelled)
//! cannot be transitioned to any other status.

use crate::engine::validator::{ValidationError, Validator};
use crate::models::{ADR, RuntimeEvent, RuntimeState, TaskStatus, Trap};

pub struct TaskTerminalTransition;

impl Validator for TaskTerminalTransition {
    fn validate(
        &self,
        event: &RuntimeEvent,
        state: &RuntimeState,
        _decisions: &[ADR],
        _traps: &[Trap],
    ) -> Result<(), ValidationError> {
        let (task_id, new_status) = match event {
            RuntimeEvent::TaskUpdated {
                task_id,
                new_status,
                ..
            } => (task_id, new_status),
            _ => return Ok(()),
        };

        // Find the current task to check its status.
        let current_status = state
            .active_tasks
            .iter()
            .find(|t| t.id == *task_id)
            .map(|t| &t.status)
            .or_else(|| state.done_tasks.iter().find(|t| t.id == *task_id).map(|t| &t.status));

        if let Some(current) = current_status {
            let is_terminal = matches!(
                current,
                TaskStatus::Done | TaskStatus::Superseded | TaskStatus::Cancelled
            );
            if is_terminal {
                return Err(ValidationError::new(
                    "TaskTerminalTransition",
                    &format!(
                        "Task {} is already in terminal status {:?} and cannot be transitioned to {:?}",
                        task_id, current, new_status
                    ),
                    "Terminal tasks (Done, Superseded, Cancelled) are immutable. Create a new task if needed.",
                ));
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TaskTerminalTransition"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RuntimeEvent, RuntimeState, Task, TaskStatus};

    fn make_task(id: &str, status: TaskStatus) -> Task {
        Task {
            id: id.into(),
            description: "test".into(),
            status,
            superseded_by: None,
        }
    }

    fn state_with_tasks(active: Vec<Task>, done: Vec<Task>) -> RuntimeState {
        RuntimeState {
            current_phase: "plan".into(),
            active_tasks: active,
            done_tasks: done,
            constraints: vec![],
        }
    }

    #[test]
    fn allows_transition_from_todo() {
        let v = TaskTerminalTransition;
        let state = state_with_tasks(vec![make_task("T1", TaskStatus::Todo)], vec![]);
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Done,
            superseded_by: None,
        };
        assert!(v.validate(&event, &state, &[], &[]).is_ok());
    }

    #[test]
    fn blocks_transition_from_done() {
        let v = TaskTerminalTransition;
        let state = state_with_tasks(vec![], vec![make_task("T1", TaskStatus::Done)]);
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Superseded,
            superseded_by: None,
        };
        assert!(v.validate(&event, &state, &[], &[]).is_err());
    }

    #[test]
    fn blocks_transition_from_superseded() {
        let v = TaskTerminalTransition;
        let state = state_with_tasks(vec![], vec![make_task("T1", TaskStatus::Superseded)]);
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Todo,
            superseded_by: None,
        };
        assert!(v.validate(&event, &state, &[], &[]).is_err());
    }

    #[test]
    fn blocks_transition_from_cancelled() {
        let v = TaskTerminalTransition;
        let state = state_with_tasks(vec![], vec![make_task("T1", TaskStatus::Cancelled)]);
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Todo,
            superseded_by: None,
        };
        assert!(v.validate(&event, &state, &[], &[]).is_err());
    }

    #[test]
    fn ignores_non_taskupdated_events() {
        let v = TaskTerminalTransition;
        let state = state_with_tasks(vec![], vec![]);
        assert!(v.validate(&RuntimeEvent::PhaseChanged("plan".into()), &state, &[], &[]).is_ok());
    }
}
