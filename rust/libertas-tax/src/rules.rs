// US-2026-r1 sources verified 2026-09-13. These rules estimate the bounded scope;
// final Form 1040 tax tables, penalties and unsupported worksheets are not implemented.
// Rates, deductions, EITC and AMT: https://www.irs.gov/pub/irs-drop/rp-25-32.pdf
// 2026 estimated tax and additional taxes: https://www.irs.gov/publications/p505
// Cash gifts: IRC 170(p), https://www.irs.gov/taxtopics/tc506
// Social Security base: https://www.ssa.gov/oact/COLA/cbb.html
use crate::*;
use alloc::{string::String, vec::Vec};

pub(crate) const MAX_AMOUNT: i64 = 100_000_000_000;
pub(crate) const TOPICS: [CoverageTopicV2; 12] = [
    CoverageTopicV2::Dependents,
    CoverageTopicV2::Retirement,
    CoverageTopicV2::Investments,
    CoverageTopicV2::Business,
    CoverageTopicV2::Foreign,
    CoverageTopicV2::Marketplace,
    CoverageTopicV2::Education,
    CoverageTopicV2::Itemizing,
    CoverageTopicV2::NewDeductions,
    CoverageTopicV2::HealthSavings,
    CoverageTopicV2::Employment,
    CoverageTopicV2::Other,
];
pub(crate) const QUESTIONS: [&str; 12] = [
    "Do you have children or other dependents, or need a filing status other than Single or Married Filing Jointly?",
    "Did you receive retirement or Social Security income, or make retirement contributions that may qualify for a savings credit?",
    "Did you receive dividends, sell investments or digital assets, receive tax-exempt interest, or have investment income other than ordinary taxable bank interest?",
    "Did you have self-employment, rental, unemployment, gambling, partnership, trust, or other income besides ordinary W-2 wages and taxable bank interest?",
    "Did you have foreign income or accounts, foreign tax, a nonresident period, or a treaty or territorial tax situation?",
    "Did anyone on this return have Marketplace health insurance, including advance premium credits?",
    "Did you have education expenses, student-loan interest, adoption expenses, or dependent-care expenses?",
    "Might itemizing mortgage interest, state/local taxes, medical expenses, charity or disaster losses benefit you, or do you have a charitable carryover? This prototype does not compare itemizing with the standard deduction.",
    "Did you have qualified tips, qualified overtime, eligible new vehicle-loan interest, or a senior deduction?",
    "Did you have HSA or IRA activity, educator expenses, alimony paid under an eligible older agreement, military moving expenses, or another adjustment to income?",
    "Did you have unusual W-2 boxes, unreported tips, railroad retirement, household employment taxes, excess payroll withholding, or additional Medicare withholding? Include stock compensation, dependent-care/adoption benefits, and retirement savings credit eligibility.",
    "Do any other federal credits, carryovers, special taxes, separate tax calculations, prior-year corrections, or uncertain tax situations apply? Include a child's investment tax and any need for a special tax worksheet.",
];

