use serde::{Serialize, Deserialize};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ButtonScheme {
    Natural,
    Inverted,
}
