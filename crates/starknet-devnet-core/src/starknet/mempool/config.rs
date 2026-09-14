use std::fmt::Display;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct MempoolOrdering(String);

impl MempoolOrdering {
    pub const FIFO: &'static str = "fifo";
    pub const STARKNET: &'static str = "starknet";
    pub const RANDOM: &'static str = "random";

    pub fn fifo() -> Self {
        Self(Self::FIFO.into())
    }

    pub fn starknet() -> Self {
        Self(Self::STARKNET.into())
    }

    pub fn random() -> Self {
        Self(Self::RANDOM.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MempoolOrdering {
    fn default() -> Self {
        Self::fifo()
    }
}

impl Display for MempoolOrdering {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MempoolOrdering {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || !value.bytes().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || character == b'-'
                    || character == b'_'
            })
        {
            return Err("policy names must contain only lowercase ASCII letters, digits, '-' or \
                        '_'"
            .into());
        }
        Ok(Self(value.into()))
    }
}

impl<'de> Deserialize<'de> for MempoolOrdering {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MempoolConfig {
    pub ordering: MempoolOrdering,
    pub random_seed: u64,
    pub max_transactions_per_block: usize,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self { ordering: MempoolOrdering::fifo(), random_seed: 0, max_transactions_per_block: 500 }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MempoolConfigUpdate {
    pub ordering: Option<MempoolOrdering>,
    pub random_seed: Option<u64>,
    pub max_transactions_per_block: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::MempoolOrdering;

    #[test]
    fn built_in_policy_names_preserve_configuration_serialization() {
        assert_eq!(serde_json::to_string(&MempoolOrdering::fifo()).unwrap(), r#""fifo""#);
        assert_eq!(serde_json::to_string(&MempoolOrdering::starknet()).unwrap(), r#""starknet""#);
        assert_eq!(serde_json::to_string(&MempoolOrdering::random()).unwrap(), r#""random""#);
        assert_eq!(
            serde_json::from_str::<MempoolOrdering>(r#""fifo""#).unwrap(),
            MempoolOrdering::fifo()
        );
    }
}
