use alloc::{string::String, vec::Vec};
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Answer
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum Answer {
    /// Yes
    Yes,
    /// No
    No,
    /// Not sure
    NotSure,
}

/// AmountBasis
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum AmountBasis {
    /// Full-year 2026 actual amounts
    /// Use only after you have the complete year, not year-to-date amounts.
    Actual,
    /// Full-year 2026 estimates
    /// Enter your own full-year estimates; this App does not annualize partial-year amounts.
    Estimate,
}

/// Owner
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum Owner {
    /// You
    Taxpayer,
    /// Your spouse
    Spouse,
}

/// ResultState
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum ResultState {
    /// More answers needed
    Incomplete,
    /// Outside this prototype’s coverage
    Unsupported,
    /// Estimated for the supported scope
    EstimatedForSupportedScope,
}

/// CalculationLine
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct CalculationLine {
    /// Stable calculation identifier
    pub id: String,
    /// Result
    #[libertas_money("USD", 2)]
    pub amount: i64,
    /// Inputs
    /// ----
    /// Accepted input or earlier calculation identifier
    pub inputs: Vec<String>,
    /// Rule identifier
    pub rule: String,
    /// Authoritative source
    pub source: String,
    /// Explanation
    pub explanation: String,
}
