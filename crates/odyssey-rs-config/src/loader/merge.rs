//! JSON merge helpers for layered configuration.

use serde_json::Value;

/// Merge overlay values into the base, recursively overriding objects.
pub(super) fn merge_json_values(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(existing) => merge_json_values(existing, value),
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base_slot, overlay_value) => {
            *base_slot = overlay_value.clone();
        }
    }
}

/// Merge overlay values into base, honoring constraints when provided.
pub(super) fn merge_json_with_constraints(
    base: &mut Value,
    overlay: &Value,
    constraints: Option<&Value>,
) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            let constraint_map = match constraints {
                Some(Value::Object(map)) => Some(map),
                Some(_) => return,
                None => None,
            };

            for (key, value) in overlay_map {
                let key_constraint = constraint_map.and_then(|map| map.get(key));
                match key_constraint {
                    None => match base_map.get_mut(key) {
                        Some(existing) => merge_json_with_constraints(existing, value, None),
                        None => {
                            base_map.insert(key.clone(), value.clone());
                        }
                    },
                    Some(Value::Object(_)) => {
                        let base_entry = base_map
                            .entry(key.clone())
                            .or_insert_with(|| Value::Object(serde_json::Map::new()));
                        merge_json_with_constraints(base_entry, value, key_constraint);
                    }
                    Some(_) => {
                        // Constrained key; skip overrides.
                    }
                }
            }
        }
        (base_slot, overlay_value) => {
            if constraints.is_none() {
                merge_json_values(base_slot, overlay_value);
            }
        }
    }
}
