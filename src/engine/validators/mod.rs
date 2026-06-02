//! Concrete validators for MemGuard runtime event application.
//!
//! Each validator implements the `Validator` trait and guards against a
//! specific class of invalid state transitions.  They are designed to be
//! independently testable and can be registered with `ValidatorRegistry`
//! in any order.

pub mod empty_task_id;
pub mod duplicate_task_id;
pub mod adr_active_conflict;
pub mod adr_rejected_repeat;
pub mod adr_invalid_transition;

pub use self::{
    empty_task_id::EmptyTaskId,
    duplicate_task_id::DuplicateTaskId,
    adr_active_conflict::AdrActiveConflict,
    adr_rejected_repeat::AdrRejectedRepeat,
    adr_invalid_transition::AdrInvalidTransition,
};

use crate::models::ADR;

/// Compute a content hash for an ADR based on its title and decision fields.
/// Used by ADR conflict validators to detect identical re-submissions.
pub(crate) fn content_hash(adr: &ADR) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    adr.title.trim().to_lowercase().hash(&mut h);
    adr.decision.trim().to_lowercase().hash(&mut h);
    h.finish()
}
