//! Type-safe device identifier for the rust-daq ecosystem.
//!
//! Replaces bare `String` device IDs throughout the codebase for:
//! - **Type safety**: Can't accidentally pass a parameter name where a device ID is expected
//! - **Allocation efficiency**: `Arc<str>` clones are reference-count bumps, not heap copies

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Type-safe device identifier, backed by `Arc<str>` for cheap cloning.
///
/// Replaces bare `String` device IDs throughout the codebase for:
/// - **Type safety**: Can't accidentally pass a parameter name where a device ID is expected
/// - **Allocation efficiency**: `Arc<str>` clones are reference-count bumps, not heap copies
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(Arc<str>);

impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self(Arc::from(s)))
    }
}

impl DeviceId {
    /// Create a new `DeviceId` from anything convertible to `Arc<str>`.
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }

    /// View the device ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DeviceId {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<String> for DeviceId {
    fn from(s: String) -> Self {
        Self(Arc::from(s))
    }
}

impl From<DeviceId> for String {
    fn from(id: DeviceId) -> Self {
        id.0.to_string()
    }
}

impl AsRef<str> for DeviceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for DeviceId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for DeviceId {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for DeviceId {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for DeviceId {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl PartialOrd for DeviceId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DeviceId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_str() {
        let id = DeviceId::from("camera_1");
        assert_eq!(id.as_str(), "camera_1");
    }

    #[test]
    fn test_from_string() {
        let id = DeviceId::from("stage_x".to_string());
        assert_eq!(id.as_str(), "stage_x");
    }

    #[test]
    fn test_clone_is_cheap() {
        let id1 = DeviceId::new("detector");
        let id2 = id1.clone();
        assert_eq!(id1, id2);
        // Arc clone = ref count bump, not heap copy
        assert!(Arc::ptr_eq(&id1.0, &id2.0));
    }

    #[test]
    fn test_display() {
        let id = DeviceId::new("power_meter");
        assert_eq!(format!("{id}"), "power_meter");
    }

    #[test]
    fn test_serde_roundtrip() {
        let id = DeviceId::new("laser_1");
        let json = serde_json::to_string(&id).expect("serialize should succeed");
        assert_eq!(json, "\"laser_1\"");
        let back: DeviceId = serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(id, back);
    }

    #[test]
    fn test_eq_str() {
        let id = DeviceId::new("cam");
        assert!(id == "cam");
        assert!(id == *"cam");
    }

    #[test]
    fn test_hash_map_key() {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(DeviceId::new("a"), 1);
        map.insert(DeviceId::new("b"), 2);
        assert_eq!(map[&DeviceId::new("a")], 1);
    }

    #[test]
    fn test_into_string() {
        let id = DeviceId::new("motor_1");
        let s: String = id.into();
        assert_eq!(s, "motor_1");
    }

    #[test]
    fn test_borrow_str() {
        use std::borrow::Borrow;
        let id = DeviceId::new("sensor");
        let s: &str = id.borrow();
        assert_eq!(s, "sensor");
    }

    #[test]
    fn test_ordering() {
        let a = DeviceId::new("alpha");
        let b = DeviceId::new("beta");
        assert!(a < b);
    }
}
