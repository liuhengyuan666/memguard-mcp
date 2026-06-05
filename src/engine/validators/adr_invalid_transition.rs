//! Rejects `AdrCommitted` events when an existing ADR with the same ID
//! is in a terminal state (Superseded or Archived) that has no valid
//! next state transitions.
//!
//! Terminal ADRs cannot be modified — the caller should create a new
//! ADR with a different identifier.

use crate::engine::state_manager::valid_transitions;
use crate::engine::validator::{ValidationError, Validator};
use crate::models::{ADR, RuntimeEvent, RuntimeState, Trap};

pub struct AdrInvalidTransition;

impl Validator for AdrInvalidTransition {
    fn validate(
        &self,
        event: &RuntimeEvent,
        _state: &RuntimeState,
        decisions: &[ADR],
        _traps: &[Trap],
    ) -> Result<(), ValidationError> {
        if let RuntimeEvent::AdrCommitted(adr) = event {
            for existing in decisions.iter() {
                if existing.id != adr.id {
                    continue;
                }
                let valid_next = valid_transitions(&existing.status);
                if valid_next.is_empty() {
                    return Err(ValidationError::new(
                        self.name(),
                        &format!(
                            "[INVALID TRANSITION] ADR {} has status {} which is terminal. Valid transitions from {}: none.",
                            adr.id, existing.status, existing.status
                        ),
                        "Create a new ADR with a different id.",
                    ));
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "adr_invalid_transition"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AdrStatus, Task, TaskStatus};

    fn empty_state() -> RuntimeState {
        RuntimeState {
            current_phase: String::new(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        }
    }

    fn empty_traps() -> Vec<Trap> {
        vec![]
    }

    #[test]
    fn passes_on_non_adr_event() {
        let validator = AdrInvalidTransition;
        let event = RuntimeEvent::TaskCreated(Task {
            id: "TASK-001".into(),
            description: "test".into(),
            status: TaskStatus::Todo,
            superseded_by: None,
        });
        let result = validator.validate(&event, &empty_state(), &[], &empty_traps());
        assert!(result.is_ok());
    }

    #[test]
    fn passes_on_empty_decisions() {
        let validator = AdrInvalidTransition;
        let adr = ADR {
            id: "ADR-001".into(),
            title: "New".into(),
            status: AdrStatus::Proposed,
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &[], &empty_traps());
        assert!(result.is_ok());
    }

    #[test]
    fn passes_on_existing_with_valid_transitions() {
        let validator = AdrInvalidTransition;
        let decisions = vec![ADR {
            id: "ADR-001".into(),
            title: "Old".into(),
            status: AdrStatus::Proposed,
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "New".into(),
            status: AdrStatus::Accepted,
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &decisions, &empty_traps());
        assert!(result.is_ok(), "Proposed status should have valid transitions");
    }

    #[test]
    fn rejects_superseded_is_terminal() {
        let validator = AdrInvalidTransition;
        let decisions = vec![ADR {
            id: "ADR-001".into(),
            title: "Old".into(),
            status: AdrStatus::Superseded,
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "New".into(),
            status: AdrStatus::Proposed,
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &decisions, &empty_traps());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.validator_name, "adr_invalid_transition");
        assert!(err.message.contains("terminal"));
        assert!(err.message.contains("ADR-001"));
        assert!(err.message.contains("Superseded"));
    }

    #[test]
    fn rejects_archived_is_terminal() {
        let validator = AdrInvalidTransition;
        let decisions = vec![ADR {
            id: "ADR-001".into(),
            title: "Old".into(),
            status: AdrStatus::Archived,
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "New".into(),
            status: AdrStatus::Proposed,
            context: "".into(),
            decision: "".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &decisions, &empty_traps());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("terminal"));
        assert!(err.message.contains("ADR-001"));
        assert!(err.message.contains("Archived"));
    }

    #[test]
    fn name_returns_expected() {
        assert_eq!(AdrInvalidTransition.name(), "adr_invalid_transition");
    }
}
