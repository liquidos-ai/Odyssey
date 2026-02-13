//! Tool output redaction and truncation policy.

use serde_json::Value;

/// Policy that redacts and truncates tool outputs for safety.
#[derive(Debug, Clone)]
pub struct ToolOutputPolicy {
    /// Maximum size of string fields in bytes.
    pub max_string_bytes: usize,
    /// Maximum number of elements in arrays.
    pub max_array_len: usize,
    /// Maximum number of object entries.
    pub max_object_entries: usize,
    /// Key names whose values should be redacted.
    pub redact_keys: Vec<String>,
    /// Value patterns that should be redacted.
    pub redact_values: Vec<String>,
    /// Replacement text for redacted values.
    pub replacement: String,
}

impl Default for ToolOutputPolicy {
    /// Default output policy with conservative limits.
    fn default() -> Self {
        Self {
            max_string_bytes: 32 * 1024,
            max_array_len: 256,
            max_object_entries: 256,
            redact_keys: Vec::new(),
            redact_values: Vec::new(),
            replacement: "[REDACTED]".to_string(),
        }
    }
}

impl ToolOutputPolicy {
    /// Apply the policy to a JSON value.
    pub fn apply(&self, value: Value) -> Value {
        self.apply_value(value)
    }

    /// Recursively apply output policy to nested values.
    fn apply_value(&self, value: Value) -> Value {
        match value {
            Value::String(value) => Value::String(self.apply_string(value)),
            Value::Array(values) => {
                let trimmed = values
                    .into_iter()
                    .take(self.max_array_len)
                    .map(|value| self.apply_value(value))
                    .collect();
                Value::Array(trimmed)
            }
            Value::Object(values) => {
                let mut trimmed =
                    serde_json::Map::with_capacity(values.len().min(self.max_object_entries));
                for (key, value) in values.into_iter().take(self.max_object_entries) {
                    let value = if self.should_redact_key(&key) {
                        Value::String(self.truncate_string(self.replacement.clone()))
                    } else {
                        self.apply_value(value)
                    };
                    trimmed.insert(key, value);
                }
                Value::Object(trimmed)
            }
            value => value,
        }
    }

    /// Apply redaction and truncation to a string value.
    fn apply_string(&self, value: String) -> String {
        if self.should_redact_value(&value) {
            return self.truncate_string(self.replacement.clone());
        }
        self.truncate_string(value)
    }

    /// Determine whether a key should be redacted.
    fn should_redact_key(&self, key: &str) -> bool {
        self.redact_keys
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(key))
    }

    /// Determine whether a value should be redacted.
    fn should_redact_value(&self, value: &str) -> bool {
        if self.redact_values.is_empty() {
            return false;
        }
        let lowered = value.to_ascii_lowercase();
        self.redact_values.iter().any(|entry| {
            let pattern = entry.to_ascii_lowercase();
            lowered.contains(&pattern)
        })
    }

    /// Truncate a string to the maximum byte size boundary.
    fn truncate_string(&self, value: String) -> String {
        let max_bytes = self.max_string_bytes;
        if max_bytes == 0 {
            return String::new();
        }
        if value.len() <= max_bytes {
            return value;
        }
        let mut end = 0;
        for (idx, ch) in value.char_indices() {
            let next = idx + ch.len_utf8();
            if next > max_bytes {
                break;
            }
            end = next;
        }
        value[..end].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::ToolOutputPolicy;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn apply_redacts_and_truncates_strings() {
        let policy = ToolOutputPolicy {
            max_string_bytes: 4,
            max_array_len: 8,
            max_object_entries: 8,
            redact_keys: vec!["secret".to_string()],
            redact_values: vec!["token".to_string()],
            replacement: "[X]".to_string(),
        };
        let input = json!({
            "secret": "value",
            "keep": "token-value",
        });

        let output = policy.apply(input);

        let expected = json!({
            "keep": "[X]",
            "secret": "[X]",
        });
        assert_eq!(output, expected);
    }

    #[test]
    fn apply_truncates_arrays() {
        let policy = ToolOutputPolicy {
            max_string_bytes: 64,
            max_array_len: 2,
            max_object_entries: 8,
            redact_keys: Vec::new(),
            redact_values: Vec::new(),
            replacement: "[X]".to_string(),
        };
        let input = json!({
            "list": ["first", "second", "third"],
        });

        let output = policy.apply(input);

        let expected = json!({
            "list": ["first", "second"],
        });
        assert_eq!(output, expected);
    }
}
