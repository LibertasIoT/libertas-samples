use alloc::string::String;
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Filing status
/// Choose the status for this synthetic interview. Eligibility is not assessed in this prototype.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub enum FilingStatusV1 {
    /// Single
    Single,
    /// Married filing jointly
    MarriedFilingJointly,
}

/// Accepted return setup
/// A complete submitted About-you page; absent pages are stored separately as absence.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct ReturnSetupV1 {
    /// Return label
    #[libertas_size(min = 1, max = 80)]
    pub label: String,
    /// Filing status
    pub filing_status: FilingStatusV1,
}

/// Accepted income overview
/// Temporary aggregate answers for the two-page demonstration, not document-level tax inputs.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct IncomeOverviewV1 {
    /// Total wages
    /// Enter a synthetic annual wage amount in dollars, or explicitly enter zero if there are no wages.
    #[libertas_money("USD", 2)]
    pub wages: i64,
    /// Did you receive taxable bank interest?
    /// Choose an answer. This prototype does not yet collect interest documents or calculate tax.
    pub has_interest: bool,
}

/// Tax return data
/// Versioned storage contract. Step one keeps accepted pages in memory and does not write this record.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub enum TaxAppData {
    /// Accepted draft, version one
    DraftV1 {
        /// Tax year
        tax_year: i32,
        /// Accepted revision
        revision: i64,
        /// Completed About-you page
        setup: Option<ReturnSetupV1>,
        /// Completed income overview
        income: Option<IncomeOverviewV1>,
    },
}
