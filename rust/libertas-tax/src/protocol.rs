use crate::{IncomeOverviewV1, ReturnSetupV1};
use alloc::{string::String, vec::Vec};
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Tax interview
/// A two-page demonstration using synthetic answers. Answers are lost when the task restarts.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub enum TaxInterviewProtocol {
    /// Open interview
    /// Start or resume this task's temporary synthetic return.
    #[libertas_request]
    #[libertas_next_response("BeginAboutV1,BeginIncomeV1,SummaryV1")]
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
}
