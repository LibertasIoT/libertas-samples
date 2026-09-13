use alloc::{string::String, vec::Vec};
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Your answer
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum AnswerV2 {
    /// Yes
    Yes,
    /// No
    No,
    /// Not sure
    NotSure,
}

/// Filing status
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum FilingStatusV2 {
    /// Single
    Single,
    /// Married filing jointly
    Joint,
    /// Another status or not sure
    Other,
}

/// Full-year amounts
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum AmountBasisV2 {
    /// Full-year 2026 actual amounts
    /// Use only after you have the complete year, not year-to-date amounts.
    Actual,
    /// Full-year 2026 estimates
    /// Enter your own full-year estimates; this App does not annualize partial-year amounts.
    Estimate,
}

/// Age on December 31, 2026
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum AgeV2 {
    /// Under 25
    Under25,
    /// 25 through 64
    From25To64,
    /// 65 or older, including a January 1, 2027 birthday
    AtLeast65,
    /// Not sure
    NotSure,
}

/// Document owner
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum OwnerV2 {
    /// You
    Taxpayer,
    /// Your spouse
    Spouse,
}

/// Document type
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum IncomeKindV2 {
    /// W-2 wages
    Wage,
    /// Taxable bank interest
    Interest,
}

/// Return destination
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum EditPurposeV2 {
    /// Continue the interview
    Interview,
    /// Return to review
    Review,
}

/// Tax situation
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum CoverageTopicV2 {
    /// Children or other dependents
    Dependents,
    /// Retirement income or retirement savings credits
    Retirement,
    /// Dividends, investments or digital assets
    Investments,
    /// Self-employment, rentals or other income
    Business,
    /// Foreign income, accounts or nonresident situations
    Foreign,
    /// Marketplace health insurance
    Marketplace,
    /// Education, student loans, care or adoption
    Education,
    /// Itemized deductions or charitable carryovers
    Itemizing,
    /// Tips, overtime, car-loan interest or senior deductions
    NewDeductions,
    /// HSA, IRA or other adjustments
    HealthSavings,
    /// Unusual W-2 amounts or additional payroll taxes
    Employment,
    /// Other credits, carryovers or special tax situations
    Other,
}

/// Interview section
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum SectionV2 {
    /// Return setup
    Setup,
    /// About you
    Taxpayer,
    /// About your spouse
    Spouse,
    /// Tax situation
    Coverage,
    /// Income documents
    Income,
    /// Cash donations
    Charity,
    /// Tax payments
    Payments,
    /// Federal review
    Review,
}

/// Estimate status
/// Select the answer that applies to the full 2026 tax year.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum ResultStateV2 {
    /// More answers needed
    Incomplete,
    /// Outside this prototype’s coverage
    Unsupported,
    /// Estimated for the supported scope
    EstimatedForSupportedScope,
}

/// Return setup
/// Complete accepted data; an unvisited page is represented by absence in the return.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct SetupV2 {
    /// Fictional return label
    #[libertas_size(min = 1, max = 80)]
    pub label: String,
    /// Filing status
    pub filing_status: FilingStatusV2,
    /// Use full-year amounts, never unadjusted year-to-date amounts
    pub basis: AmountBasisV2,
}

/// Eligibility
/// Complete accepted data; an unvisited page is represented by absence in the return.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct PersonV2 {
    /// U.S. citizen or resident for the entire year
    pub resident: AnswerV2,
    /// Age at year end
    pub age: AgeV2,
    /// Legally blind at year end
    pub blind: AnswerV2,
    /// Someone else can claim this person as a dependent
    pub claimable: AnswerV2,
}

/// Accepted situation answer
/// Complete accepted data; an unvisited page is represented by absence in the return.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct CoverageAnswerV2 {
    /// Situation
    pub topic: CoverageTopicV2,
    /// Answer
    pub answer: AnswerV2,
}

/// Income document
/// Complete accepted data; an unvisited page is represented by absence in the return.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct IncomeRecordV2 {
    /// Stable document identifier
    pub id: i64,
    /// Document type
    pub kind: IncomeKindV2,
    /// Document owner
    pub owner: OwnerV2,
    /// Fictional employer or bank
    #[libertas_ui_header]
    #[libertas_size(min = 1, max = 80)]
    pub payer: String,
    /// W-2 box 1 wages or ordinary taxable bank interest
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub amount: i64,
    /// Federal income tax withheld, W-2 box 2 or 1099-INT box 4
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub withholding: i64,
    /// W-2 box 5 Medicare wages; zero for interest
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub medicare_wages: i64,
    /// W-2 box 4 Social Security tax; zero for interest
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub social_security_withheld: i64,
}

