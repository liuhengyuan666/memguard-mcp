use serde::{Deserialize, Serialize, Serializer, Deserializer};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub status: TaskStatus,
}

/// Payload for TaskCreated events — does not include `status` because
/// new tasks are always created as `Todo` regardless of caller input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCreatePayload {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdrStatus {
    Proposed,
    Accepted,
    Superseded,
    Rejected,
    Archived,
}

impl Serialize for AdrStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            AdrStatus::Proposed => serializer.serialize_str("Proposed"),
            AdrStatus::Accepted => serializer.serialize_str("Accepted"),
            AdrStatus::Superseded => serializer.serialize_str("Superseded"),
            AdrStatus::Rejected => serializer.serialize_str("Rejected"),
            AdrStatus::Archived => serializer.serialize_str("Archived"),
        }
    }
}

impl<'de> Deserialize<'de> for AdrStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "active" => Ok(AdrStatus::Accepted),
            "proposed" => Ok(AdrStatus::Proposed),
            "superseded" => Ok(AdrStatus::Superseded),
            "rejected" => Ok(AdrStatus::Rejected),
            "archived" => Ok(AdrStatus::Archived),
            "accepted" => Ok(AdrStatus::Accepted),
            other => Err(serde::de::Error::custom(format!("unknown ADR status: {}", other))),
        }
    }
}

impl std::fmt::Display for AdrStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdrStatus::Proposed => write!(f, "Proposed"),
            AdrStatus::Accepted => write!(f, "Accepted"),
            AdrStatus::Superseded => write!(f, "Superseded"),
            AdrStatus::Rejected => write!(f, "Rejected"),
            AdrStatus::Archived => write!(f, "Archived"),
        }
    }
}

impl std::str::FromStr for AdrStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(AdrStatus::Accepted),
            "proposed" => Ok(AdrStatus::Proposed),
            "superseded" => Ok(AdrStatus::Superseded),
            "rejected" => Ok(AdrStatus::Rejected),
            "archived" => Ok(AdrStatus::Archived),
            "accepted" => Ok(AdrStatus::Accepted),
            other => Err(format!("unknown ADR status: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub struct ADR {
    pub id: String,
    pub title: String,
    pub status: AdrStatus,
    pub context: String,
    pub decision: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Trap {
    pub error_signature: String,
    pub context: String,
    pub solution: String,
    pub root_cause: String,
    pub prevention: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub current_phase: String,
    pub active_tasks: Vec<Task>,
    pub done_tasks: Vec<Task>,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum RuntimeEvent {
    TaskCreated(Task),
    TaskUpdated {
        task_id: String,
        new_status: TaskStatus,
    },
    AdrCommitted(ADR),
    TrapRecorded(Trap),
    PhaseChanged(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_blocked_serde() {
        let status = TaskStatus::Blocked;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"Blocked\"");
        let decoded: TaskStatus = serde_json::from_str(&json).unwrap();
        assert!(matches!(decoded, TaskStatus::Blocked));
    }

    #[test]
    fn test_task_status_existing_variants_unchanged() {
        assert_eq!(serde_json::to_string(&TaskStatus::Todo).unwrap(), "\"Todo\"");
        assert_eq!(serde_json::to_string(&TaskStatus::InProgress).unwrap(), "\"InProgress\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Done).unwrap(), "\"Done\"");
    }

    #[test]
    fn test_adr_status_active_maps_to_accepted() {
        let decoded: AdrStatus = serde_json::from_str("\"active\"").unwrap();
        assert_eq!(decoded, AdrStatus::Accepted);
    }

    #[test]
    fn test_adr_status_accepted_serializes_correctly() {
        let json = serde_json::to_string(&AdrStatus::Accepted).unwrap();
        assert_eq!(json, "\"Accepted\"");
    }

    #[test]
    fn test_adr_status_all_variants_roundtrip() {
        for status in [
            AdrStatus::Proposed,
            AdrStatus::Accepted,
            AdrStatus::Superseded,
            AdrStatus::Rejected,
            AdrStatus::Archived,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let decoded: AdrStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, status, "round-trip failed for {:?}", status);
        }
    }

    #[test]
    fn test_adr_status_backward_compat_proposed() {
        let decoded: AdrStatus = serde_json::from_str("\"proposed\"").unwrap();
        assert_eq!(decoded, AdrStatus::Proposed);
    }

    #[test]
    fn test_adr_status_backward_compat_superseded() {
        let decoded: AdrStatus = serde_json::from_str("\"superseded\"").unwrap();
        assert_eq!(decoded, AdrStatus::Superseded);
    }

    #[test]
    fn test_adr_status_backward_compat_rejected() {
        let decoded: AdrStatus = serde_json::from_str("\"rejected\"").unwrap();
        assert_eq!(decoded, AdrStatus::Rejected);
    }

    #[test]
    fn test_adr_status_backward_compat_archived() {
        let decoded: AdrStatus = serde_json::from_str("\"archived\"").unwrap();
        assert_eq!(decoded, AdrStatus::Archived);
    }

    #[test]
    fn test_trap_backward_compat() {
        let old_json = r#"{"error_signature":"E1","context":"C1","solution":"S1"}"#;
        let trap: Trap = serde_json::from_str(old_json).unwrap();
        assert_eq!(trap.error_signature, "E1");
        assert_eq!(trap.context, "C1");
        assert_eq!(trap.solution, "S1");
        assert_eq!(trap.root_cause, "");
        assert_eq!(trap.prevention, "");
    }

    #[test]
    fn test_trap_full_serde() {
        let trap = Trap {
            error_signature: "sig".to_string(),
            context: "ctx".to_string(),
            solution: "sol".to_string(),
            root_cause: "root".to_string(),
            prevention: "prev".to_string(),
        };
        let json = serde_json::to_string(&trap).unwrap();
        let decoded: Trap = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.error_signature, "sig");
        assert_eq!(decoded.context, "ctx");
        assert_eq!(decoded.solution, "sol");
        assert_eq!(decoded.root_cause, "root");
        assert_eq!(decoded.prevention, "prev");
    }
}