impl DraftV2 {
    pub(crate) fn empty(cookie: String) -> Self {
        Self {
            tax_year: 2026,
            cookie,
            revision: 0,
            next_id: 1,
            setup: None,
            taxpayer: None,
            spouse: None,
            coverage: Vec::new(),
            records: Vec::new(),
            income_complete: false,
            charity: None,
            charity_qualified: None,
            charity_amount: None,
            payments: None,
            finished: false,
        }
    }
    pub(crate) fn joint(&self) -> bool {
        self.setup
            .as_ref()
            .is_some_and(|s| s.filing_status == FilingStatusV2::Joint)
    }
    pub(crate) fn first_incomplete(&self) -> i32 {
        if self.setup.is_none() {
            return 0;
        }
        if self.taxpayer.is_none() {
            return 1;
        }
        if self.joint() && self.spouse.is_none() {
            return 2;
        }
        for (n, topic) in TOPICS.iter().enumerate() {
            if !self.coverage.iter().any(|a| &a.topic == topic) {
                return n as i32 + 3;
            }
        }
        if !self.income_complete {
            return 15;
        }
        if self.charity.is_none() {
            return 16;
        }
        if self.charity == Some(AnswerV2::Yes) {
            if self.charity_qualified.is_none() {
                return 17;
            }
            if self.charity_qualified == Some(AnswerV2::Yes) && self.charity_amount.is_none() {
                return 18;
            }
        }
        if self.payments.is_none() {
            return 19;
        }
        20
    }
    pub(crate) fn valid_stored(&self) -> bool {
        let amount = |v: i64| (0..=MAX_AMOUNT).contains(&v);
        // Values may survive applicability changes, but never predate creation of the return.
        (self.setup.is_some()
            || (self.taxpayer.is_none()
                && self.spouse.is_none()
                && self.coverage.is_empty()
                && self.records.is_empty()
                && !self.income_complete
                && self.charity.is_none()
                && self.payments.is_none()))
            && (self.charity == Some(AnswerV2::Yes)
                || (self.charity_qualified.is_none() && self.charity_amount.is_none()))
            && (self.charity_qualified == Some(AnswerV2::Yes) || self.charity_amount.is_none())
            && self.tax_year == 2026
            && !self.cookie.is_empty()
            && self.cookie.len() <= 128
            && self.revision >= 0
            && self.revision < i64::MAX
            && self.next_id > 0
            && self.next_id < i64::MAX
            && self.records.len() <= 100
            && self.coverage.len() <= TOPICS.len()
            && self
                .setup
                .as_ref()
                .is_none_or(|s| !s.label.trim().is_empty() && s.label.len() <= 80)
            && self.records.iter().enumerate().all(|(n, r)| {
                r.id > 0
                    && r.id < self.next_id
                    && !r.payer.trim().is_empty()
                    && r.payer.len() <= 80
                    && amount(r.amount)
                    && amount(r.withholding)
                    && amount(r.medicare_wages)
                    && amount(r.social_security_withheld)
                    && (r.kind == IncomeKindV2::Wage
                        || (r.medicare_wages == 0 && r.social_security_withheld == 0))
                    && self.records[..n].iter().all(|other| other.id != r.id)
            })
            && self
                .coverage
                .iter()
                .enumerate()
                .all(|(n, a)| self.coverage[..n].iter().all(|b| b.topic != a.topic))
            && self.charity_amount.is_none_or(amount)
            && self
                .payments
                .as_ref()
                .is_none_or(|v| amount(v.estimated) && amount(v.extension))
            && (!self.finished || self.first_incomplete() == 20)
    }
}

pub(crate) fn income_totals(
    draft: &DraftV2,
    replacement: Option<(i64, i64, i64)>,
) -> Option<(i64, i64)> {
    let mut income = 0i64;
    let mut withholding = 0i64;
    for r in &draft.records {
        if replacement.is_some_and(|(id, _, _)| id == r.id) {
            continue;
        }
        income = income.checked_add(r.amount)?;
        withholding = withholding.checked_add(r.withholding)?;
    }
    if let Some((_, amount, tax)) = replacement {
        if !(0..=MAX_AMOUNT).contains(&amount) || !(0..=MAX_AMOUNT).contains(&tax) {
            return None;
        }
        income = income.checked_add(amount)?;
        withholding = withholding.checked_add(tax)?;
    }
    Some((income, withholding))
}

