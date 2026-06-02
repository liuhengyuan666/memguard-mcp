//! Rejects `AdrCommitted` events when an accepted ADR with the same ID
//! already exists and has different content (title/decision).
//!
//! This guards against accidental overwrites of active architectural
//! decisions.  Idempotent re-submissions with identical content are
//! NOT treated as conflicts — they pass.

use crate::engine::validator::{ValidationError, Validator};
use crate::engine::validators::content_hash;
use crate::models::{ADR, AdrStatus, RuntimeEvent, RuntimeState, Trap};

pub struct AdrActiveConflict;

impl Validator for AdrActiveConflict {
    fn validate(
        &self,
        event: &RuntimeEvent,
        _state: &RuntimeState,
        decisions: &[ADR],
        _traps: &[Trap],
    ) -> Result<(), ValidationError> {
        if let RuntimeEvent::AdrCommitted(adr) = event {
            let new_hash = content_hash(adr);

            for existing in decisions.iter() {
                if existing.id != adr.id {
                    continue;
                }
                if existing.status != AdrStatus::Accepted {
                    continue;
                }
                let existing_hash = content_hash(existing);
                if existing_hash == new_hash {
                    // Idempotent — same content, not a conflict.
                    return Ok(());
                }
                // Different content — conflict.
                return Err(ValidationError::new(
                    self.name(),
                    &format!(
                        "[CONFLICT] ADR {} conflict: an active ADR with this id already exists with different content.",
                        adr.id
                    ),
                    "Re-read current ADR state. To supersede: first submit a PhaseChanged or use a new ADR id.",
                ));
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "adr_active_conflict"
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
        let validator = AdrActiveConflict;
        let event = RuntimeEvent::TaskCreated(Task {
            id: "TASK-001".into(),
            description: "test".into(),
            status: TaskStatus::Todo,
        });
        let result = validator.validate(&event, &empty_state(), &[], &empty_traps());
        assert!(result.is_ok());
    }

    #[test]
    fn passes_on_empty_decisions() {
        let validator = AdrActiveConflict;
        let adr = ADR {
            id: "ADR-001".into(),
            title: "New ADR".into(),
            status: AdrStatus::Proposed,
            context: "ctx".into(),
            decision: "dec".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &[], &empty_traps());
        assert!(result.is_ok());
    }

    #[test]
    fn passes_idempotent_same_content() {
        let validator = AdrActiveConflict;
        let existing = vec![ADR {
            id: "ADR-001".into(),
            title: "Use Postgres".into(),
            status: AdrStatus::Accepted,
            context: "ctx".into(),
            decision: "Use Postgres for persistence".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "Use Postgres".into(),
            status: AdrStatus::Proposed,
            context: "ctx".into(),
            decision: "Use Postgres for persistence".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &existing, &empty_traps());
        assert!(result.is_ok(), "idempotent should pass");
    }

    #[test]
    fn rejects_active_conflict_different_content() {
        let validator = AdrActiveConflict;
        let existing = vec![ADR {
            id: "ADR-001".into(),
            title: "Use Postgres".into(),
            status: AdrStatus::Accepted,
            context: "ctx".into(),
            decision: "Use Postgres for persistence".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "Use SQLite".into(),
            status: AdrStatus::Proposed,
            context: "ctx".into(),
            decision: "Use SQLite for persistence".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &existing, &empty_traps());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.validator_name, "adr_active_conflict");
        assert!(err.message.contains("conflict"));
        assert!(err.message.contains("ADR-001"));
    }

    #[test]
    fn ignores_non_accepted_existing() {
        let validator = AdrActiveConflict;
        let existing = vec![ADR {
            id: "ADR-001".into(),
            title: "Old".into(),
            status: AdrStatus::Superseded,
            context: "old".into(),
            decision: "old".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "New".into(),
            status: AdrStatus::Proposed,
            context: "new".into(),
            decision: "new".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &existing, &empty_traps());
        assert!(result.is_ok(), "superseded should not cause conflict");
    }

    #[test]
    fn name_returns_expected() {
        assert_eq!(AdrActiveConflict.name(), "adr_active_conflict");
    }
}