/// Federal payments
/// Complete accepted data; an unvisited page is represented by absence in the return.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct PaymentsV2 {
    /// 2026 estimated tax payments, including prior-year refund applied
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub estimated: i64,
    /// 2026 extension payment
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub extension: i64,
}

/// Accepted 2026 return
/// Complete accepted data; an unvisited page is represented by absence in the return.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct DraftV2 {
    /// Tax year
    pub tax_year: i32,
    /// Durable return identity
    pub cookie: String,
    /// Accepted revision
    pub revision: i64,
    /// Next document identifier
    pub next_id: i64,
    /// Accepted return setup
    pub setup: Option<SetupV2>,
    /// Accepted taxpayer eligibility
    pub taxpayer: Option<PersonV2>,
    /// Accepted spouse eligibility
    pub spouse: Option<PersonV2>,
    /// Accepted tax situation answers
    /// ----
    /// Situation answer
    pub coverage: Vec<CoverageAnswerV2>,
    /// Accepted income documents
    /// ----
    /// Income document
    #[libertas_size(max = 100)]
    pub records: Vec<IncomeRecordV2>,
    /// Income section accepted as complete
    pub income_complete: bool,
    /// Cash donations made in 2026
    pub charity: Option<AnswerV2>,
    /// Cash donations meet the supported deduction conditions
    pub charity_qualified: Option<AnswerV2>,
    /// Qualifying cash donation amount
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub charity_amount: Option<i64>,
    /// Accepted payments
    pub payments: Option<PaymentsV2>,
    /// Prototype review completed
    pub finished: bool,
}

/// Coverage issue
/// Reopen the named section to review this limitation.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct CoverageIssueV2 {
    /// Section to review
    pub section: SectionV2,
    /// Why the estimate is unavailable
    pub explanation: String,
}

/// 2026 federal estimate
/// Whole-dollar tax calculation from the accepted revision. Missing or unsupported answers prevent a complete estimate.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct EstimateV2 {
    /// Coverage status
    pub state: ResultStateV2,
    /// Accepted input revision
    pub revision: i64,
    /// Rules revision and method
    pub rules: String,
    /// Calculation provenance
    /// ----
    /// Calculation line
    /// Trace metadata; the visible amounts include their calculation explanations in field help.
    #[libertas_hidden]
    pub trace: Vec<CalculationLineV2>,
    /// Items needing attention
    /// ----
    /// Coverage issue
    pub issues: Vec<CoverageIssueV2>,
    /// Total accepted wages
    #[libertas_money("USD", 2)]
    pub wages: i64,
    /// Total accepted taxable interest
    #[libertas_money("USD", 2)]
    pub interest: i64,
    /// Adjusted gross income
    /// Rounded wage and interest totals. This supported scope has no other income or adjustments.
    #[libertas_money("USD", 2)]
    pub agi: Option<i64>,
    /// Standard deduction
    /// 2026 deduction: $16,100 Single or $32,200 Married Filing Jointly. Itemization is outside this prototype.
    #[libertas_money("USD", 2)]
    pub standard_deduction: Option<i64>,
    /// Non-itemizer cash donation deduction
    /// Eligible current-year cash gifts, capped at $1,000 Single or $2,000 jointly and the applicable 60% AGI limit.
    #[libertas_money("USD", 2)]
    pub charity_deduction: Option<i64>,
    /// Taxable income
    /// Adjusted gross income less the standard and supported cash donation deductions, with a minimum of zero.
    #[libertas_money("USD", 2)]
    pub taxable_income: Option<i64>,
    /// Estimated ordinary federal income tax
    /// 2026 continuous rate schedule applied to taxable income. Final IRS tax tables may differ.
    #[libertas_money("USD", 2)]
    pub income_tax: Option<i64>,
    /// Total accepted withholding and payments
    /// W-2 withholding, 1099 withholding, estimated payments and extension payments each round as separate source lines.
    #[libertas_money("USD", 2)]
    pub payments: i64,
    /// Estimated refund before penalties and offsets
    /// Payments minus estimated income tax, with a minimum of zero. No amount has been filed or approved.
    #[libertas_money("USD", 2)]
    pub refund: Option<i64>,
    /// Estimated balance before penalties and interest
    /// Estimated income tax minus payments, with a minimum of zero. Underpayment penalties are not calculated.
    #[libertas_money("USD", 2)]
    pub balance: Option<i64>,
}

/// Calculation provenance
/// Source references describe the accepted revision, never unsaved editor data.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct CalculationLineV2 {
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
