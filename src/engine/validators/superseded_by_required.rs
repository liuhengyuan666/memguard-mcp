//! Validates that TaskUpdated events with status `Superseded` include
//! a `superseded_by` field, and that the referenced task or ADR exists.

use crate::engine::validator::{ValidationError, Validator};
use crate::models::{ADR, Reference, RuntimeEvent, RuntimeState, Trap};

pub struct SupersededByRequired;

impl Validator for SupersededByRequired {
    fn validate(
        &self,
        event: &RuntimeEvent,
        _state: &RuntimeState,
        decisions: &[ADR],
        _traps: &[Trap],
    ) -> Result<(), ValidationError> {
        let (task_id, superseded_by) = match event {
            RuntimeEvent::TaskUpdated {
                task_id,
                new_status,
                superseded_by,
            } if *new_status == crate::models::TaskStatus::Superseded => {
                (task_id, superseded_by)
            }
            _ => return Ok(()),
        };

        // 1. Must have superseded_by info.
        let info = superseded_by.as_ref().ok_or_else(|| {
            ValidationError::new(
                "SupersededByRequired",
                &format!(
                    "Task {} marked as Superseded but missing 'superseded_by' field",
                    task_id
                ),
                "Provide superseded_by with reference (Task or ADR) and reason.",
            )
        })?;

        // 2. Reason must not be empty.
        if info.reason.trim().is_empty() {
            return Err(ValidationError::new(
                "SupersededByRequired",
                &format!(
                    "Task {} Superseded by {:?} but reason is empty",
                    task_id, info.reference
                ),
                "Provide a non-empty reason explaining why the task was superseded.",
            ));
        }

        // 3. If referencing an ADR, verify it exists.
        if let Reference::Adr(adr_id) = &info.reference {
            if !decisions.iter().any(|a| a.id == *adr_id) {
                return Err(ValidationError::new(
                    "SupersededByRequired",
                    &format!(
                        "Task {} Superseded by ADR {} but that ADR does not exist in decisions.md",
                        task_id, adr_id
                    ),
                    "Commit the ADR first, or correct the ADR ID in superseded_by.",
                ));
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "SupersededByRequired"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ADR, AdrStatus, Reference, RuntimeEvent, RuntimeState, SupersededInfo, Task, TaskStatus};

    fn make_adr(id: &str) -> ADR {
        ADR {
            id: id.into(),
            title: "title".into(),
            status: AdrStatus::Accepted,
            context: "ctx".into(),
            decision: "dec".into(),
            tags: vec![],
        }
    }

    fn empty_state() -> RuntimeState {
        RuntimeState {
            current_phase: "plan".into(),
            active_tasks: vec![],
            done_tasks: vec![],
            constraints: vec![],
        }
    }

    #[test]
    fn passes_when_superseded_by_adr_exists() {
        let v = SupersededByRequired;
        let decisions = vec![make_adr("ADR-053")];
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Superseded,
            superseded_by: Some(SupersededInfo {
                reference: Reference::Adr("ADR-053".into()),
                reason: "Redesigned".into(),
            }),
        };
        assert!(v.validate(&event, &empty_state(), &decisions, &[]).is_ok());
    }

    #[test]
    fn fails_when_superseded_by_missing() {
        let v = SupersededByRequired;
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Superseded,
            superseded_by: None,
        };
        assert!(v.validate(&event, &empty_state(), &[], &[]).is_err());
    }

    #[test]
    fn fails_when_reason_empty() {
        let v = SupersededByRequired;
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Superseded,
            superseded_by: Some(SupersededInfo {
                reference: Reference::Adr("ADR-053".into()),
                reason: "".into(),
            }),
        };
        assert!(v.validate(&event, &empty_state(), &[], &[]).is_err());
    }

    #[test]
    fn fails_when_adr_not_found() {
        let v = SupersededByRequired;
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Superseded,
            superseded_by: Some(SupersededInfo {
                reference: Reference::Adr("ADR-999".into()),
                reason: "Redesigned".into(),
            }),
        };
        assert!(v.validate(&event, &empty_state(), &[], &[]).is_err());
    }

    #[test]
    fn ignores_non_superseded_events() {
        let v = SupersededByRequired;
        let event = RuntimeEvent::TaskUpdated {
            task_id: "T1".into(),
            new_status: TaskStatus::Done,
            superseded_by: None,
        };
        assert!(v.validate(&event, &empty_state(), &[], &[]).is_ok());
    }
}
