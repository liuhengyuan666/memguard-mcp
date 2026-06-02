//! Rejects `AdrCommitted` events when the same decision content
//! (title + decision) was previously rejected for the same ADR ID.
//!
//! Re-submitting an identical proposal without addressing the reasons
//! for rejection wastes deliberation cycles.  The caller must explain
//! what material conditions have changed.

use crate::engine::validator::{ValidationError, Validator};
use crate::engine::validators::content_hash;
use crate::models::{ADR, AdrStatus, RuntimeEvent, RuntimeState, Trap};

pub struct AdrRejectedRepeat;

impl Validator for AdrRejectedRepeat {
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
                if existing.status != AdrStatus::Rejected {
                    continue;
                }
                let existing_hash = content_hash(existing);
                if existing_hash == new_hash {
                    return Err(ValidationError::new(
                        self.name(),
                        &format!(
                            "[CONFLICT] ADR {} was previously rejected with the same decision content.",
                            adr.id
                        ),
                        "To re-submit, explain what material conditions have changed in the context field.",
                    ));
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "adr_rejected_repeat"
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

    fn empty_traps() -> Vec<Trap> {
        vec![]
    }

    #[test]
    fn passes_on_non_adr_event() {
        let validator = AdrRejectedRepeat;
        let event = RuntimeEvent::TaskCreated(Task {
            id: "TASK-001".into(),
            description: "test".into(),
            status: TaskStatus::Todo,
        });
        let result = validator.validate(&event, &empty_state(), &[], &empty_traps());
        assert!(result.is_ok());
    }

    #[test]
    fn passes_when_no_prior_rejected() {
        let validator = AdrRejectedRepeat;
        let decisions = vec![ADR {
            id: "ADR-001".into(),
            title: "Use Postgres".into(),
            status: AdrStatus::Accepted,
            context: "".into(),
            decision: "Use Postgres for persistence".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "Use Cassandra".into(),
            status: AdrStatus::Proposed,
            context: "new".into(),
            decision: "Use Cassandra for persistence".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &decisions, &empty_traps());
        assert!(result.is_ok());
    }

    #[test]
    fn passes_rejected_different_content() {
        let validator = AdrRejectedRepeat;
        let decisions = vec![ADR {
            id: "ADR-001".into(),
            title: "Use Cassandra".into(),
            status: AdrStatus::Rejected,
            context: "old".into(),
            decision: "Use Cassandra for persistence".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "Use Cassandra with Sharding".into(),
            status: AdrStatus::Proposed,
            context: "new reqs".into(),
            decision: "Use Cassandra with consistent hashing".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &decisions, &empty_traps());
        assert!(result.is_ok(), "different content should pass");
    }

    #[test]
    fn rejects_rejected_repeat() {
        let validator = AdrRejectedRepeat;
        let decisions = vec![ADR {
            id: "ADR-001".into(),
            title: "Use Cassandra".into(),
            status: AdrStatus::Rejected,
            context: "old".into(),
            decision: "Use Cassandra for persistence".into(),
            tags: vec![],
        }];
        let adr = ADR {
            id: "ADR-001".into(),
            title: "Use Cassandra".into(),
            status: AdrStatus::Proposed,
            context: "same old".into(),
            decision: "Use Cassandra for persistence".into(),
            tags: vec![],
        };
        let event = RuntimeEvent::AdrCommitted(adr);
        let result = validator.validate(&event, &empty_state(), &decisions, &empty_traps());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.validator_name, "adr_rejected_repeat");
        assert!(err.message.contains("rejected"));
        assert!(err.message.contains("ADR-001"));
        assert!(err.suggestion.contains("material conditions"));
    }

    #[test]
    fn name_returns_expected() {
        assert_eq!(AdrRejectedRepeat.name(), "adr_rejected_repeat");
    }
}
