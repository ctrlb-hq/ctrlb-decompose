use serde::Serialize;
use std::fmt;

/// Output format mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputMode {
    Human,
    Llm,
    Json,
}

/// Variable type classification for extracted log variables
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum VarType {
    Integer,
    Float,
    Duration,
    Timestamp,
    IPv4,
    IPv6,
    UUID,
    HexID,
    Enum,
    String,
}

impl fmt::Display for VarType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarType::Integer => write!(f, "Integer"),
            VarType::Float => write!(f, "Float"),
            VarType::Duration => write!(f, "Duration"),
            VarType::Timestamp => write!(f, "Timestamp"),
            VarType::IPv4 => write!(f, "IPv4"),
            VarType::IPv6 => write!(f, "IPv6"),
            VarType::UUID => write!(f, "UUID"),
            VarType::HexID => write!(f, "HexID"),
            VarType::Enum => write!(f, "Enum"),
            VarType::String => write!(f, "String"),
        }
    }
}

/// Type alias for pattern IDs from Drain3
pub type PatternID = usize;

/// Format/processing options decoupled from CLI args.
/// Used by format modules and the WASM entry point.
pub struct FormatOptions {
    pub top: usize,
    pub context: usize,
    pub no_color: bool,
    pub no_banner: bool,
    pub output_mode: OutputMode,
    pub source_label: Option<String>,
    pub sim_threshold: f64,
}