// IRS permits whole-dollar line rounding after summing the source amounts for a line.
fn dollars(cents: i64) -> i64 {
    (cents + 50) / 100
}
// IRS Rev. Proc. 2025-32 §4.01: continuous rate schedules, not the final Form 1040 Tax Table.
pub(crate) fn rate_tax(taxable_dollars: i64, joint: bool) -> i64 {
    let edges = if joint {
        [24_800, 100_800, 211_400, 403_550, 512_450, 768_700]
    } else {
        [12_400, 50_400, 105_700, 201_775, 256_225, 640_600]
    };
    let rates = [10, 12, 22, 24, 32, 35, 37];
    let mut low = 0;
    let mut cents = 0;
    for (n, rate) in rates.iter().enumerate() {
        let high = if n < edges.len() {
            edges[n]
        } else {
            taxable_dollars
        };
        let width = (taxable_dollars.min(high) - low).max(0);
        cents += width * rate;
        low = high;
    }
    dollars(cents) * 100
}
fn issue(issues: &mut Vec<CoverageIssueV2>, section: SectionV2, message: &str) {
    issues.push(CoverageIssueV2 {
        section,
        explanation: message.into(),
    });
}

pub(crate) fn estimate(d: &DraftV2) -> EstimateV2 {
    let mut out = EstimateV2 { state: ResultStateV2::Incomplete, revision: d.revision,
        rules: "US-2026-r1 · Rev. Proc. 2025-32; Pub. 505 (2026); IRC 170(p). Whole-dollar source-line totals; continuous rate-schedule estimate, not the final Tax Table. Before penalties, interest and offsets; not filed.".into(),
        trace: Vec::new(),
        issues: Vec::new(), wages: 0, interest: 0, agi: None, standard_deduction: None,
        charity_deduction: None, taxable_income: None, income_tax: None, payments: 0, refund: None, balance: None };
    for r in &d.records {
        if r.kind == IncomeKindV2::Wage {
            out.wages += r.amount;
        } else {
            out.interest += r.amount;
        }
        out.payments += r.withholding;
        if !d.joint() && r.owner == OwnerV2::Spouse {
            issue(
                &mut out.issues,
                SectionV2::Income,
                "A spouse-owned record remains on a non-joint return. Reassign or remove it.",
            );
        }
    }
    if let Some(p) = &d.payments {
        out.payments += p.estimated + p.extension;
    }
    let mut unsupported = Vec::new();
    if d.setup
        .as_ref()
        .is_some_and(|s| s.filing_status == FilingStatusV2::Other)
    {
        issue(
            &mut unsupported,
            SectionV2::Setup,
            "Only Single and Married Filing Jointly are supported.",
        );
    }
    for (person, section) in [
        (&d.taxpayer, SectionV2::Taxpayer),
        (&d.spouse, SectionV2::Spouse),
    ] {
        if section == SectionV2::Spouse && !d.joint() {
            continue;
        }
        if let Some(p) = person {
            if p.resident != AnswerV2::Yes
                || p.blind != AnswerV2::No
                || p.claimable != AnswerV2::No
                || matches!(p.age, AgeV2::AtLeast65 | AgeV2::NotSure)
            {
                issue(
                    &mut unsupported,
                    section,
                    "Supported filers are full-year U.S. residents, under 65, not blind, and not claimable as dependents. Unknown answers require review.",
                );
            }
            if p.age == AgeV2::Under25 && out.interest > 0 {
                issue(
                    &mut unsupported,
                    section,
                    "Interest for a filer under 25 requires additional age and support checks that this prototype does not implement.",
                );
            }
        }
    }
    for answer in &d.coverage {
        if answer.answer != AnswerV2::No {
            let n = TOPICS.iter().position(|t| t == &answer.topic).unwrap_or(11);
            issue(&mut unsupported, SectionV2::Coverage, QUESTIONS[n]);
        }
    }
    if d.charity == Some(AnswerV2::NotSure)
        || (d.charity == Some(AnswerV2::Yes)
            && d.charity_qualified.is_some_and(|v| v != AnswerV2::Yes))
    {
        issue(
            &mut unsupported,
            SectionV2::Charity,
            "The cash donations need eligibility review before this prototype can estimate the deduction.",
        );
    }
    let agi = dollars(out.wages) + dollars(out.interest);
    let joint = d.joint();
    // Childless EITC screening is automatic, regardless of a generic 'no other credits' answer.
    let eitc_age = d
        .taxpayer
        .as_ref()
        .is_some_and(|p| p.age == AgeV2::From25To64)
        || (joint
            && d.spouse
                .as_ref()
                .is_some_and(|p| p.age == AgeV2::From25To64));
    if eitc_age
        && out.wages > 0
        && agi < if joint { 26_820 } else { 19_540 }
        && dollars(out.interest) <= 12_200
    {
        issue(
            &mut unsupported,
            SectionV2::Income,
            "Income may qualify for the childless Earned Income Tax Credit, which is not calculated here.",
        );
    }
    let threshold = if joint { 250_000 } else { 200_000 };
    let medicare: i64 = d.records.iter().map(|r| r.medicare_wages).sum();
    if medicare > threshold * 100 || d.records.iter().any(|r| r.medicare_wages > 20_000_000) {
        issue(
            &mut unsupported,
            SectionV2::Income,
            "Additional Medicare tax or its withholding reconciliation may apply.",
        );
    }
    if out.interest > 0 && agi > threshold {
        issue(
            &mut unsupported,
            SectionV2::Income,
            "Net Investment Income Tax may apply to the interest.",
        );
    }
    for owner in [OwnerV2::Taxpayer, OwnerV2::Spouse] {
        let social: i64 = d
            .records
            .iter()
            .filter(|r| r.owner == owner)
            .map(|r| r.social_security_withheld)
            .sum();
        if social > 1_143_900 {
            issue(
                &mut unsupported,
                SectionV2::Income,
                "Social Security withholding exceeds the 2026 per-person maximum; a refund or employer correction needs review.",
            );
        }
    }
    out.issues.extend(unsupported);
    if d.first_incomplete() != 20 {
        issue(
            &mut out.issues,
            SectionV2::Review,
            "Complete the remaining applicable interview pages before viewing an estimate.",
        );
        return out;
    }
    let standard = if joint { 32_200 } else { 16_100 };
    // IRC 170(p) excludes carryovers and the itemizer floor; eligible current cash remains subject to the percentage limit.
    let charity = if d.charity == Some(AnswerV2::Yes) && d.charity_qualified == Some(AnswerV2::Yes)
    {
        dollars(d.charity_amount.unwrap_or(0))
            .min(if joint { 2000 } else { 1000 })
            .min(dollars(agi * 60))
    } else {
        0
    };
    let taxable = (agi - standard - charity).max(0);
    let tax = rate_tax(taxable, joint);
    // A conservative tentative-AMT screen uses AGI before charitable relief. It never treats AMT as zero by assertion.
    let exemption = if joint { 140_200 } else { 90_100 };
    let phaseout = if joint { 1_000_000 } else { 500_000 };
    let exempt = (exemption - (agi - phaseout).max(0) / 2).max(0);
    let amt_base = (agi - exempt).max(0);
    let tentative = amt_base.min(244_500) * 26 + (amt_base - 244_500).max(0) * 28;
    if tentative > tax {
        issue(
            &mut out.issues,
            SectionV2::Income,
            "Alternative minimum tax needs a separate calculation outside this prototype.",
        );
    }
    if !out.issues.is_empty() {
        out.state = ResultStateV2::Unsupported;
        return out;
    }
    // Form 1040 withholding categories and payment lines round independently after aggregation.
    let wage_withheld: i64 = d
        .records
        .iter()
        .filter(|r| r.kind == IncomeKindV2::Wage)
        .map(|r| r.withholding)
        .sum();
    let interest_withheld: i64 = d
        .records
        .iter()
        .filter(|r| r.kind == IncomeKindV2::Interest)
        .map(|r| r.withholding)
        .sum();
    let payment = (dollars(wage_withheld)
        + dollars(interest_withheld)
        + d.payments
            .as_ref()
            .map_or(0, |p| dollars(p.estimated) + dollars(p.extension)))
        * 100;
    out.state = ResultStateV2::EstimatedForSupportedScope;
    out.agi = Some(agi * 100);
    out.standard_deduction = Some(standard * 100);
    out.charity_deduction = Some(charity * 100);
    out.taxable_income = Some(taxable * 100);
    out.income_tax = Some(tax);
    out.payments = payment;
    out.refund = Some((payment - tax).max(0));
    out.balance = Some((tax - payment).max(0));
    let document_inputs = |kind: Option<IncomeKindV2>, field: &str| -> Vec<String> {
        d.records
            .iter()
            .filter(|r| kind.is_none_or(|k| r.kind == k))
            .map(|r| alloc::format!("document:{}.{}", r.id, field))
            .collect()
    };
    let refs = |names: &[&str]| names.iter().map(|s| String::from(*s)).collect::<Vec<_>>();
    let mut line = |id: &str, amount, inputs, source: &str, explanation: &str| {
        out.trace.push(CalculationLineV2 {
            id: id.into(),
            amount,
            inputs,
            rule: alloc::format!("US-2026-r1.{}", id),
            source: source.into(),
            explanation: explanation.into(),
        });
    };
    let rounding = "IRS Form 1040 instructions (2025), Rounding Off to Whole Dollars; prototype policy pending final 2026 instructions: https://www.irs.gov/instructions/i1040gi";
    let rates =
        "IRS Rev. Proc. 2025-32 section 4.01: https://www.irs.gov/pub/irs-drop/rp-25-32.pdf";
    let payments =
        "IRS Publication 505 (2026), estimated tax: https://www.irs.gov/publications/p505";
    line(
        "wages",
        dollars(out.wages) * 100,
        document_inputs(Some(IncomeKindV2::Wage), "amount"),
        rounding,
        "Add exact W-2 wages, then round the total to whole dollars.",
    );
    line(
        "interest",
        dollars(out.interest) * 100,
        document_inputs(Some(IncomeKindV2::Interest), "amount"),
        rounding,
        "Add exact ordinary taxable bank interest, then round the total.",
    );
    line(
        "agi",
        agi * 100,
        refs(&["wages", "interest"]),
        payments,
        "Wages plus interest; other income and adjustments are excluded by the supported-scope checks.",
    );
    line(
        "standard_deduction",
        standard * 100,
        refs(&["setup.filing_status", "taxpayer", "spouse"]),
        "IRS Rev. Proc. 2025-32 section 4.14: https://www.irs.gov/pub/irs-drop/rp-25-32.pdf",
        "Use the published 2026 standard deduction for the supported filing status.",
    );
    line(
        "charity_deduction",
        charity * 100,
        refs(&[
            "charity",
            "charity_qualified",
            "charity_amount",
            "agi",
            "setup.filing_status",
        ]),
        "IRC 170(p), 170(b)(1)(G); https://www.irs.gov/taxtopics/tc506",
        "Apply current-year qualifying cash, the filing-status cap and the cash percentage limit. No carryovers or itemizer floor.",
    );
    line(
        "taxable_income",
        taxable * 100,
        refs(&["agi", "standard_deduction", "charity_deduction"]),
        payments,
        "Subtract supported deductions from AGI, with a minimum of zero.",
    );
    line(
        "income_tax",
        tax,
        refs(&["taxable_income", "setup.filing_status"]),
        rates,
        "Apply the continuous ordinary-income schedule and whole-dollar rounding. Not the final Tax Table.",
    );
    let mut payment_inputs = document_inputs(None, "withholding");
    payment_inputs.extend(refs(&["payments.estimated", "payments.extension"]));
    line(
        "payments",
        payment,
        payment_inputs,
        payments,
        "Round each aggregated withholding category, estimated-payment line and extension-payment line, then add.",
    );
    line(
        "refund",
        (payment - tax).max(0),
        refs(&["payments", "income_tax"]),
        payments,
        "Positive payments less tax; penalties, interest and offsets are excluded.",
    );
    line(
        "balance",
        (tax - payment).max(0),
        refs(&["income_tax", "payments"]),
        payments,
        "Positive tax less payments; penalties and interest are excluded.",
    );
    out
}
