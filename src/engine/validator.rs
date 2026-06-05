//! Validation trait and registry for guarding event application.
//!
//! Validators are called before `StateManager::apply_event` to detect
//! conflicts, inconsistencies, or policy violations. Each validator
//! returns either `Ok(())` (pass) or `Err(ValidationError)` (fail).
//! The registry stops at the first error and returns it immediately.

use crate::models::{ADR, RuntimeEvent, RuntimeState, Trap};

/// Structured validation failure carrying the validator identity,
/// a human-readable explanation, and a suggestion for resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub validator_name: String,
    pub message: String,
    pub suggestion: String,
}

impl ValidationError {
    pub fn new(validator_name: &str, message: &str, suggestion: &str) -> Self {
        Self {
            validator_name: validator_name.to_string(),
            message: message.to_string(),
            suggestion: suggestion.to_string(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} (suggestion: {})",
            self.validator_name, self.message, self.suggestion
        )
    }
}

impl std::error::Error for ValidationError {}

/// A validator inspects a proposed `RuntimeEvent` against current
/// `RuntimeState`, ADR history, and trap log before it is applied.
///
/// Validators are pure (no side-effects) and synchronous. Concrete
/// implementations live in child modules and are registered with
/// `ValidatorRegistry`.
pub trait Validator: Send + Sync {
    /// Check whether `event` is acceptable given current memory state.
    /// Returns `Ok(())` on pass, `Err(ValidationError)` on failure.
    fn validate(
        &self,
        event: &RuntimeEvent,
        state: &RuntimeState,
        decisions: &[ADR],
        traps: &[Trap],
    ) -> Result<(), ValidationError>;

    /// A unique, stable identifier used in error messages.
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
}

/// A collection of validators that are run in registration order.
/// `validate_all` stops at the first error and returns it immediately.
pub struct ValidatorRegistry {
    validators: Vec<Box<dyn Validator>>,
}

impl ValidatorRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// Register a validator. Order matters — validators are run in
    /// the order they are registered.
    pub fn register(&mut self, validator: Box<dyn Validator>) {
        self.validators.push(validator);
    }

    /// Run every registered validator in order. Returns `Ok(())` if
    /// all pass, or `Err(ValidationError)` from the first one that fails.
    pub fn validate_all(
        &self,
        event: &RuntimeEvent,
        state: &RuntimeState,
        decisions: &[ADR],
        traps: &[Trap],
    ) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate(event, state, decisions, traps)?;
        }
        Ok(())
    }

    /// Returns the number of registered validators.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.validators.len()
    }

    /// Returns `true` if no validators are registered.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }
}

impl Default for ValidatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ADR, RuntimeEvent, RuntimeState, Task, TaskStatus, Trap};

    // ---- Helper test validators ----

    struct AlwaysPass;
    impl Validator for AlwaysPass {
        fn validate(
            &self,
            _event: &RuntimeEvent,
            _state: &RuntimeState,
            _decisions: &[ADR],
            _traps: &[Trap],
        ) -> Result<(), ValidationError> {
            Ok(())
        }
        fn name(&self) -> &'static str {
            "always_pass"
        }
    }

    struct AlwaysFail;
    impl Validator for AlwaysFail {
        fn validate(
            &self,
            _event: &RuntimeEvent,
            _state: &RuntimeState,
            _decisions: &[ADR],
            _traps: &[Trap],
        ) -> Result<(), ValidationError> {
            Err(ValidationError::new(
                "always_fail",
                "deliberate test failure",
                "remove this validator",
            ))
        }
        fn name(&self) -> &'static str {
            "always_fail"
        }
    }

    struct FailOnTaskCreated;
    impl Validator for FailOnTaskCreated {
        fn validate(
            &self,
            event: &RuntimeEvent,
            _state: &RuntimeState,
            _decisions: &[ADR],
            _traps: &[Trap],
        ) -> Result<(), ValidationError> {
            if matches!(event, RuntimeEvent::TaskCreated(_)) {
                return Err(ValidationError::new(
                    "fail_on_task_created",
                    "TaskCreated not allowed",
                    "use a different event type",
                ));
            }
            Ok(())
        }
        fn name(&self) -> &'static str {
            "fail_on_task_created"
        }
    }

    // ---- Helper factories ----

    fn empty_state() -> RuntimeState {
        RuntimeState {
            current_phase: "explore".into(),
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

    // ---- Tests ----

    #[test]
    fn registry_starts_empty() {
        let reg = ValidatorRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn registry_can_hold_multiple_validators() {
        let mut reg = ValidatorRegistry::new();
        reg.register(Box::new(AlwaysPass));
        reg.register(Box::new(AlwaysPass));
        assert_eq!(reg.len(), 2);
        assert!(!reg.is_empty());
    }

    #[test]
    fn validate_all_passes_when_no_validators() {
        let reg = ValidatorRegistry::new();
        let result = reg.validate_all(
            &RuntimeEvent::PhaseChanged("plan".into()),
            &empty_state(),
            &empty_decisions(),
            &empty_traps(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_all_returns_first_error() {
        let mut reg = ValidatorRegistry::new();
        reg.register(Box::new(AlwaysPass));
        reg.register(Box::new(AlwaysFail));
        reg.register(Box::new(AlwaysPass)); // would pass, but never reached

        let result = reg.validate_all(
            &RuntimeEvent::PhaseChanged("plan".into()),
            &empty_state(),
            &empty_decisions(),
            &empty_traps(),
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.validator_name, "always_fail");
        assert!(err.message.contains("deliberate"));
        assert!(err.suggestion.contains("remove"));
    }

    #[test]
    fn validate_all_stops_at_first_failure() {
        let mut reg = ValidatorRegistry::new();
        reg.register(Box::new(FailOnTaskCreated));
        reg.register(Box::new(AlwaysFail)); // different error — should never fire

        let task = Task {
            id: "T1".into(),
            description: "test".into(),
            status: TaskStatus::Todo,
            superseded_by: None,
        };

        let result = reg.validate_all(
            &RuntimeEvent::TaskCreated(task),
            &empty_state(),
            &empty_decisions(),
            &empty_traps(),
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.validator_name, "fail_on_task_created");
    }

    #[test]
    fn validation_error_contains_correct_validator_name() {
        let err = ValidationError::new(
            "my_validator",
            "something went wrong",
            "try fixing it",
        );
        assert_eq!(err.validator_name, "my_validator");
        assert_eq!(err.message, "something went wrong");
        assert_eq!(err.suggestion, "try fixing it");

        let display = format!("{}", err);
        assert!(display.contains("my_validator"));
        assert!(display.contains("something went wrong"));
        assert!(display.contains("try fixing it"));
    }

    #[test]
    fn validation_error_implements_std_error() {
        let err = ValidationError::new("v", "m", "s");
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn validator_trait_object_send_sync() {
        // Static assertion: Box<dyn Validator> must be Send + Sync.
        fn assert_send_sync<T: Send + Sync>(_t: T) {}
        let v: Box<dyn Validator> = Box::new(AlwaysPass);
        assert_send_sync(v);
    }

    #[test]
    fn validate_all_passes_with_all_passing_validators() {
        let mut reg = ValidatorRegistry::new();
        reg.register(Box::new(AlwaysPass));
        reg.register(Box::new(AlwaysPass));

        let result = reg.validate_all(
            &RuntimeEvent::PhaseChanged("verify".into()),
            &empty_state(),
            &empty_decisions(),
            &empty_traps(),
        );
        assert!(result.is_ok());
    }
}
