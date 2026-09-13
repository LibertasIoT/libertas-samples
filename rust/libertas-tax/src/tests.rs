use crate::interview::{Interview, Reply};
use crate::rules::{TOPICS, estimate, rate_tax};
use crate::*;
use TaxInterviewProtocol as P;
use alloc::vec;

fn person() -> PersonV2 {
    PersonV2 {
        resident: AnswerV2::Yes,
        age: AgeV2::From25To64,
        blind: AnswerV2::No,
        claimable: AnswerV2::No,
    }
}
fn record(id: i64, kind: IncomeKindV2, amount: i64, withholding: i64) -> IncomeRecordV2 {
    IncomeRecordV2 {
        id,
        kind,
        owner: OwnerV2::Taxpayer,
        payer: "Example payer".into(),
        amount,
        withholding,
        medicare_wages: if kind == IncomeKindV2::Wage {
            amount
        } else {
            0
        },
        social_security_withheld: 0,
    }
}
fn complete(joint: bool) -> DraftV2 {
    let mut d = DraftV2::empty("test-return".into());
    d.setup = Some(SetupV2 {
        label: "Synthetic return".into(),
        filing_status: if joint {
            FilingStatusV2::Joint
        } else {
            FilingStatusV2::Single
        },
        basis: AmountBasisV2::Estimate,
    });
    d.taxpayer = Some(person());
    if joint {
        d.spouse = Some(person());
    }
    d.coverage = TOPICS
        .iter()
        .map(|&topic| CoverageAnswerV2 {
            topic,
            answer: AnswerV2::No,
        })
        .collect();
    d.records = if joint {
        vec![record(1, IncomeKindV2::Wage, 10_000_000, 1_000_000)]
    } else {
        vec![
            record(1, IncomeKindV2::Wage, 6_000_000, 650_000),
            record(2, IncomeKindV2::Interest, 20_000, 0),
        ]
    };
    d.next_id = 3;
    d.income_complete = true;
    d.charity = Some(AnswerV2::No);
    d.payments = Some(PaymentsV2 {
        estimated: 0,
        extension: 0,
    });
    assert!(d.valid_stored());
    d
}
fn message(reply: Reply) -> P {
    match reply {
        Reply::Response(p) | Reply::Edit(p) => p,
    }
}
fn send(s: &mut Interview, p: P) -> P {
    let encoded = p.to_avro();
    assert_eq!(P::from_avro(&encoded).unwrap(), p);
    let reply = message(s.handle(p).expect("valid request"));
    assert_eq!(P::from_avro(&reply.to_avro()).unwrap(), reply);
    reply
}
fn assert_error(p: P) {
    assert!(matches!(p, P::ProblemV2 { .. }), "{p:?}");
}
fn restart(d: &DraftV2) -> Interview {
    let saved = TaxAppData::DraftV2 { draft: d.clone() };
    let decoded = TaxAppData::from_avro(&saved.to_avro()).unwrap();
    assert_eq!(decoded, saved);
    match decoded {
        TaxAppData::DraftV2 { draft } => Interview::new(draft),
        _ => panic!("wrong storage variant"),
    }
}
fn back(s: &mut Interview, previous: i32) -> P {
    send(
        s,
        P::BackV2 {
            cookie: s.draft.cookie.clone(),
            revision: 0,
            purpose: EditPurposeV2::Review,
            page: 20,
            previous,
        },
    )
}

