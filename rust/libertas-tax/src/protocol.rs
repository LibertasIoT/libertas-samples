use crate::*;
use alloc::{string::String, vec::Vec};
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Tax interview section
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport,
)]
pub enum TaxSection {
    /// About this return
    Setup,
    /// Filers
    People,
    /// Children and people you support
    Children,
    /// Income documents
    Income,
    /// Adjustments
    Adjustments,
    /// Deductions
    Deductions,
    /// Education, care and retirement savings credits
    Credits,
    /// Remaining situations
    Screening,
    /// Federal payments
    Payments,
    /// Review
    Review,
}

/// Tax interview
/// Accepted pages are saved. Unsubmitted edits are discarded when you leave. Synthetic data only; no return is filed.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum TaxInterviewProtocol {
    /// Start or resume
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    OpenInterview,
    /// About this return
    #[libertas_response]
    #[libertas_workflow_request("SaveSetup")]
    BeginSetup {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// About this return
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveSetup {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// About this return
        value: FederalSetup,
        /// Confirm retained amounts use the changed full-year basis
        #[libertas_default(false)]
        confirm_basis_change: bool,
    },
    /// About the filers
    #[libertas_response]
    #[libertas_workflow_request("SavePeople")]
    BeginPeople {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// About the filers
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SavePeople {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// About the filers
        value: FederalPeople,
    },
    /// Adjustments to income
    #[libertas_response]
    #[libertas_workflow_request("SaveAdjustments")]
    BeginAdjustments {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// Adjustments to income
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveAdjustments {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Adjustments to income
        value: FederalAdjustments,
    },
    /// Deductions
    #[libertas_response]
    #[libertas_workflow_request("SaveDeductions")]
    BeginDeductions {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// Deductions
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveDeductions {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Deductions
        value: FederalDeductions,
    },
    /// Education, care and retirement savings credits
    #[libertas_response]
    #[libertas_workflow_request("SaveCredits")]
    BeginCredits {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
        /// Students on this return
        /// ----
        /// Student
        students: Vec<FederalStudent>,
    },
    /// Education, care and retirement savings credits
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveCredits {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Education, care and retirement savings credits
        value: FederalCredits,
    },
    /// Check remaining situations
    #[libertas_response]
    #[libertas_workflow_request("SaveScreening")]
    BeginScreening {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// Check remaining situations
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveScreening {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Check remaining situations
        value: FederalScreening,
    },
    /// Federal payments
    #[libertas_response]
    #[libertas_workflow_request("SavePayments")]
    BeginPayments {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// Federal payments
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SavePayments {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Federal payments
        value: FederalPayments,
    },
    /// Child or person you support
    #[libertas_response]
    #[libertas_workflow_request("SaveDependent")]
    BeginDependent {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// Child or person you support
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveDependent {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Child or person you support
        value: FederalDependent,
    },
    /// Income document
    #[libertas_response]
    #[libertas_workflow_request("SaveIncome")]
    BeginIncome {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// Income document
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveIncome {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        #[libertas_calculation_request("PreviewIncome")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        #[libertas_calculation_request("PreviewIncome")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Income document
        #[libertas_calculation_request("PreviewIncome")]
        value: FederalIncomeEntry,
        /// Document amount before return-level tax calculations
        /// Social Security is gross benefits; investment sales are net gain/loss. This does not determine taxable income or final tax.
        #[libertas_money("USD", 2)]
        #[libertas_read_only]
        preview: Option<i64>,
    },
    /// Child or person you supports
    #[libertas_response]
    #[libertas_next_request("AddDependent,SelectDependent,ContinueSection,Navigate,OpenInterview")]
    Children {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
        /// Accepted records
        /// ----
        /// Record
        records: Vec<FederalDependent>,
    },
    /// Add Child or person you support
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    AddDependent {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
    },
    /// Choose Child or person you support
    #[libertas_response]
    #[libertas_workflow_request("ChooseDependent")]
    BeginChooseDependent {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
        /// Accepted records
        /// ----
        /// Record
        records: Vec<FederalDependent>,
    },
    /// Edit or remove Child or person you support
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SelectDependent {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
    },
    /// Choose Child or person you support
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    ChooseDependent {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Accepted records
        /// ----
        /// Record
        #[libertas_read_only]
        #[libertas_copy_from("$.records")]
        records: Vec<FederalDependent>,
        /// Record
        #[libertas_enum_source("^.records")]
        selection: u32,
        /// Remove this record
        #[libertas_default(false)]
        remove: bool,
    },
    /// Income documents
    #[libertas_response]
    #[libertas_next_request("AddIncome,SelectIncome,ContinueSection,Navigate,OpenInterview")]
    IncomeDocuments {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
        /// Accepted records
        /// ----
        /// Record
        records: Vec<FederalIncomeEntry>,
    },
    /// Add Income document
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    AddIncome {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
    },
    /// Choose Income document
    #[libertas_response]
    #[libertas_workflow_request("ChooseIncome")]
    BeginChooseIncome {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
        /// Accepted records
        /// ----
        /// Record
        records: Vec<FederalIncomeEntry>,
    },
    /// Edit or remove Income document
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SelectIncome {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
    },
    /// Choose Income document
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    ChooseIncome {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Accepted records
        /// ----
        /// Record
        #[libertas_read_only]
        #[libertas_copy_from("$.records")]
        records: Vec<FederalIncomeEntry>,
        /// Record
        #[libertas_enum_source("^.records")]
        selection: u32,
        /// Remove this record
        #[libertas_default(false)]
        remove: bool,
    },
    /// All records entered
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    ContinueSection {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
    },
    /// Back
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    Back {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Previous page
        #[libertas_hidden]
        #[libertas_copy_from("$.previous")]
        previous: i32,
    },
    /// Review a section
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    Navigate {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Section
        section: TaxSection,
    },
    /// Review your federal estimate
    #[libertas_response]
    #[libertas_next_request("Navigate,StartFinish,OpenInterview")]
    Review {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Federal estimate and coverage
        result: FederalResult,
    },
    /// Finish review
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    StartFinish {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
    },
    /// Finish prototype review
    #[libertas_response]
    #[libertas_workflow_request("Finish")]
    BeginFinish {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        progress: Vec<u8>,
    },
    /// Finish prototype review
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,BeginDependent,SaveDependent,BeginIncome,SaveIncome,Children,IncomeDocuments,BeginChooseDependent,ChooseDependent,BeginChooseIncome,ChooseIncome,Review,BeginFinish,Finish,Finished,Problem"
    )]
    Finish {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Current section
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Return to review after saving
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.review")]
        review: bool,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
    },
    /// Prototype review completed
    #[libertas_response]
    Finished {
        /// Completion status
        #[libertas_formatted_text]
        message: Vec<u8>,
        /// Federal estimate
        result: FederalResult,
    },
    /// Please review your answer
    #[libertas_response]
    #[libertas_error]
    Problem {
        /// What needs attention
        #[libertas_formatted_text]
        message: Vec<u8>,
    },
    /// Calculate document preview
    #[libertas_request]
    #[libertas_next_response("IncomePreview,Problem")]
    PreviewIncome {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Complete income document
        #[libertas_copy_from("$.value")]
        value: FederalIncomeEntry,
    },
    /// Document preview
    #[libertas_response]
    IncomePreview {
        /// Document amount before return-level calculations
        #[libertas_money("USD", 2)]
        #[libertas_copy_to("$.preview")]
        preview: Option<i64>,
    },
}
