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
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    LibertasAvroDecode,
    LibertasAvroEncode,
    LibertasExport,
    libertas_macros::VariantIndex,
)]
pub enum TaxInterviewProtocol {
    /// Start or resume
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        /// Confirm that changing the full-year basis erases saved later sections
        #[libertas_default(false)]
        confirm_basis_change: bool,
    },
    /// About the filers
    #[libertas_response]
    #[libertas_workflow_request("SavePeople")]
    BeginPeople {
        /// Allowed spouse branches
        #[libertas_hidden]
        allowed_spouse: Vec<i32>,
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SavePeople {
        /// Allowed spouse branches
        #[libertas_hidden]
        #[libertas_copy_from("$.allowed_spouse")]
        allowed_spouse: Vec<i32>,
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
    /// Children and people you support
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveDependents {
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
        /// Children and people you support
        /// ----
        /// Child or person you support
        #[libertas_size(max = 30)]
        records: Vec<FederalDependent>,
    },
    /// Income documents
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("Back,OpenInterview")]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
    )]
    SaveIncomes {
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
        /// Income documents
        /// ----
        /// Income document
        #[libertas_size(max = 100)]
        records: Vec<IncomeEditorItem>,
    },
    /// Back
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        #[libertas_constrained_by("$.allowed_sections")]
        section: TaxSection,
        /// Available sections
        #[libertas_hidden]
        #[libertas_copy_from("$.allowed_sections")]
        allowed_sections: Vec<i32>,
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
        /// Available sections
        #[libertas_hidden]
        allowed_sections: Vec<i32>,
    },
    /// Finish review
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        "BeginSetup,SaveSetup,BeginPeople,SavePeople,BeginAdjustments,SaveAdjustments,BeginDeductions,SaveDeductions,BeginCredits,SaveCredits,BeginScreening,SaveScreening,BeginPayments,SavePayments,SaveDependents,SaveIncomes,Review,BeginFinish,Finish,Finished,Problem"
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
        /// Income facts required to calculate this document's amount
        #[libertas_copy_from("^<IncomeEditorItem>.value")]
        value: IncomeAmountInput,
    },
    /// Document preview
    #[libertas_response]
    IncomePreview {
        /// Document amount before return-level calculations
        #[libertas_money("USD", 2)]
        #[libertas_copy_to("^<IncomeEditorItem>.preview")]
        preview: Option<i64>,
    },
    /// Open tax interview
    #[libertas_request]
    #[libertas_default_request]
    #[libertas_next_response("Actions,Problem")]
    Enter,
    /// Tax interview actions
    #[libertas_response]
    #[libertas_next_request("OpenInterview,ReviewAccepted")]
    #[libertas_constrained_by("$.allowed_actions")]
    Actions {
        /// Available actions
        #[libertas_hidden]
        allowed_actions: Vec<i32>,
    },
    /// Review accepted answers and estimate
    #[libertas_request]
    #[libertas_next_response("Review,Problem")]
    ReviewAccepted,
}

/// Income document with an automatically calculated amount
/// The preview is transient editor data; only the income record is persisted.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct IncomeEditorItem {
    /// Income document
    #[libertas_ui_header]
    #[libertas_calculation_request("PreviewIncome")]
    pub value: FederalIncomeEntry,
    /// Document amount before return-level tax calculations
    /// Social Security is gross benefits; investment sales are net gain/loss. This does not determine taxable income or final tax.
    #[libertas_money("USD", 2)]
    #[libertas_read_only]
    pub preview: Option<i64>,
}

/// Income facts used for a document amount preview
/// This reduced type excludes the record identity, payer label and income owner.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub struct IncomeAmountInput {
    /// Document type and facts
    pub income: FederalIncome,
}