#[test]
fn independent_single_joint_and_charity_fixtures() {
    let d = complete(false);
    let e = estimate(&d);
    assert_eq!(e.state, ResultStateV2::EstimatedForSupportedScope);
    assert_eq!(
        (
            e.agi,
            e.standard_deduction,
            e.taxable_income,
            e.income_tax,
            e.refund
        ),
        (
            Some(6_020_000),
            Some(1_610_000),
            Some(4_410_000),
            Some(504_400),
            Some(145_600)
        )
    );
    let e = estimate(&complete(true));
    assert_eq!(
        (e.taxable_income, e.income_tax, e.refund),
        (Some(6_780_000), Some(764_000), Some(236_000))
    );
    let mut d = d;
    d.charity = Some(AnswerV2::Yes);
    d.charity_qualified = Some(AnswerV2::Yes);
    d.charity_amount = Some(100_000);
    let e = estimate(&d);
    assert_eq!(
        (
            e.charity_deduction,
            e.taxable_income,
            e.income_tax,
            e.refund
        ),
        (Some(100_000), Some(4_310_000), Some(492_400), Some(157_600))
    );
    d.charity_amount = Some(500_000);
    assert_eq!(estimate(&d).charity_deduction, Some(100_000));
    let mut d = complete(true);
    d.charity = Some(AnswerV2::Yes);
    d.charity_qualified = Some(AnswerV2::Yes);
    d.charity_amount = Some(900_000);
    assert_eq!(estimate(&d).charity_deduction, Some(200_000));
}
#[test]
fn independent_rate_schedule_edges() {
    for (income, tax) in [
        (0, 0),
        (12_400, 124_000),
        (50_400, 580_000),
        (105_700, 1_796_600),
        (201_775, 4_102_400),
        (256_225, 5_844_800),
        (640_600, 19_297_900),
    ] {
        assert_eq!(rate_tax(income, false), tax, "single {income}");
    }
    for (income, tax) in [
        (24_800, 248_000),
        (100_800, 1_160_000),
        (211_400, 3_593_200),
        (403_550, 8_204_800),
        (512_450, 11_689_600),
        (768_700, 20_658_400),
    ] {
        assert_eq!(rate_tax(income, true), tax, "joint {income}");
    }
    assert_eq!(rate_tax(12_404, false), 124_000);
    assert_eq!(rate_tax(12_405, false), 124_100);
}
#[test]
fn exact_cents_aggregate_before_line_rounding() {
    let mut d = complete(false);
    d.records = vec![
        record(1, IncomeKindV2::Wage, 3_000_025, 25),
        record(2, IncomeKindV2::Wage, 3_000_025, 25),
        record(3, IncomeKindV2::Interest, 20_049, 49),
    ];
    d.next_id = 4;
    d.payments = Some(PaymentsV2 {
        estimated: 50,
        extension: 49,
    });
    let e = estimate(&d);
    assert_eq!(e.agi, Some(6_020_100));
    assert_eq!(e.payments, 200);
    assert_eq!(e.wages, 6_000_050);
    assert_eq!(e.interest, 20_049);
}
#[test]
fn unknown_and_positive_coverage_never_produce_complete_estimate() {
    for n in 0..TOPICS.len() {
        for answer in [AnswerV2::Yes, AnswerV2::NotSure] {
            let mut d = complete(false);
            d.coverage[n].answer = answer;
            let e = estimate(&d);
            assert_eq!(e.state, ResultStateV2::Unsupported);
            assert!(e.income_tax.is_none());
            assert!(e.refund.is_none());
        }
    }
    let mut d = complete(false);
    d.coverage.pop();
    let e = estimate(&d);
    assert_eq!(e.state, ResultStateV2::Incomplete);
    assert!(e.taxable_income.is_none());
    d = complete(false);
    d.charity = Some(AnswerV2::NotSure);
    assert_eq!(estimate(&d).state, ResultStateV2::Unsupported);
}
#[test]
fn automatic_unimplemented_tax_screens_override_generic_no() {
    let mut cases = vec![];
    let mut d = complete(false);
    d.records[0].amount = 1_500_000;
    d.records[0].medicare_wages = 1_500_000;
    cases.push(d); // EITC
    let mut d = complete(false);
    d.records[0].medicare_wages = 20_000_001;
    cases.push(d);
    let mut d = complete(false);
    d.records[0].amount = 21_000_000;
    cases.push(d); // NIIT even box5 below threshold
    let mut d = complete(false);
    d.records[0].social_security_withheld = 1_143_901;
    cases.push(d);
    let mut d = complete(false);
    d.taxpayer.as_mut().unwrap().age = AgeV2::Under25;
    cases.push(d);
    let mut d = complete(false);
    d.taxpayer.as_mut().unwrap().blind = AnswerV2::Yes;
    cases.push(d);
    let mut d = complete(false);
    d.setup.as_mut().unwrap().filing_status = FilingStatusV2::Other;
    cases.push(d);
    let mut d = complete(false);
    d.records[1].owner = OwnerV2::Spouse;
    cases.push(d);
    for d in cases {
        let e = estimate(&d);
        assert_eq!(e.state, ResultStateV2::Unsupported, "{e:?}");
        assert!(e.income_tax.is_none());
        assert!(!e.issues.is_empty());
    }
}
#[test]
fn social_security_and_medicare_screen_per_person_and_return() {
    let mut d = complete(true);
    d.records = vec![
        record(1, IncomeKindV2::Wage, 12_000_000, 1_000_000),
        record(2, IncomeKindV2::Wage, 12_000_000, 1_000_000),
    ];
    d.records[1].owner = OwnerV2::Spouse;
    for r in &mut d.records {
        r.social_security_withheld = 744_000;
    }
    assert_eq!(
        estimate(&d).state,
        ResultStateV2::EstimatedForSupportedScope
    );
    d.records[1].owner = OwnerV2::Taxpayer;
    assert_eq!(estimate(&d).state, ResultStateV2::Unsupported);
    d.records[1].owner = OwnerV2::Spouse;
    d.records[1].medicare_wages = 13_000_001;
    assert_eq!(estimate(&d).state, ResultStateV2::Unsupported);
}
#[test]
fn preview_replaces_record_never_saves_and_rejects_stale_inputs() {
    let mut s = Interview::new(complete(false));
    let original = s.draft.clone();
    let p = P::PreviewWageV2 {
        cookie: s.draft.cookie.clone(),
        revision: 0,
        id: 1,
        amount: 7_000_000,
        withholding: 700_000,
    };
    assert_eq!(
        send(&mut s, p.clone()),
        P::IncomePreviewV2 {
            income: Some(7_020_000),
            withholding: Some(700_000)
        }
    );
    assert_eq!(s.draft, original);
    assert!(!s.dirty);
    let editor = back(&mut s, 19);
    let P::SavePaymentsV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        progress,
        ..
    } = editor
    else {
        panic!("expected complete payments editor")
    };
    send(
        &mut s,
        P::SavePaymentsV2 {
            cookie,
            revision,
            purpose,
            page,
            previous,
            progress,
            payments: PaymentsV2 {
                estimated: 100,
                extension: 0,
            },
        },
    );
    assert_error(send(&mut s, p));
    assert_eq!(s.draft.revision, 1);
}
#[test]
fn review_document_edit_ignores_client_preview_and_saves_one_revision() {
    let mut s = Interview::new(complete(false));
    let p = P::ChooseDocumentV2 {
        cookie: s.draft.cookie.clone(),
        revision: 0,
        purpose: EditPurposeV2::Review,
        page: 20,
        previous: 19,
    };
    let P::BeginSelectV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        records,
    } = send(&mut s, p)
    else {
        panic!("expected empty selection workflow")
    };
    let p = P::SelectDocumentV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        records,
        selection: 0,
        remove: false,
    };
    let P::SaveWageV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        progress,
        id,
        owner,
        payer,
        medicare_wages,
        social_security_withheld,
        ..
    } = send(&mut s, p)
    else {
        panic!("expected accepted document editor")
    };
    let p = P::SaveWageV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        progress,
        id,
        owner,
        payer,
        amount: 7_000_000,
        withholding: 650_000,
        medicare_wages,
        social_security_withheld,
        preview_income: Some(-1),
        preview_withholding: None,
    };
    assert!(matches!(send(&mut s, p.clone()), P::ReviewV2 { .. }));
    assert_eq!(s.draft.records.len(), 2);
    assert_eq!(s.draft.records[0].amount, 7_000_000);
    assert_eq!(s.draft.revision, 1);
    assert!(s.dirty);
    assert_error(send(&mut s, p));
    assert_eq!(s.draft.revision, 1);
    let mut restored = restart(&s.draft);
    assert!(matches!(
        send(&mut restored, P::OpenInterviewV1),
        P::ReviewV2 { .. }
    ));
}
#[test]
fn document_delete_and_stale_selection_preserve_identity() {
    let mut s = Interview::new(complete(false));
    let p = P::SelectDocumentV2 {
        cookie: s.draft.cookie.clone(),
        revision: 0,
        purpose: EditPurposeV2::Review,
        page: 15,
        previous: 20,
        records: s.draft.records.clone(),
        selection: 0,
        remove: true,
    };
    send(&mut s, p.clone());
    assert_eq!(s.draft.records[0].id, 2);
    assert_eq!(s.draft.next_id, 3);
    assert_error(send(&mut s, p));
    assert_eq!(s.draft.records.len(), 1);
    let bad = P::SelectDocumentV2 {
        cookie: s.draft.cookie.clone(),
        revision: 1,
        purpose: EditPurposeV2::Review,
        page: 15,
        previous: 20,
        records: vec![],
        selection: 0,
        remove: true,
    };
    assert_error(send(&mut s, bad));
    assert_eq!(s.draft.revision, 1);
}
#[test]
fn finish_is_a_workflow_confirmation_and_restart_returns_review() {
    let mut s = Interview::new(complete(false));
    let p = P::StartFinishV2 {
        cookie: s.draft.cookie.clone(),
        revision: 0,
    };
    let P::BeginFinishV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
    } = send(&mut s, p)
    else {
        panic!("expected workflow entry")
    };
    let p = P::FinishV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        confirmed: true,
    };
    assert!(matches!(send(&mut s, p), P::FinishedV2 { .. }));
    assert!(s.draft.finished);
    let mut restored = restart(&s.draft);
    assert!(matches!(
        send(&mut restored, P::OpenInterviewV1),
        P::ReviewV2 { .. }
    ));
}
#[test]
fn invalid_readable_storage_is_preserved_and_blocked() {
    let mut d = complete(false);
    d.records[1].id = 1;
    let mut s = restart(&d);
    assert_error(send(&mut s, P::OpenInterviewV1));
    assert_eq!(s.draft, d);
    assert!(!s.dirty);
    let mut d = complete(false);
    d.tax_year = 2025;
    assert!(!d.valid_stored());
    let mut d = complete(false);
    d.records[0].amount = -1;
    assert!(!d.valid_stored());
    let mut d = complete(false);
    d.coverage.push(d.coverage[0].clone());
    assert!(!d.valid_stored());
    let mut s = Interview::blocked();
    assert_error(send(&mut s, P::OpenInterviewV1));
    assert!(!s.dirty);
}
#[test]
fn partial_answers_resume_without_inventing_required_values() {
    let mut s = Interview::new(DraftV2::empty("test-return".into()));
    let P::BeginSetupV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        progress,
    } = send(&mut s, P::OpenInterviewV1)
    else {
        panic!("new setup")
    };
    let p = P::SaveSetupV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        progress,
        setup: complete(false).setup.unwrap(),
        confirm_basis_change: false,
    };
    assert!(matches!(send(&mut s, p), P::BeginPersonV2 { .. }));
    assert!(s.draft.taxpayer.is_none());
    let mut restored = restart(&s.draft);
    assert!(matches!(
        send(&mut restored, P::OpenInterviewV1),
        P::BeginPersonV2 { .. }
    ));
    assert!(matches!(
        back(&mut restored, 0),
        P::SaveSetupV2 { revision: 1, .. }
    ));
}
#[test]
fn changing_charity_applicability_invalidates_accepted_dependent_answers() {
    let mut d = complete(false);
    d.charity = Some(AnswerV2::Yes);
    d.charity_qualified = Some(AnswerV2::Yes);
    d.charity_amount = Some(100_000);
    let mut s = Interview::new(d);
    let P::SaveQuestionV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        progress,
        question,
        topic,
        ..
    } = back(&mut s, 16)
    else {
        panic!("charity editor")
    };
    let p = P::SaveQuestionV2 {
        cookie,
        revision,
        purpose,
        page,
        previous,
        progress,
        question,
        topic,
        answer: AnswerV2::No,
    };
    assert!(matches!(send(&mut s, p), P::ReviewV2 { .. }));
    assert!(s.draft.charity_qualified.is_none());
    assert!(s.draft.charity_amount.is_none());
}
#[test]
fn every_coverage_question_can_be_reopened_from_review() {
    let mut s = Interview::new(complete(false));
    for (n, topic) in TOPICS.into_iter().enumerate() {
        let p = P::NavigateV2 {
            cookie: s.draft.cookie.clone(),
            revision: 0,
            purpose: EditPurposeV2::Review,
            page: 20,
            previous: 19,
            section: SectionV2::Coverage,
            topic: Some(topic),
        };
        assert!(matches!(send(&mut s,p),P::SaveQuestionV2 {page,..} if page==n as i32+3));
    }
    assert!(!s.dirty);
}
#[test]
fn response_cannot_be_submitted_as_request() {
    let mut s = Interview::new(complete(false));
    assert!(
        s.handle(P::IncomePreviewV2 {
            income: None,
            withholding: None
        })
        .is_none()
    );
    assert!(!s.dirty);
}

