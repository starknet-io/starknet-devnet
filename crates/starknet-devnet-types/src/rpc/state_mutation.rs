use serde::{Deserialize, Serialize};
use starknet_rs_core::types::ContractClass;

use super::state::{ClassHashPair, ContractNonce, DeployedContract, ReplacedClasses, StorageDiff};
use crate::felt::{ClassHash, CompiledClassHash};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyStateDiffRequest {
    pub state_diff: StateDiffInput,
    #[serde(default)]
    pub classes: Vec<SuppliedClass>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StateDiffInput {
    pub storage_diffs: Vec<StorageDiff>,
    pub nonces: Vec<ContractNonce>,
    pub deployed_contracts: Vec<DeployedContract>,
    pub replaced_classes: Vec<ReplacedClasses>,
    pub declared_classes: Vec<ClassHashPair>,
    pub deprecated_declared_classes: Vec<ClassHash>,
    pub migrated_compiled_classes: Option<Vec<ClassHashPair>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuppliedClass {
    #[serde(deserialize_with = "deserialize_contract_class")]
    pub contract_class: ContractClass,
    pub class_hash: Option<ClassHash>,
    pub compiled_class_hash: Option<CompiledClassHash>,
}

fn deserialize_contract_class<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ContractClass, D::Error> {
    use serde::de::Error;

    let value = serde_json::Value::deserialize(deserializer)?;
    let is_sierra = value.get("sierra_program").is_some();
    let class_fields: &[&str] = if is_sierra {
        &["sierra_program", "contract_class_version", "entry_points_by_type", "abi"]
    } else {
        &["program", "entry_points_by_type", "abi"]
    };
    reject_unknown_fields(&value, class_fields).map_err(D::Error::custom)?;
    if let Some(entry_points) = value.get("entry_points_by_type") {
        reject_unknown_fields(entry_points, &["CONSTRUCTOR", "EXTERNAL", "L1_HANDLER"])
            .map_err(D::Error::custom)?;
        if let Some(groups) = entry_points.as_object() {
            for entries in groups.values().filter_map(serde_json::Value::as_array) {
                for entry in entries {
                    let fields: &[&str] = if is_sierra {
                        &["selector", "function_idx"]
                    } else {
                        &["selector", "offset"]
                    };
                    reject_unknown_fields(entry, fields).map_err(D::Error::custom)?;
                }
            }
        }
    }
    serde_json::from_value(value).map_err(D::Error::custom)
}

fn reject_unknown_fields(value: &serde_json::Value, allowed: &[&str]) -> Result<(), String> {
    if let Some(object) = value.as_object() {
        for field in object.keys() {
            if !allowed.contains(&field.as_str()) {
                return Err(format!("contract_class: unknown field `{field}`"));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct AppliedClass {
    pub class_hash: ClassHash,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled_class_hash: Option<CompiledClassHash>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyStateDiffResponse {
    pub classes: Vec<AppliedClass>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ApplyStateDiffRequest;

    #[test]
    fn empty_diff_defaults_and_rejects_unknown_fields() {
        let request: ApplyStateDiffRequest =
            serde_json::from_value(json!({ "state_diff": {} })).unwrap();
        assert!(request.classes.is_empty());
        assert!(request.state_diff.storage_diffs.is_empty());
        for invalid in [
            json!({}),
            json!({ "state_diff": {}, "extra": 1 }),
            json!({ "state_diff": { "extra": [] } }),
            json!({ "state_diff": { "nonces": [{
                "contract_address": "0x1", "nonce": "0x2", "extra": 1
            }] } }),
        ] {
            assert!(serde_json::from_value::<ApplyStateDiffRequest>(invalid).is_err());
        }
    }

    #[test]
    fn supplied_class_rejects_unknown_fields_without_changing_other_rpc_class_parsers() {
        let class = json!({
            "program": "", "entry_points_by_type": { "CONSTRUCTOR": [], "EXTERNAL": [], "L1_HANDLER": [] }
        });
        let valid = json!({ "state_diff": {}, "classes": [{ "contract_class": class }] });
        assert!(serde_json::from_value::<ApplyStateDiffRequest>(valid.clone()).is_ok());
        let mut extra_field = valid.clone();
        extra_field["classes"][0]["contract_class"]["extra"] = json!(1);
        let mut extra_group = valid.clone();
        extra_group["classes"][0]["contract_class"]["entry_points_by_type"]["extra"] = json!([]);
        let mut extra_entry = valid.clone();
        extra_entry["classes"][0]["contract_class"]["entry_points_by_type"]["EXTERNAL"] =
            json!([{ "selector": "0x1", "offset": "0x0", "extra": 1 }]);
        let mut extra_wrapper = valid;
        extra_wrapper["classes"][0]["extra"] = json!(1);
        for invalid in [extra_field, extra_group, extra_entry, extra_wrapper] {
            assert!(serde_json::from_value::<ApplyStateDiffRequest>(invalid).is_err());
        }
    }
}
