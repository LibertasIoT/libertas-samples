//! Libertas tax interview prototype
//! #[libertas_string_resources(APP_STRINGS)]
//! Interview, saved accepted answers, and a U.S. federal tax year 2026 estimate.
//! Use synthetic data only. This prototype does not file a return.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod federal_model;
mod federal_rules;
mod ira;
pub use federal_model::{
    CareClaim, CharityClaim, DeductionChoice, DeductionSelection, DependentCare,
    DependentResidency, EducationChoice, EducationMethod, EducatorClaim, FederalAdjustments,
    FederalAmount, FederalBusiness, FederalCredits, FederalDeductions, FederalDependent,
    FederalDividend, FederalDraft, FederalEducation, FederalHsa, FederalIncome, FederalIncomeEntry,
    FederalInterest, FederalIraContribution, FederalIras, FederalIssue, FederalPayments,
    FederalPeople, FederalPerson, FederalResult, FederalRetirement, FederalSale, FederalSaver,
    FederalSaverDistribution, FederalScreening, FederalSetup, FederalSocial, FederalStatus,
    FederalStudent, FederalUnemployment, FederalVehicleInterest, FederalWage,
    FederalWorkDeductions, FilingChoice, GainTerm, HsaCoverage, IraChoice, IraSpouse,
    ItemizedChoice, ItemizedExpenses, MaritalState, MortgageClaim, OvertimeClaim, Relationship,
    SaverChoice, SaverDistributionPeriod, SaverDistributionYear, SpouseAmount, SpouseFiler,
    SpouseLiving, StudentLoanClaim, SupportShare, TipsClaim, WorkDeductionChoice,
};
mod common;
pub use common::{AmountBasis, Answer, CalculationLine, Owner, ResultState};
mod interview;
mod model;
mod protocol;
#[cfg(test)]
mod tests;
pub use model::TaxAppData;
pub use protocol::{IncomeAmountInput, IncomeEditorItem, TaxInterviewProtocol, TaxSection};

use alloc::boxed::Box;
use core::any::Any;
use interview::{Interview, Reply};
use libertas::{
    LibertasEndpoint, LibertasEndpointStatus, OP_ENDPOINT_REQ, libertas_endpoint_edit_request,
    libertas_endpoint_response, libertas_register_endpoint_listener,
};
use libertas_macros::{libertas_data_schema, libertas_export};

pub const APP_STRINGS: &[(&str, &str)] = &[
    ("RETURN_DATA", "Saved 2026 federal prototype return"),
    ("TEXT", "{0}"),
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
    // One accepted revision is one data record. Preview and navigation never write.
    // The SDK write contract is assumed successful; persist before sending the correlated reply.
    if state.dirty {
        libertas::libertas_data_write_single(
            "RETURN_DATA",
            &[],
            &TaxAppData::Draft {
                draft: state.draft.clone(),
            },
        );
        state.dirty = false;
    }
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
/// Create one saved synthetic return per task. Resume accepted pages after reopening; larger forms can restore completed field edits saved on this device. Unfinished typing is discarded. No return is filed.
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
    let state = match libertas::libertas_data_read_single::<TaxAppData>("RETURN_DATA", &[]) {
        Some(TaxAppData::Draft { draft }) => Interview::new(draft),
        None => {
            let cookie = alloc::format!(
                "{:016x}{:016x}",
                libertas::libertas_get_random(8),
                libertas::libertas_get_random(8)
            );
            let draft = FederalDraft::empty(cookie);
            libertas::libertas_data_write_single(
                "RETURN_DATA",
                &[],
                &TaxAppData::Draft {
                    draft: draft.clone(),
                },
            );
            Interview::new(draft)
        }
    };
    libertas_register_endpoint_listener::<TaxInterviewProtocol, _>(
        interview,
        handle_request,
        Box::new(state),
    );
}