#[test]
fn complete_new_interview_single_and_joint_with_restart_after_each_page() {
    for joint in [false, true] {
        let mut s = Interview::new(DraftV2::empty("test-return".into()));
        let mut response = send(&mut s, P::OpenInterviewV1);
        let mut documents = 0;
        for _ in 0..35 {
            let request = match response {
                P::BeginSetupV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                } => P::SaveSetupV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    setup: complete(joint).setup.unwrap(),
                    confirm_basis_change: false,
                },
                P::BeginPersonV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                } => P::SavePersonV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    person: person(),
                },
                P::BeginQuestionV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    question,
                    topic,
                } => P::SaveQuestionV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    question,
                    topic,
                    answer: if page == 16 || page == 17 {
                        AnswerV2::Yes
                    } else {
                        AnswerV2::No
                    },
                },
                P::IncomeOverviewV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    ..
                } => {
                    if documents == 0 {
                        P::AddWageV2 {
                            cookie,
                            revision,
                            purpose,
                            page,
                            previous,
                        }
                    } else if documents == 1 {
                        P::AddInterestV2 {
                            cookie,
                            revision,
                            purpose,
                            page,
                            previous,
                        }
                    } else {
                        P::ContinueIncomeV2 {
                            cookie,
                            revision,
                            purpose,
                            page,
                            previous,
                        }
                    }
                }
                P::BeginWageV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    id,
                } => {
                    assert_eq!(s.draft.next_id, id);
                    assert_eq!(s.draft.records.len(), 0);
                    documents += 1;
                    P::SaveWageV2 {
                        cookie,
                        revision,
                        purpose,
                        page,
                        previous,
                        progress,
                        id,
                        owner: OwnerV2::Taxpayer,
                        payer: "Synthetic employer".into(),
                        amount: 6_000_000,
                        withholding: 650_000,
                        medicare_wages: 6_000_000,
                        social_security_withheld: 372_000,
                        preview_income: None,
                        preview_withholding: None,
                    }
                }
                P::BeginInterestV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    id,
                } => {
                    documents += 1;
                    P::SaveInterestV2 {
                        cookie,
                        revision,
                        purpose,
                        page,
                        previous,
                        progress,
                        id,
                        owner: OwnerV2::Taxpayer,
                        payer: "Synthetic bank".into(),
                        amount: 20_000,
                        withholding: 0,
                        preview_income: None,
                        preview_withholding: None,
                    }
                }
                P::BeginCharityAmountV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                } => P::SaveCharityAmountV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    amount: 100_000,
                },
                P::BeginPaymentsV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                } => P::SavePaymentsV2 {
                    cookie,
                    revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    payments: PaymentsV2 {
                        estimated: 0,
                        extension: 0,
                    },
                },
                P::ReviewV2 {
                    estimate, notice, ..
                } => {
                    assert_eq!(
                        estimate.state,
                        ResultStateV2::EstimatedForSupportedScope,
                        "{estimate:?}"
                    );
                    let count = if joint { "20 of 20" } else { "19 of 19" };
                    assert!(notice.windows(count.len()).any(|v| v == count.as_bytes()));
                    break;
                }
                other => panic!("unexpected page: {other:?}"),
            };
            let old_revision = s.draft.revision;
            response = send(&mut s, request);
            assert!(s.draft.revision == old_revision || s.draft.revision == old_revision + 1);
            // Navigation is per interaction; the next response survives while the persisted App restarts.
            s = restart(&s.draft);
        }
        assert_eq!(s.draft.first_incomplete(), 20);
        assert_eq!(s.draft.records.len(), 2);
        assert_eq!(s.draft.next_id, 3);
    }
}
#[test]
fn basis_change_requires_explicit_confirmation_and_revalidation() {
    let mut s = Interview::new(complete(false));
    let mut p = back(&mut s, 0);
    if let P::SaveSetupV2 { setup, .. } = &mut p {
        setup.basis = AmountBasisV2::Actual;
    } else {
        panic!("setup editor")
    };
    assert_error(send(&mut s, p.clone()));
    assert_eq!(s.draft.revision, 0);
    if let P::SaveSetupV2 {
        confirm_basis_change,
        ..
    } = &mut p
    {
        *confirm_basis_change = true;
    }
    send(&mut s, p);
    assert_eq!(s.draft.records.len(), 2);
    assert!(!s.draft.income_complete);
    assert!(s.draft.payments.is_none());
    assert_eq!(estimate(&s.draft).state, ResultStateV2::Incomplete);
}
#[test]
fn bounded_maximum_arithmetic_and_zero_are_safe() {
    let mut d = complete(false);
    d.records = (1..=100)
        .map(|id| record(id, IncomeKindV2::Wage, 100_000_000_000, 100_000_000_000))
        .collect();
    d.next_id = 101;
    assert!(d.valid_stored());
    let e = estimate(&d);
    assert_eq!(e.wages, 10_000_000_000_000);
    assert_eq!(e.state, ResultStateV2::Unsupported);
    d.records.clear();
    let e = estimate(&d);
    assert_eq!(e.state, ResultStateV2::EstimatedForSupportedScope);
    assert_eq!(e.income_tax, Some(0));
    assert_eq!(e.refund, Some(0));
}

#[test]
fn trace_uses_stable_document_ids_and_calculation_dependencies() {
    let e = estimate(&complete(false));
    assert_eq!(e.trace.len(), 10);
    assert_eq!(e.trace[0].inputs, vec!["document:1.amount"]);
    assert_eq!(e.trace[1].inputs, vec!["document:2.amount"]);
    assert_eq!(
        e.trace
            .iter()
            .find(|l| l.id == "income_tax")
            .unwrap()
            .amount,
        504_400
    );
    for (n, line) in e.trace.iter().enumerate() {
        assert!(line.rule.starts_with("US-2026-r1."));
        assert!(line.source.contains("https://www.irs.gov/"));
        assert!(e.trace[..n].iter().all(|prior| prior.id != line.id));
    }
}

#[test]
fn original_protocol_and_storage_discriminants_remain_stable() {
    assert_eq!(P::OpenInterviewV1.to_avro(), vec![0]);
    let old = TaxAppData::DraftV1 {
        tax_year: 2026,
        revision: 1,
        setup: None,
        income: None,
    };
    assert_eq!(old.to_avro(), vec![0, 0xd4, 0x1f, 2, 0, 0]);
    assert_eq!(TaxAppData::from_avro(&old.to_avro()).unwrap(), old);
}
