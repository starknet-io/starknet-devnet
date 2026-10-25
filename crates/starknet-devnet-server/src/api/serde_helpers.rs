/// A module that deserializes `[]` and `{}` optionally
pub mod empty_params {
    use serde::de::Error as DeError;
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    pub fn deserialize<'de, D>(d: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: Value = Deserialize::deserialize(d)?;

        match value {
            Value::Null => Ok(()),
            Value::Object(obj) if obj.is_empty() => Ok(()),
            Value::Array(arr) if arr.is_empty() => Ok(()),
            other => Err(DeError::custom(format!(
                "expected empty object or array for params; got: {other:?}"
            ))),
        }
    }
}

/// Parameters of `devnet_dump`, which takes none.
pub mod dump_params {
    use serde::de::Error as DeError;
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    pub fn deserialize<'de, D>(d: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(d)? {
            Value::Object(params)
                if let Some(removed) =
                    ["path", "inline"].into_iter().find(|key| params.contains_key(*key)) =>
            {
                Err(DeError::custom(format!(
                    "The `{removed}` parameter of devnet_dump was removed; it returns the dump \
                     and writes no files"
                )))
            }
            params => super::empty_params::deserialize(params).map_err(DeError::custom),
        }
    }
}

pub mod optional_params {
    use serde::de::{DeserializeOwned, Error as DeError};
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    pub fn deserialize<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let value: Value = Deserialize::deserialize(d)?;

        match value {
            Value::Null => Ok(None),
            Value::Object(obj) if obj.is_empty() => Ok(None),
            Value::Array(arr) if arr.is_empty() => Ok(None),
            other => Ok(Some(serde_json::from_value(other).map_err(DeError::custom)?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::de::IntoDeserializer;
    use serde_json::{self, Value};

    use super::empty_params::deserialize;
    use super::optional_params;

    fn test_deserialization(json_str: &str) -> Result<(), serde_json::Error> {
        let value: Value = serde_json::from_str(json_str)?;
        let deserializer = value.into_deserializer();
        deserialize(deserializer)
    }

    #[test]
    fn deserialize_empty_object() {
        let json_str = "{}";
        assert!(test_deserialization(json_str).is_ok());
    }

    #[test]
    fn deserialize_empty_array() {
        let json_str = "[]";
        assert!(test_deserialization(json_str).is_ok());
    }

    #[test]
    fn deserialize_non_empty_object() {
        let json_str = "{\"key\": \"value\"}";
        assert!(test_deserialization(json_str).is_err());
    }

    #[test]
    fn deserialize_non_empty_array() {
        let json_str = "[1, 2, 3]";
        assert!(test_deserialization(json_str).is_err());
    }

    #[test]
    fn deserialize_other_types() {
        let json_str = "\"string\"";
        assert!(test_deserialization(json_str).is_err());
    }

    #[test]
    fn deserialize_null() {
        let value: Value = serde_json::Value::Null;
        let deserializer = value.into_deserializer();
        assert!(deserialize(deserializer).is_ok());
    }
    #[test]
    fn deserialize_to_empty_object_or_with_some_data() {
        let json_str = "[1, 2, 3]";
        let value: Value = serde_json::from_str(json_str).unwrap();
        let deserializer = value.into_deserializer();
        let arr: Option<Vec<u32>> = optional_params::deserialize(deserializer).unwrap();
        assert_eq!(arr, Some(vec![1, 2, 3]));
        let empty_field: Option<()> =
            optional_params::deserialize(Value::Null.into_deserializer()).unwrap();
        assert!(empty_field.is_none());
    }
}
