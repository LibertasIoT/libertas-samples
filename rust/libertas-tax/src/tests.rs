use crate::federal_rules::{
    care_rate, eic_amount, estimate, preferential_tax, social_taxable, valid_birth,
};
use crate::interview::{Interview, Reply};
use crate::*;
use TaxInterviewProtocol as P;
use alloc::vec;
fn person() -> FederalPerson {
    FederalPerson {
        name: "Synthetic Filer".into(),
        birth_date: 19800615,
        blind: false,
        full_year_resident: Answer::Yes,
        claimable: Answer::No,
        alive_at_year_end: true,
        another_eitc_child: Answer::No,
        valid_ssn: Answer::Yes,
        valid_tin: Answer::Yes,
        us_home_over_half_year: Answer::Yes,
        full_time_student: false,
        disabled: false,
        incapable_self_care: false,
        support_share: SupportShare::MoreThanHalf,
        parent_alive: true,
    }
}
fn wage(id: i64, owner: Owner, amount: i64, withholding: i64) -> FederalIncomeEntry {
    FederalIncomeEntry {
        id,
        label: alloc::format!("Employer {id}"),
        owner,
        income: FederalIncome::Wage {
            data: FederalWage {
                wages: amount,
                withholding,
                social_wages: amount.min(18_450_000),
                social_tips: 0,
                work_deductions: (None).into(),
                social_tax: amount.min(18_450_000) * 62 / 1000,
                medicare_wages: amount,
                medicare_tax: amount * 145 / 10000 + (amount - 20_000_000).max(0) * 9 / 1000,
                special_boxes: Answer::No,
            },
        },
    }
}
fn filing_choice(status: FederalStatus) -> FilingChoice {
    match status {
        FederalStatus::Single => FilingChoice::Single,
        FederalStatus::Joint => FilingChoice::Joint,
        FederalStatus::Separate => FilingChoice::Separate {
            spouse_itemizes: false,
        },
        FederalStatus::Head => FilingChoice::Head,
        FederalStatus::Surviving => FilingChoice::Surviving,
    }
}
fn complete(status: FederalStatus) -> FederalDraft {
    let mut d = FederalDraft::empty("synthetic-return".into());
    let joint = status == FederalStatus::Joint;
    d.setup = Some(FederalSetup {
        label: "Synthetic return".into(),
        basis: AmountBasis::Estimate,
        status: filing_choice(status),
        marital_state: if joint || status == FederalStatus::Separate {
            MaritalState::Married {
                living: SpouseLiving::Together,
            }
        } else {
            MaritalState::Unmarried
        },
        paid_over_half_home: true,
        community_property: Answer::No,
    });
    d.people = Some(FederalPeople {
        taxpayer: person(),
        spouse: (if joint { Some(person()) } else { None }).into(),
    });
    d.dependents_complete = true;
    d.income_complete = true;
    d.income = vec![wage(
        1,
        Owner::Taxpayer,
        if joint { 10_000_000 } else { 6_000_000 },
        650_000,
    )];
    d.next_id = 2;
    d.adjustments = Some(FederalAdjustments {
        hsa: vec![],
        iras: (None).into(),
        other_adjustments: Answer::No,
        student_loan: StudentLoanClaim::No,
        educators: EducatorClaim::No,
    });
    d.deductions = Some(FederalDeductions {
        choice: DeductionSelection::Automatic {
            expenses: ItemizedChoice::No,
        },
        special_itemizing: Answer::No,
        vehicle_loans: vec![],
        special_work_deductions: Answer::No,
        charity: CharityClaim::No,
    });
    d.credits = Some(FederalCredits {
        students: crate::federal_rules::students(&d),
        education: vec![],
        saver: (None).into(),
        special_credits: Answer::No,
        care: CareClaim::No,
    });
    d.screening = Some(FederalScreening {
        foreign: Answer::No,
        property: Answer::No,
        carryovers: Answer::No,
        special_taxes: Answer::No,
        uncertain: Answer::No,
    });
    d.payments = Some(FederalPayments {
        estimated: 0,
        extension: 0,
    });
    d
}
fn child(id: i64) -> FederalDependent {
    FederalDependent {
        id,
        label: alloc::format!("Child {id}"),
        birth_date: 20180615,
        relationship: Relationship::Child,
        residency: DependentResidency::Days { days: 365 },
        us_home_over_half_year: Answer::Yes,
        student: false,
        disabled: false,
        incapable_self_care: false,
        own_support_over_half: false,
        your_support_over_half: true,
        gross_income: 0,
        us_citizen_resident: Answer::Yes,
        valid_ssn: Answer::Yes,
        valid_tin: Answer::Yes,
        joint_return_nonrefund: false,
        competing_claim: Answer::No,
        care: DependentCare::No,
    }
}
fn add(d: &mut FederalDraft, mut e: FederalIncomeEntry) {
    e.id = d.next_id;
    d.next_id += 1;
    d.income.push(e);
}
fn add_child(d: &mut FederalDraft) {
    let c = child(d.next_id);
    d.next_id += 1;
    d.dependents.push(c);
}
fn income(value: FederalIncome) -> FederalIncomeEntry {
    FederalIncomeEntry {
        id: 0,
        label: "Synthetic document".into(),
        owner: Owner::Taxpayer,
        income: value,
    }
}
fn value(r: &FederalResult, id: &str) -> i64 {
    r.lines
        .iter()
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("missing {id}: {r:?}"))
        .amount
        / 100
}
fn supported(d: &FederalDraft) -> FederalResult {
    let r = estimate(d);
    assert_eq!(
        r.state,
        ResultState::EstimatedForSupportedScope,
        "{:#?}",
        r.issues
    );
    r
}
fn unsupported(d: &FederalDraft, section: &str) {
    let r = estimate(d);
    assert_eq!(r.state, ResultState::Unsupported, "{r:?}");
    assert!(
        r.issues.iter().any(|i| i.section.contains(section)),
        "{r:?}"
    );
    assert!(r.tax.is_none() && r.refund.is_none() && r.lines.is_empty());
}
fn send(s: &mut Interview, p: P) -> Reply {
    assert_eq!(P::from_avro(&p.to_avro()).unwrap(), p);
    let r = s.handle(p).unwrap();
    let data = match &r {
        Reply::Response(v) | Reply::Edit(v) => v,
    };
    assert_eq!(P::from_avro(&data.to_avro()).unwrap(), *data);
    r
}
fn restart(d: FederalDraft) -> Interview {
    let saved = TaxAppData::Draft { draft: d };
    let decoded = TaxAppData::from_avro(&saved.to_avro()).unwrap();
    assert_eq!(decoded, saved);
    let TaxAppData::Draft { draft } = decoded;
    Interview::new(draft)
}
fn error(r: Reply) {
    assert!(matches!(r, Reply::Response(P::Problem { .. })), "{r:?}");
}
#[test]
fn independent_wage_interest_joint_charity_fixtures() {
    let mut d = complete(FederalStatus::Single);
    add(
        &mut d,
        income(FederalIncome::Interest {
            data: FederalInterest {
                taxable: 20_000,
                exempt: 0,
                withholding: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    let r = supported(&d);
    assert_eq!(r.tax, Some(504_400));
    assert_eq!(r.refund, Some(145_600));
    assert_eq!(value(&r, "agi"), 60200);
    let gifts = d.deductions.as_mut().unwrap();
    gifts.set_cash_charity(100_000);
    gifts.set_cash_charity_eligible(Answer::Yes);
    assert_eq!(supported(&d).tax, Some(492_400));
    assert_eq!(
        supported(&complete(FederalStatus::Joint)).tax,
        Some(764_000)
    );
}
#[test]
fn published_ordinary_bracket_edges_all_statuses() {
    let fixtures = [
        (
            FederalStatus::Single,
            vec![
                (12400, 1240),
                (50400, 5800),
                (105700, 17966),
                (201775, 41024),
                (256225, 58448),
                (640600, 192979),
            ],
        ),
        (
            FederalStatus::Joint,
            vec![
                (24800, 2480),
                (100800, 11600),
                (211400, 35932),
                (403550, 82048),
                (512450, 116896),
                (768700, 206584),
            ],
        ),
        (
            FederalStatus::Head,
            vec![
                (17700, 1770),
                (67450, 7740),
                (105700, 16155),
                (201750, 39207),
                (256200, 56631),
                (640600, 191171),
            ],
        ),
        (
            FederalStatus::Separate,
            vec![
                (12400, 1240),
                (50400, 5800),
                (105700, 17966),
                (201775, 41024),
                (256225, 58448),
                (384350, 103292),
            ],
        ),
    ];
    for (status, rows) in fixtures {
        for (income, tax) in rows {
            assert_eq!(
                preferential_tax(income, 0, status),
                tax,
                "{status:?} {income}"
            );
            assert!(preferential_tax(income + 1, 0, status) >= tax);
        }
    }
    assert_eq!(preferential_tax(67800, 0, FederalStatus::Surviving), 7640);
}
#[test]
fn preferential_rate_stacking_and_cross_netting() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 5_000_000, 0);
    add(
        &mut d,
        income(FederalIncome::Dividend {
            data: FederalDividend {
                ordinary: 1_000_000,
                qualified: 1_000_000,
                capital_distributions: 0,
                withholding: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    assert_eq!(supported(&d).tax, Some(382_000));
    assert_eq!(preferential_tax(59450, 20000, FederalStatus::Single), 5986);
    add(
        &mut d,
        income(FederalIncome::Sale {
            data: FederalSale {
                term: GainTerm::Short,
                proceeds: 0,
                basis: 500_000,
                wash_sale: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    add(
        &mut d,
        income(FederalIncome::Sale {
            data: FederalSale {
                term: GainTerm::Long,
                proceeds: 1_000_000,
                basis: 0,
                wash_sale: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    let r = supported(&d);
    assert_eq!(value(&r, "capital_gain"), 5000);
    assert_eq!(value(&r, "capital_loss_carryforward"), 0);
}
#[test]
fn capital_loss_limit_and_carryforward() {
    let mut d = complete(FederalStatus::Single);
    add(
        &mut d,
        income(FederalIncome::Sale {
            data: FederalSale {
                term: GainTerm::Long,
                proceeds: 100_000,
                basis: 900_000,
                wash_sale: 100_000,
                special_treatment: Answer::No,
            },
        }),
    );
    let r = supported(&d);
    assert_eq!(value(&r, "capital_gain"), -3000);
    assert_eq!(value(&r, "capital_loss_carryforward"), 4000);
    d.setup.as_mut().unwrap().status = filing_choice(FederalStatus::Separate);
    d.setup.as_mut().unwrap().marital_state = MaritalState::Married {
        living: SpouseLiving::Together,
    };
    assert_eq!(value(&supported(&d), "capital_gain"), -1500);
}
#[test]
fn irs_social_security_worksheet_examples() {
    assert_eq!(
        social_taxable(5980, 18600 + 9400 + 990, 0, FederalStatus::Single, false),
        2990
    );
    assert_eq!(
        social_taxable(
            5600,
            15500 + 14000 + 250 - 1000,
            0,
            FederalStatus::Joint,
            false
        ),
        0
    );
    assert_eq!(
        social_taxable(20000, 100000, 0, FederalStatus::Single, false),
        17000
    );
    assert_eq!(
        social_taxable(20000, 0, 0, FederalStatus::Separate, false),
        8500
    );
    assert_eq!(
        social_taxable(20000, -3000, 0, FederalStatus::Separate, false),
        5950
    );
}
#[test]
fn retirement_senior_and_social_flow() {
    let mut d = complete(FederalStatus::Single);
    d.people.as_mut().unwrap().taxpayer.birth_date = 19500615;
    d.income = vec![wage(1, Owner::Taxpayer, 940_000, 0)];
    add(
        &mut d,
        income(FederalIncome::Retirement {
            data: FederalRetirement {
                kind: RetirementKind::Pension,
                gross: 1_860_000,
                taxable: 1_860_000,
                withholding: 0,
                ordinary_distribution: Answer::Yes,
            },
        }),
    );
    add(
        &mut d,
        income(FederalIncome::Interest {
            data: FederalInterest {
                taxable: 99_000,
                exempt: 0,
                withholding: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    add(
        &mut d,
        income(FederalIncome::SocialSecurity {
            data: FederalSocial {
                benefits: 598_000,
                withholding: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    let r = supported(&d);
    assert_eq!(value(&r, "taxable_social_security"), 2990);
    assert_eq!(value(&r, "senior_deduction"), 6000);
    assert_eq!(value(&r, "deduction"), 18150);
    assert_eq!(r.tax, Some(78_300));
}
#[test]
fn self_employment_and_qbi_independent_fixture() {
    let mut d = complete(FederalStatus::Single);
    d.income.clear();
    add(
        &mut d,
        income(FederalIncome::Business {
            data: FederalBusiness {
                receipts: 6_000_000,
                expenses: vec![FederalBusinessExpense {
                    label: "Supplies".into(),
                    amount: 1_000_000,
                }],
                material_participation: Answer::Yes,
                special_treatment: Answer::No,
            },
        }),
    );
    let r = supported(&d);
    assert_eq!(value(&r, "self_employment_tax"), 7065);
    assert_eq!(value(&r, "agi"), 46467);
    assert_eq!(value(&r, "qbi_deduction"), 6073);
    assert_eq!(r.tax, Some(973_200));
}
#[test]
fn active_qbi_minimum_applies_after_taxable_income_cap() {
    let mut d = complete(FederalStatus::Single);
    d.income.clear();
    add(
        &mut d,
        income(FederalIncome::Business {
            data: FederalBusiness {
                receipts: 108_000,
                expenses: vec![],
                material_participation: Answer::Yes,
                special_treatment: Answer::No,
            },
        }),
    );
    let r = supported(&d);
    assert_eq!(value(&r, "qbi_deduction"), 400);
    assert_eq!(value(&r, "taxable_income"), 0);
}
#[test]
fn child_and_other_dependent_credit() {
    let mut d = complete(FederalStatus::Joint);
    add_child(&mut d);
    add_child(&mut d);
    assert_eq!(supported(&d).tax, Some(324_000));
    d.dependents[1].birth_date = 20040615;
    d.dependents[1].student = true;
    assert_eq!(value(&supported(&d), "child_credit"), 2700);
    d.dependents[1].gross_income = 530_000;
    d.dependents[1].student = false;
    assert_eq!(value(&supported(&d), "child_credit"), 2200);
}
#[test]
fn head_of_household_eitc_and_additional_child_credit() {
    let mut d = complete(FederalStatus::Head);
    d.income[0] = wage(1, Owner::Taxpayer, 2_000_000, 0);
    add_child(&mut d);
    let r = supported(&d);
    assert_eq!(r.tax, Some(0));
    assert_eq!(value(&r, "earned_income_credit"), 4427);
    assert_eq!(value(&r, "additional_child_credit"), 1700);
    assert_eq!(r.refund, Some(612_700));
}
#[test]
fn qualifying_survivor_uses_joint_rates_but_own_credit_thresholds() {
    let mut d = complete(FederalStatus::Surviving);
    d.setup.as_mut().unwrap().marital_state = MaritalState::Widowed2025 {
        could_file_joint: Answer::Yes,
    };
    add_child(&mut d);
    let r = supported(&d);
    assert_eq!(value(&r, "deduction"), 32200);
    assert_eq!(value(&r, "child_credit"), 2200);
    d.dependents[0].residency = DependentResidency::Days { days: 200 };
    unsupported(&d, "Filing status");
}
#[test]
fn eitc_2026_phase_boundaries() {
    assert_eq!(eic_amount(8680, 8680, 0, false), 664);
    assert_eq!(eic_amount(13020, 13020, 1, false), 4427);
    assert_eq!(eic_amount(18290, 18290, 2, false), 7316);
    assert_eq!(eic_amount(18290, 18290, 3, false), 8231);
    assert_eq!(eic_amount(70244, 70244, 3, true), 0);
    assert_eq!(eic_amount(19540, 19540, 0, false), 0);
    assert_eq!(eic_amount(20000, 80000, 1, false), 0);
}
#[test]
fn three_child_payroll_refund_alternative() {
    let mut d = complete(FederalStatus::Head);
    d.income[0] = wage(1, Owner::Taxpayer, 1_000_000, 0);
    for _ in 0..3 {
        add_child(&mut d);
    }
    let r = supported(&d);
    assert_eq!(value(&r, "additional_child_credit"), 1125);
    assert_eq!(value(&r, "earned_income_credit"), 4500);
}
#[test]
fn care_2026_rates_and_joint_earned_income_caps() {
    for (agi, j, rate) in [
        (15000, false, 50),
        (15001, false, 49),
        (45000, false, 35),
        (75000, false, 35),
        (75001, false, 34),
        (103001, false, 20),
        (150000, true, 35),
        (150001, true, 34),
    ] {
        assert_eq!(care_rate(agi, j), rate);
    }
    let mut d = complete(FederalStatus::Joint);
    d.income[0] = wage(1, Owner::Taxpayer, 5_000_000, 0);
    let id = d.next_id;
    add(&mut d, wage(id, Owner::Spouse, 5_000_000, 0));
    add_child(&mut d);
    d.dependents[0].set_care_expenses(300_000);
    d.dependents[0].set_care_period_eligible(Answer::Yes);
    d.credits.as_mut().unwrap().set_care_eligible(Answer::Yes);
    assert_eq!(supported(&d).tax, Some(439_000));
}
#[test]
fn education_credits_and_claimant_refund_restriction() {
    let mut d = complete(FederalStatus::Single);
    d.credits
        .as_mut()
        .unwrap()
        .education
        .push(FederalEducation {
            student: 0,
            expenses: education_expenses(400_000),
            eligible_student: Answer::Yes,
            method: education_choice(EducationMethod::AmericanOpportunity, Answer::Yes),
        });
    let r = supported(&d);
    assert_eq!(r.tax, Some(352_000));
    assert_eq!(value(&r, "refundable_education_credit"), 1000);
    d.people.as_mut().unwrap().taxpayer.birth_date = 20060615;
    d.people.as_mut().unwrap().taxpayer.full_time_student = true;
    d.people.as_mut().unwrap().taxpayer.support_share = SupportShare::LessThanHalf;
    let r = supported(&d);
    assert_eq!(value(&r, "refundable_education_credit"), 0);
    assert_eq!(r.tax, Some(252_000));
}
#[test]
fn lifetime_learning_is_capped_per_return() {
    let mut d = complete(FederalStatus::Joint);
    for student in [0, 1] {
        d.credits
            .as_mut()
            .unwrap()
            .education
            .push(FederalEducation {
                student,
                expenses: education_expenses(1_000_000),
                eligible_student: Answer::Yes,
                method: education_choice(EducationMethod::LifetimeLearning, Answer::No),
            });
    }
    assert_eq!(value(&supported(&d), "education_credit"), 2000);
}
#[test]
fn itemized_medical_salt_mortgage_charity_floor() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 10_000_000, 0);
    let v = d.deductions.as_mut().unwrap();
    itemized(v).medical = 1_000_000;
    itemized(v).state_local_taxes = 1_000_000;
    itemized(v).set_mortgage_interest(1_500_000);
    itemized(v).set_mortgage_within_limit(Answer::Yes);
    v.set_cash_charity(200_000);
    v.set_cash_charity_eligible(Answer::Yes);
    let r = supported(&d);
    assert_eq!(value(&r, "deduction"), 29000);
    assert_eq!(r.tax, Some(1_033_200));
}
#[test]
fn loan_phaseout_and_hsa_limits() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 9_250_000, 0);
    let a = d.adjustments.as_mut().unwrap();
    a.set_student_loan_interest(250_000);
    a.set_student_loan_eligible(Answer::Yes);
    assert_eq!(value(&supported(&d), "adjustments"), 1250);
    d.adjustments.as_mut().unwrap().hsa.push(FederalHsa {
        owner: Owner::Taxpayer,
        coverage: HsaCoverage::SelfOnly,
        personal: 440_000,
        employer: 0,
        distributions: 0,
        fully_eligible: Answer::Yes,
    });
    assert!(supported(&d).tax.is_some());
    d.adjustments.as_mut().unwrap().hsa[0].personal += 100;
    unsupported(&d, "HSA");
}
#[test]
fn additional_medicare_and_niit_are_not_silently_zero() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 21_000_000, 0);
    add(
        &mut d,
        income(FederalIncome::Interest {
            data: FederalInterest {
                taxable: 1_000_000,
                exempt: 0,
                withholding: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    let r = supported(&d);
    assert_eq!(value(&r, "additional_medicare_tax"), 90);
    assert_eq!(value(&r, "net_investment_income_tax"), 380);
    assert_eq!(value(&r, "payments"), 90);
}
#[test]
fn unsupported_sources_and_unknowns_never_publish_totals() {
    let mut d = complete(FederalStatus::Single);
    d.screening.as_mut().unwrap().foreign = Answer::NotSure;
    unsupported(&d, "Coverage");
    d.screening.as_mut().unwrap().foreign = Answer::No;
    add(
        &mut d,
        income(FederalIncome::Retirement {
            data: FederalRetirement {
                kind: RetirementKind::Pension,
                gross: 10_000,
                taxable: 10_000,
                withholding: 0,
                ordinary_distribution: Answer::NotSure,
            },
        }),
    );
    unsupported(&d, "Income");
}
#[test]
fn domain_validation_rejects_inconsistent_documents() {
    let e = income(FederalIncome::Dividend {
        data: FederalDividend {
            ordinary: 100,
            qualified: 200,
            capital_distributions: 0,
            withholding: 0,
            special_treatment: Answer::No,
        },
    });
    assert!(crate::federal_rules::validate_income(&e).is_err());
    let e = income(FederalIncome::Sale {
        data: FederalSale {
            term: GainTerm::Short,
            proceeds: 100,
            basis: 200,
            wash_sale: 101,
            special_treatment: Answer::No,
        },
    });
    assert!(crate::federal_rules::validate_income(&e).is_err());
    for date in [20260229, 20261301, 20270001, 0] {
        assert!(!valid_birth(date));
    }
    assert!(valid_birth(20000229));
}
#[test]
fn exact_cents_sum_before_tax_line_rounding() {
    let mut d = complete(FederalStatus::Single);
    for _ in 0..3 {
        add(
            &mut d,
            income(FederalIncome::Interest {
                data: FederalInterest {
                    taxable: 49,
                    exempt: 0,
                    withholding: 0,
                    special_treatment: Answer::No,
                },
            }),
        );
    }
    assert_eq!(value(&supported(&d), "agi"), 60001);
    assert_eq!(crate::federal_rules::dollars(-150), -2);
}
#[test]
fn maximum_supported_inputs_do_not_overflow() {
    let mut d = complete(FederalStatus::Single);
    d.income.clear();
    for _ in 0..100 {
        add(
            &mut d,
            income(FederalIncome::Interest {
                data: FederalInterest {
                    taxable: 100_000_000_000,
                    exempt: 0,
                    withholding: 0,
                    special_treatment: Answer::No,
                },
            }),
        );
    }
    let result = estimate(&d);
    assert_ne!(result.state, ResultState::Incomplete);
}
#[test]
fn accepted_storage_round_trip_and_invalid_identity_block() {
    let d = complete(FederalStatus::Single);
    let mut s = restart(d.clone());
    assert!(matches!(
        send(&mut s, P::OpenInterview),
        Reply::Response(P::Review { .. })
    ));
    let mut invalid = d;
    invalid.income[0].id = 0;
    let mut s = restart(invalid.clone());
    error(send(&mut s, P::OpenInterview));
    assert_eq!(s.draft, invalid);
    assert!(!s.dirty);
}
#[test]
fn preview_is_pure_and_excludes_unrelated_income_fields() {
    let mut session = Interview::new(complete(FederalStatus::Single));
    let before = session.draft.clone();
    let mut entry = before.income[0].clone();
    if let FederalIncome::Wage { data } = &mut entry.income {
        data.wages = 6_200_000;
    }
    let request = P::PreviewIncome {
        value: IncomeAmountInput {
            income: entry.income,
        },
    };
    for _ in 0..2 {
        assert!(matches!(
            send(&mut session, request.clone()),
            Reply::Response(P::IncomePreview {
                preview: Some(6_200_000)
            })
        ));
    }
    assert_eq!(session.draft, before);
    assert!(!session.dirty);
}
#[test]
fn income_array_saves_atomically_and_ignores_preview() {
    let original = complete(FederalStatus::Single);
    let mut session = Interview::new(original.clone());
    let request = |d: &FederalDraft, records| P::SaveIncomes {
        cookie: d.cookie.clone(),
        revision: d.revision,
        page: 3,
        previous: 2,
        review: true,
        progress: vec![],
        records,
    };
    let records: alloc::vec::Vec<_> = original
        .income
        .iter()
        .cloned()
        .map(|value| IncomeEditorItem {
            value,
            preview: Some(-123),
        })
        .collect();
    send(&mut session, request(&original, records.clone()));
    assert_eq!(session.draft, original);
    assert!(!session.dirty);
    let mut invalid = records.clone();
    invalid.push(records[0].clone());
    error(send(&mut session, request(&original, invalid)));
    assert_eq!(session.draft, original);
    let mut changed = records;
    if let FederalIncome::Wage { data } = &mut changed[0].value.income {
        data.wages = 6_100_000;
    }
    let save = request(&original, changed);
    send(&mut session, save.clone());
    assert_eq!(session.draft.income.len(), 1);
    assert_eq!(session.draft.income[0].id, 1);
    assert_eq!(session.draft.revision, original.revision + 1);
    error(send(&mut session, save));
    let accepted = session.draft.clone();
    send(&mut session, request(&accepted, vec![]));
    assert!(session.draft.income.is_empty());
    assert!(session.draft.income_complete);
}
#[test]
fn basis_change_erases_later_pages() {
    let d = complete(FederalStatus::Single);
    let mut s = Interview::new(d.clone());
    let mut setup = d.setup.clone().unwrap();
    setup.basis = AmountBasis::Actual;
    let request = P::SaveSetup {
        cookie: d.cookie.clone(),
        revision: 0,
        page: 0,
        previous: 0,
        review: true,
        progress: vec![],
        value: setup.clone(),
        confirm_basis_change: false,
    };
    error(send(&mut s, request));
    assert_eq!(s.draft, d);
    send(
        &mut s,
        P::SaveSetup {
            cookie: d.cookie,
            revision: 0,
            page: 0,
            previous: 0,
            review: true,
            progress: vec![],
            value: setup,
            confirm_basis_change: true,
        },
    );
    assert!(s.draft.income.is_empty());
    assert_eq!(s.draft.first_incomplete(), 1);
    assert_eq!(estimate(&s.draft).state, ResultState::Incomplete);
    let mut restarted = restart(s.draft);
    assert!(matches!(
        send(&mut restarted, P::OpenInterview),
        Reply::Response(P::BeginPeople { .. })
    ));
}
#[test]
fn response_cannot_be_submitted_as_request() {
    let mut s = Interview::new(complete(FederalStatus::Single));
    assert!(s.handle(P::IncomePreview { preview: Some(1) }).is_none());
}
#[test]
fn finish_is_explicit_correlated_workflow_and_reopens_review() {
    let d = complete(FederalStatus::Single);
    let mut s = Interview::new(d.clone());
    assert!(matches!(
        send(
            &mut s,
            P::StartFinish {
                cookie: d.cookie.clone(),
                revision: 0
            }
        ),
        Reply::Response(P::BeginFinish { .. })
    ));
    assert!(!s.dirty);
    assert!(matches!(
        send(
            &mut s,
            P::Finish {
                cookie: d.cookie,
                revision: 0,
                page: 9,
                previous: 9,
                review: true,
                progress: vec![]
            }
        ),
        Reply::Response(P::Finished { .. })
    ));
    assert!(s.draft.finished);
    let mut restarted = restart(s.draft);
    assert!(matches!(
        send(&mut restarted, P::OpenInterview),
        Reply::Response(P::Review { .. })
    ));
}
#[test]
fn new_workflow_restarts_after_each_accepted_section() {
    let fixture = complete(FederalStatus::Single);
    let mut s = Interview::new(FederalDraft::empty(fixture.cookie.clone()));
    let mut reply = send(&mut s, P::OpenInterview);
    for _ in 0..12 {
        let p = match reply {
            Reply::Response(P::BeginSetup {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
            }) => P::SaveSetup {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                value: fixture.setup.clone().unwrap(),
                confirm_basis_change: false,
            },
            Reply::Response(P::BeginPeople {
                allowed_spouse,
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
            }) => P::SavePeople {
                allowed_spouse,
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                value: fixture.people.clone().unwrap(),
            },
            Reply::Edit(request @ P::SaveDependents { .. }) => request,
            Reply::Edit(request @ P::SaveIncomes { .. }) => request,
            Reply::Response(P::BeginAdjustments {
                allowed_ira_spouse,
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
            }) => P::SaveAdjustments {
                allowed_ira_spouse,
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                value: fixture.adjustments.clone().unwrap(),
            },
            Reply::Response(P::BeginDeductions {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
            }) => P::SaveDeductions {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                value: fixture.deductions.clone().unwrap(),
            },
            Reply::Response(P::BeginCredits {
                students: _,
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
            }) => P::SaveCredits {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                value: fixture.credits.clone().unwrap(),
            },
            Reply::Response(P::BeginScreening {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
            }) => P::SaveScreening {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                value: fixture.screening.clone().unwrap(),
            },
            Reply::Response(P::BeginPayments {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
            }) => P::SavePayments {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                value: fixture.payments.clone().unwrap(),
            },
            Reply::Response(P::Review { result, .. }) => {
                assert_eq!(result.state, ResultState::EstimatedForSupportedScope);
                return;
            }
            other => panic!("unexpected {other:?}"),
        };
        let _ = send(&mut s, p);
        assert!(s.dirty);
        s = restart(s.draft);
        reply = send(&mut s, P::OpenInterview);
    }
    panic!("workflow did not complete");
}

#[test]
fn student_identity_survives_roster_reordering_and_removed_people_are_repairable() {
    let mut d = complete(FederalStatus::Joint);
    add_child(&mut d);
    add_child(&mut d);
    let second = d.dependents[1].id;
    d.credits.as_mut().unwrap().students = crate::federal_rules::students(&d);
    d.credits
        .as_mut()
        .unwrap()
        .education
        .push(FederalEducation {
            student: 3,
            expenses: education_expenses(100_000),
            eligible_student: Answer::Yes,
            method: education_choice(EducationMethod::LifetimeLearning, Answer::No),
        });
    let before = supported(&d);
    d.dependents.remove(0);
    let editor = crate::federal_rules::editable_credits(&d, d.credits.as_ref().unwrap());
    assert_eq!(editor.education[0].student, 2);
    assert_eq!(editor.students[2].id, second);
    assert_eq!(
        value(&supported(&d), "education_credit"),
        value(&before, "education_credit")
    );
    d.dependents.clear();
    unsupported(&d, "Education");
    let editor = crate::federal_rules::editable_credits(&d, d.credits.as_ref().unwrap());
    assert_eq!(
        editor.students[editor.education[0].student as usize].id,
        second
    );
    let mut s = Interview::new(d.clone());
    error(send(
        &mut s,
        P::SaveCredits {
            cookie: d.cookie.clone(),
            revision: d.revision,
            page: 6,
            previous: 9,
            review: true,
            progress: vec![],
            value: editor.clone(),
        },
    ));
    assert_eq!(s.draft, d);
    let mut repaired = editor;
    repaired.education.clear();
    assert!(matches!(
        send(
            &mut s,
            P::SaveCredits {
                cookie: d.cookie,
                revision: d.revision,
                page: 6,
                previous: 9,
                review: true,
                progress: vec![],
                value: repaired
            }
        ),
        Reply::Response(P::BeginScreening { review: false, .. })
    ));
}
#[test]
fn care_ceiling_counts_a_second_child_even_when_only_one_has_expenses() {
    let mut d = complete(FederalStatus::Joint);
    d.income[0] = wage(1, Owner::Taxpayer, 5_000_000, 0);
    let id = d.next_id;
    add(&mut d, wage(id, Owner::Spouse, 5_000_000, 0));
    add_child(&mut d);
    add_child(&mut d);
    d.dependents[0].set_care_expenses(600_000);
    d.dependents[0].set_care_period_eligible(Answer::Yes);
    d.credits.as_mut().unwrap().set_care_eligible(Answer::Yes);
    assert_eq!(value(&supported(&d), "care_credit"), 2100);
}
#[test]
fn failed_deletion_at_revision_limit_is_not_reported_as_success() {
    let mut d = complete(FederalStatus::Single);
    d.revision = i64::MAX - 1;
    let mut s = Interview::new(d.clone());
    error(send(
        &mut s,
        P::SaveIncomes {
            cookie: d.cookie.clone(),
            revision: d.revision,
            page: 3,
            previous: 3,
            review: true,
            progress: vec![],
            records: vec![],
        },
    ));
    assert_eq!(s.draft, d);
    assert!(!s.dirty);
}
#[test]
fn joint_status_change_erases_later_pages() {
    let d = complete(FederalStatus::Single);
    let mut s = Interview::new(d.clone());
    let mut setup = d.setup.clone().unwrap();
    setup.status = filing_choice(FederalStatus::Joint);
    setup.marital_state = MaritalState::Married {
        living: SpouseLiving::Together,
    };
    send(
        &mut s,
        P::SaveSetup {
            cookie: d.cookie.clone(),
            revision: d.revision,
            page: 0,
            previous: 9,
            review: true,
            progress: vec![],
            value: setup,
            confirm_basis_change: false,
        },
    );
    assert!(s.draft.income.is_empty());
    assert_eq!(s.draft.first_incomplete(), 1);
    let old = s.draft.clone();
    error(send(
        &mut s,
        P::SavePeople {
            allowed_spouse: vec![1],
            cookie: old.cookie,
            revision: old.revision,
            page: 1,
            previous: 0,
            review: false,
            progress: vec![],
            value: d.people.unwrap(),
        },
    ));
    assert_eq!(s.draft.revision, 1);
}

#[test]
fn young_filer_investment_tax_is_screened_even_when_not_claimable() {
    let mut d = complete(FederalStatus::Single);
    d.people.as_mut().unwrap().taxpayer.birth_date = 20080615;
    d.people.as_mut().unwrap().taxpayer.support_share = SupportShare::ExactlyHalf;
    add(
        &mut d,
        income(FederalIncome::Interest {
            data: FederalInterest {
                taxable: 300_000,
                exempt: 0,
                withholding: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    unsupported(&d, "Investment tax");
    d.people.as_mut().unwrap().taxpayer.support_share = SupportShare::MoreThanHalf;
    assert!(supported(&d).tax.is_some());
}

fn saver(you: i64, spouse: i64) -> FederalSaver {
    FederalSaver {
        taxpayer_contributions: you,
        spouse_contributions: spouse.into(),
        eligible_contributions: Answer::Yes,
        distributions: vec![],
        reviewed_distributions: Answer::Yes,
    }
}

#[test]
fn saver_2026_rate_boundaries_for_every_status() {
    use crate::federal_rules::saver_rate;
    for (status, limits) in [
        (FederalStatus::Joint, [48500, 52500, 80500]),
        (FederalStatus::Head, [36375, 39375, 60375]),
        (FederalStatus::Single, [24250, 26250, 40250]),
        (FederalStatus::Separate, [24250, 26250, 40250]),
        (FederalStatus::Surviving, [24250, 26250, 40250]),
    ] {
        for (agi, expected) in [
            (0, 50),
            (limits[0], 50),
            (limits[0] + 1, 20),
            (limits[1], 20),
            (limits[1] + 1, 10),
            (limits[2], 10),
            (limits[2] + 1, 0),
        ] {
            assert_eq!(saver_rate(agi, status), expected, "{status:?}, {agi}");
        }
    }
}

#[test]
fn saver_single_joint_and_nonrefundable_limits() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 2_500_000, 0);
    d.credits.as_mut().unwrap().saver = (Some(saver(300_000, 0))).into();
    let r = supported(&d);
    assert_eq!(value(&r, "agi"), 25000); // W-2 already excludes pretax deferrals.
    assert_eq!(value(&r, "income_tax"), 890);
    assert_eq!(value(&r, "saver_credit"), 400);
    assert_eq!(r.tax, Some(49_000));
    d.income[0] = wage(1, Owner::Taxpayer, 2_400_000, 0);
    assert_eq!(value(&supported(&d), "saver_credit"), 790);
    assert_eq!(supported(&d).tax, Some(0));
    d.income[0] = wage(1, Owner::Taxpayer, 1_000_000, 0);
    assert_eq!(value(&supported(&d), "saver_credit"), 0);

    let mut d = complete(FederalStatus::Joint);
    d.income[0] = wage(1, Owner::Taxpayer, 2_500_000, 0);
    let id = d.next_id;
    add(&mut d, wage(id, Owner::Spouse, 2_500_000, 0));
    d.credits.as_mut().unwrap().saver = (Some(saver(200_000, 200_000))).into();
    let r = supported(&d);
    assert_eq!(value(&r, "income_tax"), 1780);
    assert_eq!(value(&r, "saver_credit"), 800);
    assert_eq!(r.tax, Some(98_000));
}

#[test]
fn saver_spousal_distribution_exception_and_ineligible_recipient() {
    use crate::federal_rules::saver_potential;
    let mut p = complete(FederalStatus::Joint).people.unwrap();
    let mut f = saver(700_000, 700_000);
    // Form 8880 example adapted to the 2026 lookback: own 5,000 current-year
    // distribution and spouse 2,000 prior-year distribution when they did not file together.
    f.distributions = vec![
        FederalSaverDistribution {
            owner: Owner::Taxpayer,
            amount: 500_000,
            year: distribution_period(SaverDistributionYear::TaxYear, Answer::No),
        },
        FederalSaverDistribution {
            owner: Owner::Spouse,
            amount: 200_000,
            year: distribution_period(SaverDistributionYear::PriorTwo, Answer::No),
        },
    ];
    assert_eq!(
        saver_potential(&f, &p, FederalStatus::Joint, 40000),
        Ok(1000)
    );
    f.distributions[1].year = distribution_period(f.distributions[1].year(), Answer::Yes);
    assert_eq!(saver_potential(&f, &p, FederalStatus::Joint, 40000), Ok(0));
    f.distributions[1].year = distribution_period(f.distributions[1].year(), Answer::NotSure);
    assert!(saver_potential(&f, &p, FederalStatus::Joint, 40000).is_err());
    f.distributions[1].year = distribution_period(
        SaverDistributionYear::TaxYear,
        f.distributions[1].joint_with_current_spouse(),
    );
    assert_eq!(saver_potential(&f, &p, FederalStatus::Joint, 40000), Ok(0));
    // Even an ineligible spouse's distribution reduces the eligible filer's base.
    p.spouse.as_mut().unwrap().full_time_student = true;
    f.spouse_contributions = 0.into();
    f.distributions.remove(0);
    f.taxpayer_contributions = 200_000;
    assert_eq!(saver_potential(&f, &p, FederalStatus::Joint, 40000), Ok(0));
    f.distributions[0].year = distribution_period(
        SaverDistributionYear::FollowingYear,
        f.distributions[0].joint_with_current_spouse(),
    );
    f.distributions[0].year = distribution_period(f.distributions[0].year(), Answer::No);
    assert_eq!(
        saver_potential(&f, &p, FederalStatus::Joint, 40000),
        Ok(1000)
    );
    f.distributions[0].year = distribution_period(f.distributions[0].year(), Answer::Yes);
    assert_eq!(saver_potential(&f, &p, FederalStatus::Joint, 40000), Ok(0));
}

#[test]
fn saver_age_student_and_dependent_eligibility() {
    use crate::federal_rules::saver_potential;
    let mut p = complete(FederalStatus::Single).people.unwrap();
    let f = saver(200_000, 0);
    p.taxpayer.birth_date = 20090101;
    assert_eq!(
        saver_potential(&f, &p, FederalStatus::Single, 20000),
        Ok(1000)
    );
    p.taxpayer.birth_date = 20090102;
    assert_eq!(saver_potential(&f, &p, FederalStatus::Single, 20000), Ok(0));
    p.taxpayer.birth_date = 19800615;
    p.taxpayer.full_time_student = true;
    assert_eq!(saver_potential(&f, &p, FederalStatus::Single, 20000), Ok(0));
    p.taxpayer.full_time_student = false;
    p.taxpayer.claimable = Answer::Yes;
    assert_eq!(saver_potential(&f, &p, FederalStatus::Single, 20000), Ok(0));
}

#[test]
fn saver_follows_education_care_and_precedes_child_credit() {
    let mut d = complete(FederalStatus::Joint);
    d.income[0] = wage(1, Owner::Taxpayer, 2_500_000, 0);
    let id = d.next_id;
    add(&mut d, wage(id, Owner::Spouse, 2_500_000, 0));
    add_child(&mut d);
    let without = supported(&d);
    assert_eq!(value(&without, "child_credit"), 1780);
    assert_eq!(value(&without, "additional_child_credit"), 420);
    d.credits.as_mut().unwrap().saver = (Some(saver(200_000, 200_000))).into();
    let r = supported(&d);
    assert_eq!(value(&r, "saver_credit"), 800);
    assert_eq!(value(&r, "child_credit"), 980);
    assert_eq!(value(&r, "additional_child_credit"), 1220);
    d.credits
        .as_mut()
        .unwrap()
        .education
        .push(FederalEducation {
            student: 0,
            expenses: education_expenses(500_000),
            eligible_student: Answer::Yes,
            method: education_choice(EducationMethod::LifetimeLearning, Answer::No),
        });
    d.dependents[0].set_care_expenses(100_000);
    d.dependents[0].set_care_period_eligible(Answer::Yes);
    d.credits.as_mut().unwrap().set_care_eligible(Answer::Yes);
    let r = supported(&d);
    assert_eq!(value(&r, "education_credit"), 1000);
    assert_eq!(value(&r, "care_credit"), 350);
    assert_eq!(value(&r, "saver_credit"), 430);
    assert_eq!(value(&r, "child_credit"), 0);
    assert_eq!(value(&r, "additional_child_credit"), 1700);
    assert_eq!(r.tax, Some(0));
}

#[test]
fn saver_aggregates_distribution_cents_before_rounding() {
    use crate::federal_rules::saver_potential;
    let p = complete(FederalStatus::Single).people.unwrap();
    let mut f = saver(100_000, 0);
    f.distributions = vec![
        FederalSaverDistribution {
            owner: Owner::Taxpayer,
            amount: 49,
            year: distribution_period(SaverDistributionYear::PriorOne, Answer::No),
        };
        4
    ];
    assert_eq!(
        saver_potential(&f, &p, FederalStatus::Single, 20000),
        Ok(499)
    );
}

#[test]
fn saver_uncertainty_and_retained_spouse_answers_block_estimate() {
    let mut d = complete(FederalStatus::Single);
    d.credits.as_mut().unwrap().saver = (Some(saver(200_000, 0))).into();
    d.credits
        .as_mut()
        .unwrap()
        .saver
        .as_mut()
        .unwrap()
        .reviewed_distributions = Answer::NotSure;
    unsupported(&d, "Saver");
    let f = d.credits.as_mut().unwrap().saver.as_mut().unwrap();
    f.reviewed_distributions = Answer::Yes;
    f.eligible_contributions = Answer::No;
    unsupported(&d, "Saver");
    let f = d.credits.as_mut().unwrap().saver.as_mut().unwrap();
    f.eligible_contributions = Answer::Yes;
    f.spouse_contributions = 1.into();
    unsupported(&d, "Saver");
    let f = d.credits.as_mut().unwrap().saver.as_mut().unwrap();
    f.spouse_contributions = 0.into();
    f.distributions.push(FederalSaverDistribution {
        owner: Owner::Spouse,
        amount: 100,
        year: distribution_period(SaverDistributionYear::PriorOne, Answer::No),
    });
    unsupported(&d, "Saver");
}

#[test]
fn saver_accepted_credit_page_persists_reopens_and_rejects_stale_edit() {
    let d = complete(FederalStatus::Single);
    let mut f = d.credits.clone().unwrap();
    f.saver = (Some(saver(200_000, 0))).into();
    f.saver
        .as_mut()
        .unwrap()
        .distributions
        .push(FederalSaverDistribution {
            owner: Owner::Taxpayer,
            amount: 10000,
            year: distribution_period(SaverDistributionYear::PriorOne, Answer::No),
        });
    let mut s = restart(d.clone());
    let request = P::SaveCredits {
        cookie: d.cookie.clone(),
        revision: d.revision,
        page: 6,
        previous: 9,
        review: true,
        progress: vec![],
        value: f.clone(),
    };
    assert!(matches!(
        send(&mut s, request.clone()),
        Reply::Response(P::BeginScreening { review: false, .. })
    ));
    assert!(s.dirty);
    assert_eq!(s.draft.revision, d.revision + 1);
    let accepted = s.draft.clone();
    let mut s = restart(accepted.clone());
    assert_eq!(s.draft.credits, Some(f.clone()));
    let reopened = send(
        &mut s,
        P::Navigate {
            cookie: accepted.cookie.clone(),
            revision: accepted.revision,
            section: TaxSection::Credits,
            allowed_sections: (0..=9).collect(),
        },
    );
    assert!(
        matches!(reopened, Reply::Edit(P::SaveCredits { value, revision, .. })
        if value == f && revision == accepted.revision)
    );
    error(send(&mut s, request));
    assert_eq!(s.draft, accepted);
    assert!(!s.dirty);
    let mut malformed = accepted.clone();
    malformed
        .credits
        .as_mut()
        .unwrap()
        .saver
        .as_mut()
        .unwrap()
        .taxpayer_contributions = -1;
    assert!(!malformed.valid_stored());
    let mut malformed = accepted;
    let f = malformed.credits.as_mut().unwrap().saver.as_mut().unwrap();
    f.distributions = vec![f.distributions[0].clone(); 101];
    assert!(!malformed.valid_stored());
}

fn ira_facts(traditional: i64, roth: i64, covered: Answer) -> FederalIras {
    FederalIras {
        taxpayer: FederalIraContribution {
            traditional,
            roth,
            covered_at_work: covered,
        },
        spouse: IraSpouse::NotApplicable,
        regular_contributions: Answer::Yes,
    }
}
#[test]
fn ira_2026_phaseouts_round_up_and_preserve_minimum_until_endpoint() {
    use crate::ira::phase_limit;
    for (start, width) in [(81000, 10000), (129000, 20000), (242000, 10000), (0, 10000)] {
        assert_eq!(phase_limit(7500, start, start, width), 7500);
        assert_eq!(phase_limit(7500, start + width / 2, start, width), 3750);
        assert_eq!(phase_limit(7500, start + width - 1, start, width), 200);
        assert_eq!(phase_limit(7500, start + width, start, width), 0);
        assert_eq!(phase_limit(8600, start + width / 2, start, width), 4300);
    }
    assert_eq!(phase_limit(7500, 81101, 81000, 10000), 7430);
}
#[test]
fn ira_deduction_uses_coverage_and_spousal_compensation() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 8_600_000, 0);
    d.adjustments.as_mut().unwrap().iras = (Some(ira_facts(750_000, 0, Answer::Yes))).into();
    let r = supported(&d);
    assert_eq!(value(&r, "ira_deduction"), 3750);
    assert_eq!(value(&r, "taxpayer_nondeductible_ira"), 3750);
    assert_eq!(value(&r, "agi"), 82250);
    d.adjustments
        .as_mut()
        .unwrap()
        .iras
        .as_mut()
        .unwrap()
        .taxpayer
        .covered_at_work = Answer::No;
    assert_eq!(value(&supported(&d), "ira_deduction"), 7500);
    let mut d = complete(FederalStatus::Joint);
    let mut f = ira_facts(750_000, 0, Answer::Yes);
    f.spouse = IraSpouse::Joint {
        facts: FederalIraContribution {
            traditional: 750_000,
            roth: 0,
            covered_at_work: Answer::No,
        },
    };
    d.adjustments.as_mut().unwrap().iras = (Some(f)).into();
    assert_eq!(value(&supported(&d), "ira_deduction"), 15000);
    // The nonworking spouse shares compensation, but it cannot fund both accounts twice.
    d.income[0] = wage(1, Owner::Taxpayer, 1_400_000, 0);
    unsupported(&d, "IRA");
}
#[test]
fn ira_separate_apart_and_surviving_spouse_thresholds() {
    let d = complete(FederalStatus::Separate);
    let mut setup = d.setup.unwrap();
    let people = d.people.unwrap();
    let mut f = ira_facts(750_000, 0, Answer::Yes);
    f.spouse = IraSpouse::SeparateTogether {
        covered_at_work: Answer::No,
    };
    assert_eq!(
        crate::ira::calculate(&f, &people, &setup, [86000, 0], 86000)
            .unwrap()
            .deduction,
        0
    );
    setup.marital_state = MaritalState::Married {
        living: SpouseLiving::AllYear,
    };
    f.spouse = IraSpouse::NotApplicable;
    assert_eq!(
        crate::ira::calculate(&f, &people, &setup, [86000, 0], 86000)
            .unwrap()
            .deduction,
        3750
    );
    setup.status = filing_choice(FederalStatus::Surviving);
    assert_eq!(
        crate::ira::calculate(&f, &people, &setup, [139000, 0], 139000)
            .unwrap()
            .deduction,
        3750
    );
    setup.status = filing_choice(FederalStatus::Separate);
    setup.marital_state = MaritalState::Married {
        living: SpouseLiving::Together,
    };
    f.taxpayer.covered_at_work = Answer::No;
    f.spouse = IraSpouse::SeparateTogether {
        covered_at_work: Answer::Yes,
    };
    assert_eq!(
        crate::ira::calculate(&f, &people, &setup, [86000, 0], 86000)
            .unwrap()
            .deduction,
        0
    );
}
#[test]
fn ira_social_security_recomputed_then_student_loan_phaseout() {
    let mut d = complete(FederalStatus::Single);
    d.people.as_mut().unwrap().taxpayer.birth_date = 19650615;
    d.income[0] = wage(1, Owner::Taxpayer, 3_000_000, 0);
    add(
        &mut d,
        income(FederalIncome::SocialSecurity {
            data: FederalSocial {
                benefits: 2_000_000,
                withholding: 0,
                special_treatment: Answer::No,
            },
        }),
    );
    let a = d.adjustments.as_mut().unwrap();
    a.iras = (Some(ira_facts(860_000, 0, Answer::Yes))).into();
    a.set_student_loan_interest(250_000);
    a.set_student_loan_eligible(Answer::Yes);
    d.credits.as_mut().unwrap().saver = (Some(saver(0, 0))).into();
    let r = supported(&d);
    assert_eq!(value(&r, "ira_deduction"), 8600);
    // (30,000 - 8,600 + half of 20,000 - 25,000) * 50% = 3,200.
    assert_eq!(value(&r, "taxable_social_security"), 3200);
    assert_eq!(value(&r, "agi"), 22100);
    assert_eq!(value(&r, "adjustments"), 11100);
}
#[test]
fn roth_limits_share_annual_cap_and_feed_saver_without_second_deduction() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 2_500_000, 0);
    d.adjustments.as_mut().unwrap().iras = (Some(ira_facts(0, 200_000, Answer::No))).into();
    unsupported(&d, "Saver"); // distribution review cannot silently be skipped
    d.credits.as_mut().unwrap().saver = (Some(saver(100_000, 0))).into();
    let r = supported(&d);
    assert_eq!(value(&r, "agi"), 25000);
    assert_eq!(value(&r, "saver_credit"), 400); // one shared $2,000 cap
    let mut f = ira_facts(400_000, 350_000, Answer::Yes);
    let p = d.people.as_ref().unwrap();
    let s = d.setup.as_ref().unwrap();
    assert!(crate::ira::validate_roth(&f, p, s, [160500, 0], 160500).is_ok());
    f.taxpayer.roth = 350_001;
    assert!(crate::ira::validate_roth(&f, p, s, [160500, 0], 160500).is_err());
    f.taxpayer.traditional = 0;
    f.taxpayer.roth = 20_000;
    assert!(crate::ira::validate_roth(&f, p, s, [167999, 0], 167999).is_ok());
    assert!(crate::ira::validate_roth(&f, p, s, [168000, 0], 168000).is_err());
}
#[test]
fn ira_age_catchup_exact_cent_limits_and_unknown_coverage() {
    let mut d = complete(FederalStatus::Single);
    d.people.as_mut().unwrap().taxpayer.birth_date = 19770101;
    d.adjustments.as_mut().unwrap().iras = (Some(ira_facts(860_000, 0, Answer::Yes))).into();
    assert_eq!(value(&supported(&d), "ira_deduction"), 8600);
    d.people.as_mut().unwrap().taxpayer.birth_date = 19770102;
    unsupported(&d, "IRA");
    let f = d.adjustments.as_mut().unwrap().iras.as_mut().unwrap();
    f.taxpayer.traditional = 750_001;
    unsupported(&d, "IRA");
    let f = d.adjustments.as_mut().unwrap().iras.as_mut().unwrap();
    f.taxpayer.traditional = 750_000;
    f.taxpayer.covered_at_work = Answer::NotSure;
    unsupported(&d, "IRA");
}
#[test]
fn ira_accepted_page_round_trip_and_special_basis_guard() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 9_100_000, 0);
    d.adjustments.as_mut().unwrap().iras = (Some(ira_facts(750_000, 0, Answer::Yes))).into();
    let restored = restart(d.clone());
    assert_eq!(restored.draft, d);
    assert_eq!(
        value(&supported(&restored.draft), "taxpayer_nondeductible_ira"),
        7500
    );
    add(
        &mut d,
        income(FederalIncome::Retirement {
            data: FederalRetirement {
                kind: RetirementKind::Pension,
                gross: 100_000,
                taxable: 100_000,
                withholding: 0,
                ordinary_distribution: Answer::Yes,
            },
        }),
    );
    unsupported(&d, "IRA");
}

#[test]
fn schedule_one_a_phaseout_rounding_and_return_limits() {
    use crate::federal_rules::additional_deductions as calc;
    assert_eq!(
        calc(150000, FederalStatus::Single, 30000, 20000, 10000),
        [25000, 12500, 0]
    );
    assert_eq!(
        calc(150999, FederalStatus::Single, 30000, 20000, 0),
        [25000, 12500, 0]
    );
    assert_eq!(
        calc(151000, FederalStatus::Single, 30000, 20000, 0),
        [24900, 12400, 0]
    );
    assert_eq!(
        calc(100000, FederalStatus::Single, 0, 0, 20000),
        [0, 0, 10000]
    );
    assert_eq!(
        calc(100001, FederalStatus::Single, 0, 0, 20000),
        [0, 0, 9800]
    );
    assert_eq!(
        calc(101000, FederalStatus::Single, 0, 0, 20000),
        [0, 0, 9800]
    );
    assert_eq!(
        calc(101001, FederalStatus::Single, 0, 0, 20000),
        [0, 0, 9600]
    );
    assert_eq!(
        calc(300000, FederalStatus::Joint, 60000, 60000, 0),
        [25000, 25000, 0]
    );
    assert_eq!(
        calc(10000, FederalStatus::Separate, 1000, 1000, 1000),
        [0, 0, 1000]
    );
    assert_eq!(
        calc(200000, FederalStatus::Surviving, 0, 0, 10000),
        [0, 0, 0]
    );
}
fn work_deductions(tips: i64, overtime: i64) -> FederalWorkDeductions {
    FederalWorkDeductions {
        tips_claim: TipsClaim::Yes {
            tips,
            tips_eligible: Answer::Yes,
        },
        overtime_claim: OvertimeClaim::Yes {
            overtime,
            overtime_eligible: Answer::Yes,
        },
    }
}
#[test]
fn employee_tips_and_overtime_change_taxable_income_not_payroll_or_agi() {
    let mut d = complete(FederalStatus::Single);
    let baseline = supported(&d);
    let FederalIncome::Wage { data: w } = &mut d.income[0].income else {
        panic!()
    };
    w.social_wages = 4_000_000;
    w.social_tips = 2_000_000;
    w.work_deductions = (Some(work_deductions(2_000_000, 500_000))).into();
    let r = supported(&d);
    assert_eq!(value(&r, "agi"), 60000);
    assert_eq!(value(&r, "tips_deduction"), 20000);
    assert_eq!(value(&r, "overtime_deduction"), 5000);
    assert_eq!(value(&r, "taxable_income"), 18900);
    assert_eq!(r.tax, Some(202_000));
    assert_eq!(r.payments, baseline.payments);
    let FederalIncome::Wage { data: w } = &mut d.income[0].income else {
        panic!()
    };
    w.work_deductions
        .as_mut()
        .unwrap()
        .set_tips_eligible(Answer::NotSure);
    unsupported(&d, "Income");
}
#[test]
fn work_deductions_share_joint_caps_and_require_claimant_ssn() {
    let mut d = complete(FederalStatus::Joint);
    d.income[0] = wage(1, Owner::Taxpayer, 8_000_000, 0);
    let id = d.next_id;
    add(&mut d, wage(id, Owner::Spouse, 8_000_000, 0));
    for entry in &mut d.income {
        let FederalIncome::Wage { data: w } = &mut entry.income else {
            panic!()
        };
        w.work_deductions = (Some(work_deductions(2_000_000, 2_000_000))).into();
    }
    let r = supported(&d);
    assert_eq!(value(&r, "tips_deduction"), 25000);
    assert_eq!(value(&r, "overtime_deduction"), 25000);
    d.people
        .as_mut()
        .unwrap()
        .spouse
        .as_mut()
        .unwrap()
        .valid_ssn = Answer::No;
    let r = supported(&d);
    assert_eq!(value(&r, "tips_deduction"), 20000);
    assert_eq!(value(&r, "overtime_deduction"), 20000);
}
#[test]
fn vehicle_interest_aggregates_loans_and_does_not_reduce_agi() {
    let mut d = complete(FederalStatus::Single);
    d.deductions.as_mut().unwrap().vehicle_loans = vec![
        FederalVehicleInterest {
            label: "Synthetic car".into(),
            interest: 600_000,
            eligible: Answer::Yes,
        },
        FederalVehicleInterest {
            label: "Synthetic van".into(),
            interest: 600_000,
            eligible: Answer::Yes,
        },
    ];
    let r = supported(&d);
    assert_eq!(value(&r, "vehicle_interest_deduction"), 10000);
    assert_eq!(value(&r, "agi"), 60000);
    assert_eq!(r.tax, Some(382_000));
    d.deductions.as_mut().unwrap().vehicle_loans[0].eligible = Answer::No;
    unsupported(&d, "Vehicle");
}
#[test]
fn educator_excess_is_itemized_once_and_only_when_itemizing() {
    let mut d = complete(FederalStatus::Single);
    let a = d.adjustments.as_mut().unwrap();
    a.set_educator_taxpayer(100_000);
    a.set_eligible_educators(Answer::Yes);
    let r = supported(&d);
    assert_eq!(value(&r, "adjustments"), 350);
    assert_eq!(value(&r, "deduction"), 16100);
    let v = d.deductions.as_mut().unwrap();
    itemized(v).state_local_taxes = 1_000_000;
    itemized(v).set_mortgage_interest(1_000_000);
    itemized(v).set_mortgage_within_limit(Answer::Yes);
    let r = supported(&d);
    assert_eq!(value(&r, "deduction"), 20650); // 20,000 plus only the remaining 650.
    assert_eq!(value(&r, "agi"), 59650);
    d.deductions.as_mut().unwrap().choice = DeductionSelection::Standard;
    assert_eq!(value(&supported(&d), "deduction"), 16100);
}
#[test]
fn additional_deduction_facts_survive_restart_and_invalid_amounts_are_rejected() {
    let mut d = complete(FederalStatus::Single);
    let FederalIncome::Wage { data: w } = &mut d.income[0].income else {
        panic!()
    };
    w.work_deductions = (Some(work_deductions(0, 500_000))).into();
    d.deductions.as_mut().unwrap().vehicle_loans = vec![FederalVehicleInterest {
        label: "Synthetic car".into(),
        interest: 100_000,
        eligible: Answer::Yes,
    }];
    assert_eq!(supported(&restart(d.clone()).draft), supported(&d));
    let FederalIncome::Wage { data: w } = &mut d.income[0].income else {
        panic!()
    };
    w.work_deductions.as_mut().unwrap().set_overtime(-1);
    assert!(!d.valid_stored());
}

#[test]
fn incomplete_spousal_ira_facts_cannot_panic_during_roth_validation() {
    let mut d = complete(FederalStatus::Joint);
    d.income[0] = wage(1, Owner::Spouse, 10_000_000, 0);
    d.adjustments.as_mut().unwrap().iras = (Some(ira_facts(0, 100_000, Answer::No))).into();
    unsupported(&d, "IRA");
    d.adjustments
        .as_mut()
        .unwrap()
        .iras
        .as_mut()
        .unwrap()
        .spouse = IraSpouse::Joint {
        facts: FederalIraContribution {
            traditional: 0,
            roth: 0,
            covered_at_work: Answer::No,
        },
    };
    d.people.as_mut().unwrap().spouse = SpouseFiler::No;
    unsupported(&d, "IRA");
}

#[test]
fn itemized_high_income_screen_adds_back_deduction_when_testing_section_68() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 65_000_000, 0);
    let v = d.deductions.as_mut().unwrap();
    v.choice = DeductionSelection::Itemized {
        facts: itemized(v).clone(),
    };
    itemized(v).set_mortgage_interest(3_000_000);
    itemized(v).set_mortgage_within_limit(Answer::Yes);
    // Taxable income falls below 640,600, but the section 68 trigger adds itemized
    // deductions back. A result without this worksheet would overstate the deduction.
    unsupported(&d, "Deductions");
}

#[test]
fn ira_overtime_vehicle_and_education_cross_topic_fixture() {
    let mut d = complete(FederalStatus::Single);
    d.income[0] = wage(1, Owner::Taxpayer, 8_000_000, 650_000);
    let FederalIncome::Wage { data: w } = &mut d.income[0].income else {
        panic!()
    };
    w.work_deductions = (Some(work_deductions(0, 500_000))).into();
    d.adjustments.as_mut().unwrap().iras = (Some(ira_facts(750_000, 0, Answer::Yes))).into();
    d.deductions.as_mut().unwrap().vehicle_loans = vec![FederalVehicleInterest {
        label: "Synthetic vehicle".into(),
        interest: 200_000,
        eligible: Answer::Yes,
    }];
    d.credits.as_mut().unwrap().education = vec![FederalEducation {
        student: 0,
        expenses: education_expenses(400_000),
        eligible_student: Answer::Yes,
        method: education_choice(EducationMethod::AmericanOpportunity, Answer::Yes),
    }];
    let r = supported(&d);
    assert_eq!(value(&r, "agi"), 72500);
    assert_eq!(value(&r, "taxable_income"), 49400);
    assert_eq!(value(&r, "income_tax"), 5680);
    assert_eq!(value(&r, "education_credit"), 1500);
    assert_eq!(r.tax, Some(418_000));
    assert_eq!(r.refund, Some(332_000));
    assert_eq!(supported(&restart(d).draft), r);
}

#[test]
fn setup_unions_round_trip_every_applicable_branch() {
    let situations = [
        MaritalState::Unmarried,
        MaritalState::Married {
            living: SpouseLiving::Together,
        },
        MaritalState::Married {
            living: SpouseLiving::LastSixMonths,
        },
        MaritalState::Married {
            living: SpouseLiving::AllYear,
        },
        MaritalState::Widowed2026,
        MaritalState::Widowed2025 {
            could_file_joint: Answer::Yes,
        },
        MaritalState::Widowed2024 {
            could_file_joint: Answer::NotSure,
        },
    ];
    for marital_state in situations {
        for status in [
            FilingChoice::Single,
            FilingChoice::Joint,
            FilingChoice::Separate {
                spouse_itemizes: false,
            },
            FilingChoice::Separate {
                spouse_itemizes: true,
            },
            FilingChoice::Head,
            FilingChoice::Surviving,
        ] {
            let mut draft = complete(FederalStatus::Single);
            let setup = draft.setup.as_mut().unwrap();
            setup.marital_state = marital_state;
            setup.status = status;
            assert_eq!(restart(draft.clone()).draft, draft);
        }
    }
}

#[test]
fn spouse_living_choice_preserves_the_two_tax_period_tests() {
    let mut setup = complete(FederalStatus::Separate).setup.unwrap();
    for (living, all_year, last_six) in [
        (SpouseLiving::Together, false, false),
        (SpouseLiving::LastSixMonths, false, true),
        (SpouseLiving::AllYear, true, true),
    ] {
        setup.marital_state = MaritalState::Married { living };
        assert_eq!(setup.lived_apart_all_year(), all_year);
        assert_eq!(setup.lived_apart_last_six_months(), last_six);
    }
    setup.marital_state = MaritalState::Unmarried;
    assert!(!setup.lived_apart_all_year());
    assert!(!setup.lived_apart_last_six_months());
}

#[test]
fn separate_filing_branch_owns_spouse_itemization() {
    let mut draft = complete(FederalStatus::Separate);
    draft.setup.as_mut().unwrap().status = FilingChoice::Separate {
        spouse_itemizes: true,
    };
    draft.deductions.as_mut().unwrap().choice = DeductionSelection::Standard;
    unsupported(&draft, "Deductions");
    draft.setup.as_mut().unwrap().status = FilingChoice::Separate {
        spouse_itemizes: false,
    };
    assert!(!draft.setup.as_ref().unwrap().spouse_itemizes());
    supported(&draft);
}

fn education_choice(method: EducationMethod, requirements: Answer) -> EducationChoice {
    match method {
        EducationMethod::AmericanOpportunity => {
            EducationChoice::AmericanOpportunity { requirements }
        }
        EducationMethod::LifetimeLearning => EducationChoice::LifetimeLearning,
    }
}
fn distribution_period(
    year: SaverDistributionYear,
    joint_with_current_spouse: Answer,
) -> SaverDistributionPeriod {
    match year {
        SaverDistributionYear::PriorTwo => SaverDistributionPeriod::PriorTwo {
            joint_with_current_spouse,
        },
        SaverDistributionYear::PriorOne => SaverDistributionPeriod::PriorOne {
            joint_with_current_spouse,
        },
        SaverDistributionYear::TaxYear => SaverDistributionPeriod::TaxYear,
        SaverDistributionYear::FollowingYear => SaverDistributionPeriod::FollowingYear {
            joint_with_current_spouse,
        },
    }
}

// Tests mutate expense fixtures explicitly; production editors construct the selected branch.
fn itemized(d: &mut FederalDeductions) -> &mut ItemizedExpenses {
    if matches!(
        d.choice,
        DeductionSelection::Standard
            | DeductionSelection::Automatic {
                expenses: ItemizedChoice::No
            }
    ) {
        d.choice = DeductionSelection::Automatic {
            expenses: ItemizedChoice::Yes {
                facts: ItemizedExpenses {
                    medical: 0,
                    state_local_taxes: 0,
                    mortgage: MortgageClaim::No,
                },
            },
        };
    }
    match &mut d.choice {
        DeductionSelection::Automatic {
            expenses: ItemizedChoice::Yes { facts },
        }
        | DeductionSelection::Itemized { facts } => facts,
        _ => unreachable!(),
    }
}

#[test]
fn conditional_union_branches_round_trip_exact_values_and_unknown_answers() {
    macro_rules! round_trip {
        ($ty:ty, $($value:expr),+ $(,)?) => { $( {
            let value: $ty = $value;
            assert_eq!(<$ty>::from_avro(&value.to_avro()).unwrap(), value);
        } )+ };
    }
    round_trip!(
        DependentCare,
        DependentCare::No,
        DependentCare::Yes {
            care_expenses: 12345,
            care_period_eligible: Answer::NotSure
        }
    );
    round_trip!(
        TipsClaim,
        TipsClaim::No,
        TipsClaim::Yes {
            tips: 12345,
            tips_eligible: Answer::NotSure
        }
    );
    round_trip!(
        OvertimeClaim,
        OvertimeClaim::No,
        OvertimeClaim::Yes {
            overtime: 12345,
            overtime_eligible: Answer::NotSure
        }
    );
    round_trip!(
        StudentLoanClaim,
        StudentLoanClaim::No,
        StudentLoanClaim::Yes {
            student_loan_interest: 12345,
            student_loan_eligible: Answer::NotSure
        }
    );
    round_trip!(
        EducatorClaim,
        EducatorClaim::No,
        EducatorClaim::Yes {
            educator_taxpayer: 12345,
            educator_spouse: SpouseAmount::No,
            eligible_educators: Answer::NotSure
        },
        EducatorClaim::Yes {
            educator_taxpayer: 12345,
            educator_spouse: SpouseAmount::Yes { amount: 6789 },
            eligible_educators: Answer::Yes
        }
    );
    round_trip!(
        MortgageClaim,
        MortgageClaim::No,
        MortgageClaim::Yes {
            mortgage_interest: 12345,
            mortgage_within_limit: Answer::NotSure
        }
    );
    round_trip!(
        CharityClaim,
        CharityClaim::No,
        CharityClaim::Yes {
            cash_charity: 12345,
            cash_charity_eligible: Answer::NotSure
        }
    );
    round_trip!(
        CareClaim,
        CareClaim::No,
        CareClaim::Yes {
            care_expenses: SpouseAmount::No,
            care_eligible: Answer::NotSure,
            care_benefits: 12345
        },
        CareClaim::Yes {
            care_expenses: SpouseAmount::Yes { amount: 6789 },
            care_eligible: Answer::Yes,
            care_benefits: 0
        }
    );
    round_trip!(
        EducationChoice,
        EducationChoice::LifetimeLearning,
        EducationChoice::AmericanOpportunity {
            requirements: Answer::NotSure
        }
    );
    round_trip!(
        SaverDistributionPeriod,
        SaverDistributionPeriod::TaxYear,
        SaverDistributionPeriod::PriorTwo {
            joint_with_current_spouse: Answer::NotSure
        },
        SaverDistributionPeriod::PriorOne {
            joint_with_current_spouse: Answer::Yes
        },
        SaverDistributionPeriod::FollowingYear {
            joint_with_current_spouse: Answer::No
        }
    );
    round_trip!(
        SpouseFiler,
        SpouseFiler::No,
        SpouseFiler::Yes { facts: person() }
    );
    round_trip!(
        WorkDeductionChoice,
        WorkDeductionChoice::No,
        WorkDeductionChoice::Yes {
            facts: FederalWorkDeductions {
                tips_claim: TipsClaim::No,
                overtime_claim: OvertimeClaim::No
            }
        }
    );
    round_trip!(
        IraChoice,
        IraChoice::No,
        IraChoice::Yes {
            facts: ira_facts(12345, 0, Answer::NotSure)
        }
    );
    round_trip!(
        IraSpouse,
        IraSpouse::NotApplicable,
        IraSpouse::Joint {
            facts: FederalIraContribution {
                traditional: 12345,
                roth: 6789,
                covered_at_work: Answer::NotSure
            }
        },
        IraSpouse::SeparateTogether {
            covered_at_work: Answer::NotSure
        }
    );
    let expenses = ItemizedExpenses {
        medical: 12345,
        state_local_taxes: 6789,
        mortgage: MortgageClaim::No,
    };
    round_trip!(
        DeductionSelection,
        DeductionSelection::Standard,
        DeductionSelection::Automatic {
            expenses: ItemizedChoice::No
        },
        DeductionSelection::Automatic {
            expenses: ItemizedChoice::Yes {
                facts: expenses.clone()
            }
        },
        DeductionSelection::Itemized { facts: expenses }
    );
    round_trip!(
        SaverChoice,
        SaverChoice::No,
        SaverChoice::Yes {
            facts: saver(12345, 6789)
        }
    );
    round_trip!(
        DependentResidency,
        DependentResidency::WholeYearException,
        DependentResidency::Days { days: 183 }
    );
    round_trip!(
        SpouseAmount,
        SpouseAmount::No,
        SpouseAmount::Yes { amount: 0 },
        SpouseAmount::Yes { amount: 12345 }
    );
}

#[test]
fn declining_conditional_claim_removes_its_facts_without_hiding_unknown_eligibility() {
    let mut draft = complete(FederalStatus::Single);
    draft.adjustments.as_mut().unwrap().student_loan = StudentLoanClaim::Yes {
        student_loan_interest: 12345,
        student_loan_eligible: Answer::NotSure,
    };
    unsupported(&draft, "Student");
    draft.adjustments.as_mut().unwrap().student_loan = StudentLoanClaim::No;
    assert_eq!(value(&supported(&draft), "adjustments"), 0);
    let state = restart(draft);
    assert!(matches!(
        state.draft.adjustments.unwrap().student_loan,
        StudentLoanClaim::No
    ));
}

#[test]
fn ira_spouse_branch_must_match_the_return_context() {
    let draft = complete(FederalStatus::Single);
    let mut facts = ira_facts(10000, 0, Answer::No);
    facts.spouse = IraSpouse::SeparateTogether {
        covered_at_work: Answer::Yes,
    };
    assert!(
        crate::ira::calculate(
            &facts,
            draft.people.as_ref().unwrap(),
            draft.setup.as_ref().unwrap(),
            [50000, 0],
            50000
        )
        .is_err()
    );
    facts.spouse = IraSpouse::NotApplicable;
    assert!(
        crate::ira::calculate(
            &facts,
            draft.people.as_ref().unwrap(),
            draft.setup.as_ref().unwrap(),
            [50000, 0],
            50000
        )
        .is_ok()
    );
}

#[test]
fn unchanged_history_next_preserves_every_saved_page_and_completion() {
    let mut original = complete(FederalStatus::Single);
    original.finished = true;
    let mut session = Interview::new(original.clone());
    let reply = send(
        &mut session,
        P::SavePeople {
            cookie: original.cookie.clone(),
            revision: original.revision,
            allowed_spouse: vec![0],
            page: 1,
            previous: 0,
            review: false,
            progress: vec![],
            value: original.people.clone().unwrap(),
        },
    );
    assert!(matches!(reply, Reply::Edit(P::SaveDependents { .. })));
    assert_eq!(session.draft, original);
    assert!(!session.dirty);
    assert_eq!(restart(session.draft).draft, original);
}

#[test]
fn changed_history_replaces_page_and_erases_downstream_atomically() {
    let original = complete(FederalStatus::Single);
    let mut session = Interview::new(original.clone());
    let mut people = original.people.clone().unwrap();
    people.taxpayer.name = "Another Synthetic Filer".into();
    send(
        &mut session,
        P::SavePeople {
            cookie: original.cookie.clone(),
            revision: original.revision,
            allowed_spouse: vec![0],
            page: 1,
            previous: 0,
            review: false,
            progress: vec![],
            value: people.clone(),
        },
    );
    let saved = &session.draft;
    assert_eq!(saved.setup, original.setup);
    assert_eq!(saved.people, Some(people));
    assert!(saved.dependents.is_empty() && !saved.dependents_complete);
    assert!(saved.income.is_empty() && !saved.income_complete);
    assert!(saved.adjustments.is_none() && saved.deductions.is_none());
    assert!(saved.credits.is_none() && saved.screening.is_none() && saved.payments.is_none());
    assert!(!saved.finished);
    assert_eq!(saved.next_id, original.next_id);
    assert_eq!(saved.revision, original.revision + 1);
    assert!(session.dirty);
    let mut restored = restart(session.draft.clone());
    assert!(matches!(
        send(&mut restored, P::OpenInterview),
        Reply::Edit(P::SaveDependents { .. })
    ));
    error(send(
        &mut session,
        P::SavePeople {
            cookie: original.cookie,
            revision: original.revision,
            allowed_spouse: vec![0],
            page: 1,
            previous: 0,
            review: false,
            progress: vec![],
            value: original.people.unwrap(),
        },
    ));
    assert_eq!(session.draft, restored.draft);
}

#[test]
fn rejected_history_edit_preserves_downstream_answers() {
    let original = complete(FederalStatus::Single);
    let mut session = Interview::new(original.clone());
    let mut people = original.people.clone().unwrap();
    people.taxpayer.name.clear();
    error(send(
        &mut session,
        P::SavePeople {
            cookie: original.cookie.clone(),
            revision: original.revision,
            allowed_spouse: vec![0],
            page: 1,
            previous: 0,
            review: false,
            progress: vec![],
            value: people,
        },
    ));
    assert_eq!(session.draft, original);
    assert!(!session.dirty);
}

#[test]
fn erasure_boundaries_cover_all_nine_sections() {
    for page in 0..9 {
        let mut draft = complete(FederalStatus::Single);
        draft.erase_after(page);
        assert_eq!(draft.people.is_none(), page < 1);
        assert_eq!(!draft.dependents_complete, page < 2);
        assert_eq!(!draft.income_complete, page < 3);
        assert_eq!(draft.adjustments.is_none(), page < 4);
        assert_eq!(draft.deductions.is_none(), page < 5);
        assert_eq!(draft.credits.is_none(), page < 6);
        assert_eq!(draft.screening.is_none(), page < 7);
        assert_eq!(draft.payments.is_none(), page < 8);
        assert!(draft.valid_stored());
    }
}

#[test]
fn landing_actions_follow_accepted_state_without_mutation() {
    let mut interview =
        crate::interview::Interview::new(crate::FederalDraft::empty("landing".into()));
    let Some(crate::interview::Reply::Response(P::Actions { allowed_actions })) =
        interview.handle(P::Enter)
    else {
        panic!("actions expected")
    };
    assert_eq!(
        allowed_actions,
        alloc::vec![libertas_macros::variant_index!(P::OpenInterview) as i32]
    );
    assert!(!interview.dirty);
    assert!(matches!(
        interview.handle(P::ReviewAccepted),
        Some(crate::interview::Reply::Response(P::Problem { .. }))
    ));
}

#[test]
fn saved_landing_offers_review_and_keeps_revision() {
    let draft = complete(FederalStatus::Single);
    let revision = draft.revision;
    let mut interview = Interview::new(draft);
    let Some(Reply::Response(P::Actions { allowed_actions })) = interview.handle(P::Enter) else {
        panic!("actions expected")
    };
    assert_eq!(
        allowed_actions,
        vec![
            libertas_macros::variant_index!(P::OpenInterview) as i32,
            libertas_macros::variant_index!(P::ReviewAccepted) as i32
        ]
    );
    assert!(matches!(
        interview.handle(P::ReviewAccepted),
        Some(Reply::Response(P::Review { .. }))
    ));
    assert_eq!(interview.draft.revision, revision);
    assert!(!interview.dirty);
}

#[test]
fn section_choices_match_backend_admission_at_every_saved_stage() {
    let sections = [
        TaxSection::Setup,
        TaxSection::People,
        TaxSection::Children,
        TaxSection::Income,
        TaxSection::Adjustments,
        TaxSection::Deductions,
        TaxSection::Credits,
        TaxSection::Screening,
        TaxSection::Payments,
        TaxSection::Review,
    ];
    for saved_page in 0..=8 {
        let mut draft = complete(FederalStatus::Single);
        draft.erase_after(saved_page);
        let mut interview = Interview::new(draft.clone());
        let Reply::Response(P::Review {
            allowed_sections, ..
        }) = send(&mut interview, P::ReviewAccepted)
        else {
            panic!("review expected")
        };
        let expected: alloc::vec::Vec<i32> = (0..=(saved_page + 1).min(8))
            .chain(core::iter::once(9))
            .collect();
        assert_eq!(allowed_sections, expected);
        for section in sections {
            let response = send(
                &mut interview,
                P::Navigate {
                    cookie: draft.cookie.clone(),
                    revision: draft.revision,
                    section,
                    // A fabricated client list cannot unlock a future section.
                    allowed_sections: (0..=9).collect(),
                },
            );
            assert_eq!(
                !matches!(response, Reply::Response(P::Problem { .. })),
                allowed_sections.contains(&(section as i32))
            );
        }
        assert_eq!(interview.draft, draft);
        assert!(!interview.dirty);
    }
}

#[test]
fn dependent_array_saves_atomically_and_unchanged_review_preserves_history() {
    let original = complete(FederalStatus::Single);
    let mut session = Interview::new(original.clone());
    let request = |d: &FederalDraft, records| P::SaveDependents {
        cookie: d.cookie.clone(),
        revision: d.revision,
        page: 2,
        previous: 1,
        review: true,
        progress: vec![],
        records,
    };
    send(
        &mut session,
        request(&original, original.dependents.clone()),
    );
    assert_eq!(session.draft, original);
    assert!(!session.dirty);
    let mut rows = original.dependents.clone();
    rows.push(child(0));
    send(&mut session, request(&original, rows));
    assert!(session.dirty);
    assert!(session.draft.dependents_complete);
    assert_eq!(
        session.draft.dependents.last().unwrap().id,
        original.next_id
    );
    assert!(session.draft.income.is_empty());
    assert!(!session.draft.income_complete);
    let accepted = session.draft.clone();
    let mut session = restart(accepted.clone());
    let mut invalid = accepted.dependents.clone();
    invalid.push(invalid[0].clone());
    error(send(&mut session, request(&accepted, invalid)));
    assert_eq!(session.draft, accepted);
    assert!(!session.dirty);
    send(&mut session, request(&accepted, vec![]));
    assert!(session.draft.dependents.is_empty());
    assert!(session.draft.dependents_complete);
}

fn education_expenses(tuition: i64) -> FederalEducationExpenses {
    FederalEducationExpenses {
        tuition,
        school_materials: 0,
        other_materials: 0,
        assistance: 0,
        refunds: 0,
        other_benefits: 0,
    }
}

#[test]
fn business_expenses_are_totaled_once_and_preview_matches_return() {
    let business = FederalBusiness {
        receipts: 6_000_000,
        expenses: vec![
            FederalBusinessExpense {
                label: "Supplies".into(),
                amount: 600_000,
            },
            FederalBusinessExpense {
                label: "Advertising".into(),
                amount: 400_000,
            },
        ],
        material_participation: Answer::Yes,
        special_treatment: Answer::No,
    };
    assert_eq!(business.expense_total(), 1_000_000);
    let mut draft = complete(FederalStatus::Single);
    draft.income.clear();
    add(
        &mut draft,
        income(FederalIncome::Business {
            data: business.clone(),
        }),
    );
    let result = supported(&draft);
    assert_eq!(value(&result, "agi"), 46_467);
    let mut invalid = business;
    invalid.expenses[0].label.clear();
    assert!(
        crate::federal_rules::validate_income_amount(&FederalIncome::Business { data: invalid })
            .is_err()
    );
}

#[test]
fn education_expenses_derive_the_selected_credit_base() {
    let mut expenses = education_expenses(300_000);
    expenses.school_materials = 50_000;
    expenses.other_materials = 100_000;
    expenses.assistance = 40_000;
    expenses.refunds = 20_000;
    expenses.other_benefits = 30_000;
    assert_eq!(
        expenses.qualified(EducationMethod::AmericanOpportunity),
        360_000
    );
    assert_eq!(
        expenses.qualified(EducationMethod::LifetimeLearning),
        260_000
    );
    let mut draft = complete(FederalStatus::Single);
    draft
        .credits
        .as_mut()
        .unwrap()
        .education
        .push(FederalEducation {
            student: 0,
            expenses: expenses.clone(),
            eligible_student: Answer::Yes,
            method: education_choice(EducationMethod::AmericanOpportunity, Answer::Yes),
        });
    let result = supported(&draft);
    assert_eq!(
        value(&result, "education_credit") + value(&result, "refundable_education_credit"),
        2400
    );
    expenses.assistance = 1_000_000;
    assert_eq!(expenses.qualified(EducationMethod::AmericanOpportunity), 0);
}

#[test]
fn proposed_refund_rules_are_advisory_and_do_not_reduce_the_refund() {
    let mut d = complete(FederalStatus::Single);
    d.income.clear();
    // No earnings and no refundable credits: no proposed-credit review flag.
    let ordinary = supported(&d);
    assert!(ordinary.review_notes.is_empty());
    // Low earnings produce a childless EIC exceeding the calculated tax.
    d.income.push(wage(1, Owner::Taxpayer, 1_000_000, 0));
    let r = supported(&d);
    assert_eq!(
        r.refund,
        Some((r.payments.unwrap() + r.refundable_credits.unwrap() - r.tax.unwrap()).max(0))
    );
    assert!(r.refundable_credits.unwrap() > r.tax.unwrap());
    assert_eq!(r.review_notes.len(), 1);
    assert!(r.issues.is_empty());
}

fn return_line(result: &FederalResult, reference: &str) -> i64 {
    let prefix = alloc::format!("1040 {reference} — ");
    result
        .return_lines
        .iter()
        .find(|line| line.label.starts_with(&prefix))
        .unwrap()
        .amount
}

#[test]
fn calculated_return_lines_reconcile_to_the_same_refund() {
    for status in [FederalStatus::Single, FederalStatus::Joint] {
        let result = supported(&complete(status));
        let l = |reference| return_line(&result, reference);
        assert_eq!(
            l("9"),
            ["1a / 1z", "2b", "3b", "4b", "5b", "6b", "7a", "8"]
                .into_iter()
                .map(l)
                .sum::<i64>()
        );
        assert_eq!(l("11a / 11b"), l("9") - l("10"));
        assert_eq!(
            l("14"),
            ["12e", "12f", "13a", "13b"].into_iter().map(l).sum::<i64>()
        );
        assert_eq!(l("15"), (l("11a / 11b") - l("14")).max(0));
        assert_eq!(l("24a / 24c"), l("22") + l("23"));
        assert_eq!(l("24a / 24c"), result.tax.unwrap());
        assert_eq!(
            l("33"),
            l("25d") + l("26") + l("27a") + l("28") + l("29") + l("31")
        );
        assert_eq!(l("34"), result.refund.unwrap());
        assert_eq!(l("37"), result.balance.unwrap());
    }
}

#[test]
fn ira_and_pension_round_at_their_own_return_lines() {
    let mut draft = complete(FederalStatus::Single);
    let initial_agi = value(&supported(&draft), "agi");
    for kind in [RetirementKind::Ira, RetirementKind::Pension] {
        add(
            &mut draft,
            income(FederalIncome::Retirement {
                data: FederalRetirement {
                    kind,
                    gross: 100_050,
                    taxable: 100_050,
                    withholding: 0,
                    ordinary_distribution: Answer::Yes,
                },
            }),
        );
    }
    let result = supported(&draft);
    assert_eq!(return_line(&result, "4b"), 100_100);
    assert_eq!(return_line(&result, "5b"), 100_100);
    assert_eq!(value(&result, "agi"), initial_agi + 2002);
}

#[test]
fn incomplete_and_unsupported_returns_never_show_completed_return_lines() {
    let incomplete = crate::federal_rules::estimate(&FederalDraft::empty("draft".into()));
    assert!(incomplete.return_lines.is_empty() && incomplete.review_notes.is_empty());
    let mut draft = complete(FederalStatus::Single);
    draft.screening.as_mut().unwrap().uncertain = Answer::Yes;
    let unsupported = crate::federal_rules::estimate(&draft);
    assert!(unsupported.return_lines.is_empty() && unsupported.refund.is_none());
}

#[test]
fn ira_spouse_choices_follow_accepted_setup_for_new_and_saved_adjustments() {
    for (status, apart, expected) in [
        (FederalStatus::Single, false, 0),
        (FederalStatus::Head, false, 0),
        (FederalStatus::Surviving, false, 0),
        (FederalStatus::Joint, false, 1),
        (FederalStatus::Separate, false, 2),
        (FederalStatus::Separate, true, 0),
    ] {
        for saved in [false, true] {
            let mut draft = complete(status);
            if apart {
                draft.setup.as_mut().unwrap().marital_state = MaritalState::Married {
                    living: SpouseLiving::AllYear,
                };
            }
            if !saved {
                draft.erase_after(3);
            }
            let mut interview = Interview::new(draft.clone());
            let reply = send(
                &mut interview,
                P::Navigate {
                    cookie: draft.cookie.clone(),
                    revision: draft.revision,
                    section: TaxSection::Adjustments,
                    allowed_sections: alloc::vec![4],
                },
            );
            let allowed = match reply {
                Reply::Response(P::BeginAdjustments {
                    allowed_ira_spouse, ..
                }) if !saved => allowed_ira_spouse,
                Reply::Edit(P::SaveAdjustments {
                    allowed_ira_spouse, ..
                }) if saved => allowed_ira_spouse,
                _ => panic!("expected adjustments editor"),
            };
            assert_eq!(allowed, alloc::vec![expected]);
            assert_eq!(interview.draft, draft);
        }
    }
}

#[test]
fn deductions_review_edit_resumes_invalidated_pages_instead_of_review() {
    for changed in [false, true] {
        let original = complete(FederalStatus::Single);
        let mut session = Interview::new(original.clone());
        let mut value = original.deductions.clone().unwrap();
        if changed {
            value.set_cash_charity(10_000);
            value.set_cash_charity_eligible(Answer::Yes);
        }
        let reply = send(
            &mut session,
            P::SaveDeductions {
                cookie: original.cookie.clone(),
                revision: original.revision,
                page: 5,
                previous: 4,
                review: true,
                progress: vec![],
                value: value.clone(),
            },
        );
        if changed {
            assert!(matches!(
                reply,
                Reply::Response(P::BeginCredits { review: false, .. })
            ));
            assert_eq!(session.draft.deductions, Some(value));
            assert!(session.draft.credits.is_none());
            assert!(session.draft.screening.is_none());
            assert!(session.draft.payments.is_none());
            assert_eq!(session.draft.first_incomplete(), 6);
            assert_eq!(session.draft.revision, original.revision + 1);
            let mut restored = restart(session.draft.clone());
            assert!(matches!(
                send(&mut restored, P::OpenInterview),
                Reply::Response(P::BeginCredits { .. })
            ));
        } else {
            assert!(matches!(reply, Reply::Response(P::Review { .. })));
            assert_eq!(session.draft, original);
            assert!(!session.dirty);
        }
    }
}
