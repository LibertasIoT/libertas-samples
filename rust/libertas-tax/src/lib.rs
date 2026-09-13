//! Libertas tax interview prototype
//! #[libertas_string_resources(APP_STRINGS)]
//! Explore a two-page U.S. federal tax year 2026 interview with synthetic answers.
//! Answers are temporary. This prototype does not calculate tax or file a return.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod interview;
mod model;
mod protocol;
pub use model::{FilingStatusV1, IncomeOverviewV1, ReturnSetupV1, TaxAppData};
pub use protocol::TaxInterviewProtocol;

use alloc::boxed::Box;
use core::any::Any;
use interview::{Interview, Reply};
use libertas::{
    LibertasEndpoint, LibertasEndpointStatus, OP_ENDPOINT_REQ, libertas_endpoint_edit_request,
    libertas_endpoint_response, libertas_register_endpoint_listener,
};
use libertas_macros::{libertas_data_schema, libertas_export};

pub const APP_STRINGS: &[(&str, &str)] = &[
    (
        "ABOUT_PROGRESS",
        "Step 1 of 2 · About you · Tax year 2026 · Synthetic prototype",
    ),
    (
        "INCOME_PROGRESS",
        "Step 2 of 2 · Income · Tax year 2026 · Synthetic prototype",
    ),
    (
        "SUMMARY",
        "Both pages submitted. These temporary answers are not a tax estimate and are not saved across task restarts.",
    ),
    (
        "INVALID_COOKIE",
        "This interview is unavailable. Reopen the endpoint to continue.",
    ),
    (
        "STALE_EDIT",
        "The accepted answers changed. Reopen the interview before submitting again.",
    ),
    (
        "NO_ACCEPTED_PAGE",
        "This page has not been submitted yet. Reopen the interview to continue.",
    ),
    (
        "NEGATIVE_WAGES",
        "Total wages cannot be negative. Enter a synthetic amount, or enter zero if there are no wages.",
    ),
];

fn handle_request(
    endpoint: LibertasEndpoint,
    opcode: u8,
    message: Option<TaxInterviewProtocol>,
    context: &mut Box<dyn Any>,
    transaction_id: u32,
    peer: u32,
) -> LibertasEndpointStatus {
    if opcode != OP_ENDPOINT_REQ {
        return LibertasEndpointStatus::InvalidMessage;
    }
    let Some(state) = context.downcast_mut::<Interview>() else {
        return LibertasEndpointStatus::InvalidMessage;
    };
    let Some(reply) = message.and_then(|request| state.handle(request)) else {
        return LibertasEndpointStatus::InvalidMessage;
    };
    // Finish the state transition before calling the host; replies retain the original correlation.
    match reply {
        Reply::Response(value) => {
            libertas_endpoint_response(endpoint, &value, transaction_id, peer)
        }
        Reply::Edit(value) => {
            libertas_endpoint_edit_request(endpoint, &value, transaction_id, peer)
        }
    }
    LibertasEndpointStatus::Success
}

/// Tax interview prototype
/// Create a temporary two-page interview. Use synthetic answers only; restarting this task discards them.
/// [DefaultTaskName]
/// Tax interview prototype
#[libertas_export]
#[libertas_data_schema(TaxAppData)]
pub fn tax_interview(
    /*
     * Tax interview
     * Open the synthetic U.S. federal tax year 2026 interview.
     * [DefaultText]
     * Tax interview prototype
     */
    #[libertas_endpoint_schema(TaxInterviewProtocol)]
    #[libertas_endpoint_server]
    interview: LibertasEndpoint,
) {
    libertas_register_endpoint_listener::<TaxInterviewProtocol, _>(
        interview,
        handle_request,
        Box::new(Interview::default()),
    );
}
