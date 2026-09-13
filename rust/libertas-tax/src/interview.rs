use crate::{IncomeOverviewV1, ReturnSetupV1, TaxInterviewProtocol as P};
use alloc::string::String;
use libertas::libertas_formatted_text;

// Step one owns one synthetic return per task, not one return per opaque peer.
// Durable identities, restart/resume and multiple returns belong to the storage milestone.
const COOKIE: &str = "synthetic-return-v1";
#[derive(Default)]
pub(crate) struct Interview {
    revision: i64,
    setup: Option<ReturnSetupV1>,
    income: Option<IncomeOverviewV1>,
}
#[derive(Debug)]
pub(crate) enum Reply {
    Response(P),
    Edit(P),
}
fn problem(resource: &str) -> Reply {
    Reply::Response(P::ProblemV1 {
        message: libertas_formatted_text(resource, &[]),
    })
}
impl Interview {
    fn about(&self) -> Reply {
        let cookie = String::from(COOKIE);
        let revision = self.revision;
        let progress = libertas_formatted_text("ABOUT_PROGRESS", &[]);
        match &self.setup {
            Some(setup) => Reply::Edit(P::SaveAboutV1 {
                cookie,
                revision,
                progress,
                setup: setup.clone(),
            }),
            None => Reply::Response(P::BeginAboutV1 {
                cookie,
                revision,
                progress,
            }),
        }
    }
    fn income(&self) -> Reply {
        let cookie = String::from(COOKIE);
        let revision = self.revision;
        let progress = libertas_formatted_text("INCOME_PROGRESS", &[]);
        match &self.income {
            Some(income) => Reply::Edit(P::SaveIncomeV1 {
                cookie,
                revision,
                progress,
                income: income.clone(),
            }),
            None => Reply::Response(P::BeginIncomeV1 {
                cookie,
                revision,
                progress,
            }),
        }
    }
    fn resume(&self) -> Reply {
        match (&self.setup, &self.income) {
            (Some(setup), Some(income)) => Reply::Response(P::SummaryV1 {
                cookie: String::from(COOKIE),
                status: libertas_formatted_text("SUMMARY", &[]),
                setup: setup.clone(),
                income: income.clone(),
            }),
            (Some(_), None) => self.income(),
            _ => self.about(),
        }
    }
    pub(crate) fn handle(&mut self, request: P) -> Option<Reply> {
        let reply = match request {
            P::OpenInterviewV1 => self.resume(),
            P::BackToAboutV1 { cookie } | P::BackToIncomeV1 { cookie } if cookie != COOKIE => {
                problem("INVALID_COOKIE")
            }
            P::BackToAboutV1 { .. } => {
                if self.setup.is_some() {
                    self.about()
                } else {
                    problem("NO_ACCEPTED_PAGE")
                }
            }
            P::BackToIncomeV1 { .. } => {
                if self.income.is_some() {
                    self.income()
                } else {
                    problem("NO_ACCEPTED_PAGE")
                }
            }
            P::SaveAboutV1 {
                cookie, revision, ..
            }
            | P::SaveIncomeV1 {
                cookie, revision, ..
            } if cookie != COOKIE || revision != self.revision => problem(if cookie != COOKIE {
                "INVALID_COOKIE"
            } else {
                "STALE_EDIT"
            }),
            P::SaveAboutV1 { setup, .. } => {
                let Some(next) = self.revision.checked_add(1) else {
                    return Some(problem("STALE_EDIT"));
                };
                self.setup = Some(setup);
                self.revision = next;
                self.income()
            }
            P::SaveIncomeV1 { income, .. } => {
                if self.setup.is_none() {
                    problem("NO_ACCEPTED_PAGE")
                } else if income.wages < 0 {
                    problem("NEGATIVE_WAGES")
                } else {
                    let Some(next) = self.revision.checked_add(1) else {
                        return Some(problem("STALE_EDIT"));
                    };
                    self.income = Some(income);
                    self.revision = next;
                    self.resume()
                }
            }
            _ => return None,
        };
        Some(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FilingStatusV1, TaxAppData};
    use alloc::vec;

    fn setup_request(revision: i64) -> P {
        P::SaveAboutV1 {
            cookie: COOKIE.into(),
            revision,
            progress: vec![],
            setup: ReturnSetupV1 {
                label: "Example return".into(),
                filing_status: FilingStatusV1::Single,
            },
        }
    }
    fn income_request(revision: i64, wages: i64) -> P {
        P::SaveIncomeV1 {
            cookie: COOKIE.into(),
            revision,
            progress: vec![],
            income: IncomeOverviewV1 {
                wages,
                has_interest: false,
            },
        }
    }
    #[test]
    fn new_page_cookie_never_invents_required_answers() {
        let mut state = Interview::default();
        assert!(matches!(
            state.handle(P::OpenInterviewV1),
            Some(Reply::Response(P::BeginAboutV1 { revision: 0, .. }))
        ));
        assert!(matches!(
            state.handle(setup_request(0)),
            Some(Reply::Response(P::BeginIncomeV1 { revision: 1, .. }))
        ));
        assert!(state.income.is_none());
        assert!(matches!(
            state.handle(P::OpenInterviewV1),
            Some(Reply::Response(P::BeginIncomeV1 { .. }))
        ));
    }
    #[test]
    fn back_and_review_restore_only_accepted_values() {
        let mut state = Interview::default();
        state.handle(setup_request(0));
        assert!(matches!(
            state.handle(P::BackToAboutV1 {
                cookie: COOKIE.into()
            }),
            Some(Reply::Edit(P::SaveAboutV1 { revision: 1, .. }))
        ));
        state.handle(income_request(1, 1234567));
        match state
            .handle(P::BackToIncomeV1 {
                cookie: COOKIE.into(),
            })
            .unwrap()
        {
            Reply::Edit(P::SaveIncomeV1 {
                income, revision, ..
            }) => {
                assert_eq!(income.wages, 1234567);
                assert!(!income.has_interest);
                assert_eq!(revision, 2);
            }
            _ => panic!("expected a complete income editor"),
        }
        assert!(matches!(
            state.handle(setup_request(2)),
            Some(Reply::Edit(P::SaveIncomeV1 { revision: 3, .. }))
        ));
    }
    #[test]
    fn errors_preserve_accepted_answers_and_revision() {
        let mut state = Interview::default();
        assert!(matches!(
            state.handle(income_request(0, 5)),
            Some(Reply::Response(P::ProblemV1 { .. }))
        ));
        state.handle(setup_request(0));
        for request in [
            income_request(0, 10),
            income_request(1, -1),
            P::BackToAboutV1 {
                cookie: "wrong".into(),
            },
        ] {
            assert!(matches!(
                state.handle(request),
                Some(Reply::Response(P::ProblemV1 { .. }))
            ));
            assert_eq!(state.revision, 1);
            assert!(state.income.is_none());
        }
        assert!(matches!(
            state.handle(income_request(1, 0)),
            Some(Reply::Response(P::SummaryV1 { .. }))
        ));
    }
    #[test]
    fn protocol_and_storage_round_trip_exact_cents_and_false() {
        // Above the exact-integer range of f64: there must be no floating-point conversion.
        let request = income_request(1, 9_007_199_254_740_993);
        assert_eq!(P::from_avro(&request.to_avro()).unwrap(), request);
        let data = TaxAppData::DraftV1 {
            tax_year: 2026,
            revision: 1,
            setup: None,
            income: Some(IncomeOverviewV1 {
                wages: i64::MAX,
                has_interest: false,
            }),
        };
        assert_eq!(TaxAppData::from_avro(&data.to_avro()).unwrap(), data);
    }
    #[test]
    fn response_is_not_an_incoming_request() {
        let mut state = Interview::default();
        assert!(state.handle(P::ProblemV1 { message: vec![] }).is_none());
        assert_eq!(state.revision, 0);
    }
}
