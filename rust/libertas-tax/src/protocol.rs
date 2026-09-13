use crate::{
    AnswerV2, CoverageTopicV2, EditPurposeV2, EstimateV2, IncomeOverviewV1, IncomeRecordV2,
    OwnerV2, PaymentsV2, PersonV2, ReturnSetupV1, SectionV2, SetupV2,
};
use alloc::{string::String, vec::Vec};
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Tax interview
/// A saved, bounded 2026 federal interview and estimate using synthetic answers.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub enum TaxInterviewProtocol {
    /// Open interview
    /// Start or resume this task's saved synthetic return.
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    OpenInterviewV1,

    /// About-you entry
    #[libertas_response]
    #[libertas_workflow_request("SaveAboutV1")]
    BeginAboutV1 {
        #[libertas_hidden]
        cookie: String,
        #[libertas_hidden]
        revision: i64,
        #[libertas_formatted_text]
        progress: Vec<u8>,
    },
    /// About you
    /// Use a fictional return label. No personal identifiers are needed.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_next_response("BeginIncomeV1,SaveIncomeV1,ProblemV1")]
    SaveAboutV1 {
        #[libertas_hidden]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        #[libertas_hidden]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Your return
        setup: ReturnSetupV1,
    },
    /// Income entry
    #[libertas_response]
    #[libertas_workflow_request("SaveIncomeV1")]
    BeginIncomeV1 {
        #[libertas_hidden]
        cookie: String,
        #[libertas_hidden]
        revision: i64,
        #[libertas_formatted_text]
        progress: Vec<u8>,
    },
    /// Income
    /// Use synthetic amounts only. Back discards unsent changes and reopens the accepted About-you page.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackToAboutV1")]
    #[libertas_next_response("SummaryV1,ProblemV1")]
    SaveIncomeV1 {
        #[libertas_hidden]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        #[libertas_hidden]
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Income answers
        income: IncomeOverviewV1,
    },
    /// Edit About you
    #[libertas_request]
    #[libertas_next_response("SaveAboutV1,ProblemV1")]
    BackToAboutV1 {
        #[libertas_hidden]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
    },
    /// Edit income
    #[libertas_request]
    #[libertas_next_response("SaveIncomeV1,ProblemV1")]
    BackToIncomeV1 {
        #[libertas_hidden]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
    },
    /// Your synthetic answers
    /// This is an interview demonstration, not a tax estimate or filed return. Answers are temporary.
    #[libertas_response]
    #[libertas_next_request("BackToAboutV1,BackToIncomeV1")]
    SummaryV1 {
        #[libertas_hidden]
        cookie: String,
        /// Prototype status
        #[libertas_formatted_text]
        status: Vec<u8>,
        /// About you
        setup: ReturnSetupV1,
        /// Income
        income: IncomeOverviewV1,
    },
    /// Please review your answer
    #[libertas_response]
    #[libertas_error]
    ProblemV1 {
        /// What needs attention
        #[libertas_formatted_text]
        message: Vec<u8>,
    },
    /// Continue your 2026 interview
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_workflow_request("SaveSetupV2")]
    BeginSetupV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        progress: Vec<u8>,
    },
    /// Your 2026 return
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackV2,ReloadV2")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    SaveSetupV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Return setup
        setup: SetupV2,
        /// If changing the amount basis, I confirm the retained amounts use the new basis
        /// Needed only when switching between full-year estimates and actuals. Income must be reviewed again; donation amount and payment pages must be resubmitted.
        #[libertas_default(false)]
        confirm_basis_change: bool,
    },
    /// Continue your 2026 interview
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_workflow_request("SavePersonV2")]
    BeginPersonV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        progress: Vec<u8>,
    },
    /// Eligibility
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackV2,ReloadV2")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    SavePersonV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// About this person
        person: PersonV2,
    },
    /// Continue your 2026 interview
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_workflow_request("SaveQuestionV2")]
    BeginQuestionV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        progress: Vec<u8>,
        /// Question
        #[libertas_formatted_text]
        question: Vec<u8>,
        /// Situation
        #[libertas_hidden]
        topic: CoverageTopicV2,
    },
    /// Your tax situation
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackV2,ReloadV2")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    SaveQuestionV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Question
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.question")]
        question: Vec<u8>,
        /// Situation
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.topic")]
        topic: CoverageTopicV2,
        /// Your answer
        answer: AnswerV2,
    },
    /// Your income documents
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_next_request(
        "AddWageV2,AddInterestV2,ChooseDocumentV2,ContinueIncomeV2,NavigateV2,ReloadV2"
    )]
    IncomeOverviewV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Add each W-2 and taxable bank-interest record, then continue
        #[libertas_formatted_text]
        progress: Vec<u8>,
        /// Accepted documents
        /// ----
        /// Document
        records: Vec<IncomeRecordV2>,
    },
    /// Add a W-2
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    AddWageV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
    },
    /// Add bank interest
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    AddInterestV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
    },
    /// Edit or remove a document
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    ChooseDocumentV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
    },
    /// Income is complete
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    ContinueIncomeV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
    },
    /// Choose an income document
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackV2,ReloadV2")]
    SelectDocumentV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Accepted documents
        /// ----
        /// Document
        #[libertas_copy_from("$.records")]
        #[libertas_read_only]
        records: Vec<IncomeRecordV2>,
        /// Document
        #[libertas_enum_source("^.records")]
        selection: u32,
        /// Remove this document instead of editing
        #[libertas_default(false)]
        remove: bool,
    },
    /// Continue your 2026 interview
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_workflow_request("SaveWageV2")]
    BeginWageV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        progress: Vec<u8>,
        /// Document identity
        #[libertas_hidden]
        id: i64,
    },
    /// W-2 document
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackV2,ReloadV2")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    SaveWageV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        #[libertas_calculation_request("PreviewWageV2")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        #[libertas_calculation_request("PreviewWageV2")]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Document identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.id")]
        #[libertas_calculation_request("PreviewWageV2")]
        id: i64,
        /// Document owner
        owner: OwnerV2,
        /// Fictional employer or bank
        #[libertas_size(min = 1, max = 80)]
        payer: String,
        /// W-2 box 1 wages
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        #[libertas_calculation_request("PreviewWageV2")]
        amount: i64,
        /// Federal income tax withheld
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        #[libertas_calculation_request("PreviewWageV2")]
        withholding: i64,
        /// W-2 box 5 Medicare wages
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        medicare_wages: i64,
        /// W-2 box 4 Social Security tax withheld
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        social_security_withheld: i64,
        /// Income total including this edit
        #[libertas_money("USD", 2)]
        #[libertas_read_only]
        preview_income: Option<i64>,
        /// Withholding total including this edit
        #[libertas_money("USD", 2)]
        #[libertas_read_only]
        preview_withholding: Option<i64>,
    },
    /// Preview income totals
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_exclude_ui]
    #[libertas_next_response("IncomePreviewV2,ProblemV2")]
    PreviewWageV2 {
        /// Cookie
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Revision
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Id
        #[libertas_copy_from("$.id")]
        id: i64,
        /// Amount
        #[libertas_copy_from("$.amount")]
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        amount: i64,
        /// Withholding
        #[libertas_copy_from("$.withholding")]
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        withholding: i64,
    },
    /// Continue your 2026 interview
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_workflow_request("SaveInterestV2")]
    BeginInterestV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        progress: Vec<u8>,
        /// Document identity
        #[libertas_hidden]
        id: i64,
    },
    /// Bank interest document
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackV2,ReloadV2")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    SaveInterestV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        #[libertas_calculation_request("PreviewInterestV2")]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        #[libertas_calculation_request("PreviewInterestV2")]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Document identity
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.id")]
        #[libertas_calculation_request("PreviewInterestV2")]
        id: i64,
        /// Document owner
        owner: OwnerV2,
        /// Fictional employer or bank
        #[libertas_size(min = 1, max = 80)]
        payer: String,
        /// Ordinary taxable bank interest
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        #[libertas_calculation_request("PreviewInterestV2")]
        amount: i64,
        /// Federal income tax withheld
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        #[libertas_calculation_request("PreviewInterestV2")]
        withholding: i64,
        /// Income total including this edit
        #[libertas_money("USD", 2)]
        #[libertas_read_only]
        preview_income: Option<i64>,
        /// Withholding total including this edit
        #[libertas_money("USD", 2)]
        #[libertas_read_only]
        preview_withholding: Option<i64>,
    },
    /// Preview income totals
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_exclude_ui]
    #[libertas_next_response("IncomePreviewV2,ProblemV2")]
    PreviewInterestV2 {
        /// Cookie
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        /// Revision
        #[libertas_copy_from("$.revision")]
        revision: i64,
        /// Id
        #[libertas_copy_from("$.id")]
        id: i64,
        /// Amount
        #[libertas_copy_from("$.amount")]
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        amount: i64,
        /// Withholding
        #[libertas_copy_from("$.withholding")]
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        withholding: i64,
    },
    /// Updated income totals
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    IncomePreviewV2 {
        /// Income total
        #[libertas_money("USD", 2)]
        #[libertas_copy_to("$.preview_income")]
        income: Option<i64>,
        /// Withholding total
        #[libertas_money("USD", 2)]
        #[libertas_copy_to("$.preview_withholding")]
        withholding: Option<i64>,
    },
    /// Continue your 2026 interview
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_workflow_request("SaveCharityAmountV2")]
    BeginCharityAmountV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        progress: Vec<u8>,
    },
    /// Qualifying cash donations
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackV2,ReloadV2")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    SaveCharityAmountV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Total qualifying cash donated during 2026
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        amount: i64,
    },
    /// Continue your 2026 interview
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_workflow_request("SavePaymentsV2")]
    BeginPaymentsV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        progress: Vec<u8>,
    },
    /// Federal tax payments
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_prev_request("BackV2,ReloadV2")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    SavePaymentsV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Interview progress
        #[libertas_formatted_text]
        #[libertas_read_only]
        #[libertas_copy_from("$.progress")]
        progress: Vec<u8>,
        /// Payments excluding income-document withholding
        payments: PaymentsV2,
    },
    /// Federal review — 2026 estimate
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_next_request("StartFinishV2,NavigateV2,ChooseDocumentV2,ReloadV2")]
    ReviewV2 {
        /// Return identity
        #[libertas_hidden]
        #[libertas_read_only]
        cookie: String,
        /// Accepted revision
        #[libertas_hidden]
        #[libertas_read_only]
        revision: i64,
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        previous: i32,
        /// Estimate method and limits
        #[libertas_formatted_text]
        notice: Vec<u8>,
        /// Calculation breakdown
        estimate: EstimateV2,
        /// Income source documents
        /// ----
        /// Document
        records: Vec<IncomeRecordV2>,
    },
    /// Finish prototype review
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_access_privilege("Write")]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    #[libertas_prev_request("BackV2,ReloadV2")]
    FinishV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// I reviewed these synthetic answers; this does not file a return
        confirmed: bool,
    },
    /// Prototype review complete
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    FinishedV2 {
        /// Accepted answers are saved; no return has been filed
        #[libertas_formatted_text]
        notice: Vec<u8>,
        /// Accepted estimate
        estimate: EstimateV2,
    },
    /// Edit an interview section
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    NavigateV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
        /// Section to edit
        section: SectionV2,
        /// Tax situation to edit
        /// Optional: select the specific question when editing Tax situation. Ignored for other sections.
        topic: Option<CoverageTopicV2>,
    },
    /// Back — discard this unsubmitted edit
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    BackV2 {
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
        /// Return destination
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.purpose")]
        purpose: EditPurposeV2,
        /// Current interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.page")]
        page: i32,
        /// Previous interview page
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.previous")]
        previous: i32,
    },
    /// Reload accepted answers
    /// Discard unsent edits and resume from the accepted return.
    #[libertas_request]
    #[libertas_next_response(
        "BeginSetupV2,SaveSetupV2,BeginPersonV2,SavePersonV2,BeginQuestionV2,SaveQuestionV2,IncomeOverviewV2,BeginWageV2,SaveWageV2,BeginInterestV2,SaveInterestV2,BeginCharityAmountV2,SaveCharityAmountV2,BeginPaymentsV2,SavePaymentsV2,ReviewV2,FinishedV2,ProblemV2,SelectDocumentV2,BeginSelectV2"
    )]
    ReloadV2,
    /// Please review
    /// Use synthetic data. Only complete submitted pages are saved.
    #[libertas_response]
    #[libertas_error]
    ProblemV2 {
        /// What needs attention
        #[libertas_formatted_text]
        message: Vec<u8>,
    },
    /// Choose a document
    /// Select an accepted record before editing or removing it.
    #[libertas_response]
    #[libertas_workflow_request("SelectDocumentV2")]
    BeginSelectV2 {
        #[libertas_hidden]
        cookie: String,
        #[libertas_hidden]
        revision: i64,
        #[libertas_hidden]
        purpose: EditPurposeV2,
        #[libertas_hidden]
        page: i32,
        #[libertas_hidden]
        previous: i32,
        /// Accepted documents
        /// Choose a document by its employer or bank label.
        /// ----
        /// Income document
        /// One accepted source document.
        records: Vec<IncomeRecordV2>,
    },
    /// Complete your review
    /// Open the final prototype confirmation.
    #[libertas_response]
    #[libertas_workflow_request("FinishV2")]
    BeginFinishV2 {
        #[libertas_hidden]
        cookie: String,
        #[libertas_hidden]
        revision: i64,
        #[libertas_hidden]
        purpose: EditPurposeV2,
        #[libertas_hidden]
        page: i32,
        #[libertas_hidden]
        previous: i32,
    },
    /// Finish review
    /// Open confirmation; no tax return is filed.
    #[libertas_request]
    #[libertas_next_response("BeginFinishV2,ProblemV2")]
    StartFinishV2 {
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.cookie")]
        cookie: String,
        #[libertas_hidden]
        #[libertas_read_only]
        #[libertas_copy_from("$.revision")]
        revision: i64,
    },
}
