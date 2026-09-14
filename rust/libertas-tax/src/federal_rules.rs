// 2026 estimate rules; verified 2026-09-13. Integer cents are rounded at tax-line boundaries.
// Annual parameters: IRS Rev. Proc. 2025-32, https://www.irs.gov/irb/2025-45_IRB
// Estimated tax method: https://www.irs.gov/publications/p505
// Enacted 2026 changes: https://www.govinfo.gov/content/pkg/PLAW-119publ21/html/PLAW-119publ21.htm
// This is a rate-schedule estimate, not the final filing tax/EIC tables.
use crate::*;
use alloc::{string::String, vec, vec::Vec};
use core::cmp::{max, min};

const ANNUAL: &str = "https://www.irs.gov/irb/2025-45_IRB";
const LAW: &str = "https://www.govinfo.gov/content/pkg/PLAW-119publ21/html/PLAW-119publ21.htm";
const MAX: i64 = 100_000_000_000;
pub(crate) fn dollars(cents: i64) -> i64 {
    if cents < 0 {
        -((-cents + 50) / 100)
    } else {
        (cents + 50) / 100
    }
}
fn fraction(value: i64, numerator: i64, denominator: i64) -> i64 {
    // Bounded inputs and record counts keep intermediates within i64; use i128 for products.
    ((i128::from(value) * i128::from(numerator) + i128::from(denominator / 2))
        / i128::from(denominator)) as i64
}
fn positive(v: i64) -> i64 {
    max(v, 0)
}
fn ceiling(v: i64, unit: i64) -> i64 {
    (positive(v) + unit - 1) / unit
}
fn yes(v: Answer) -> bool {
    v == Answer::Yes
}
fn issue(result: &mut FederalResult, section: &str, message: &str) {
    result.issues.push(FederalIssue {
        section: section.into(),
        message: message.into(),
    });
}
fn require(result: &mut FederalResult, ok: bool, section: &str, message: &str) {
    if !ok {
        issue(result, section, message);
    }
}
fn line(
    result: &mut FederalResult,
    id: &str,
    amount: i64,
    inputs: &[&str],
    source: &str,
    explanation: &str,
) {
    result.breakdown.push(FederalAmount {
        label: if id == "agi" {
            "Adjusted gross income".into()
        } else {
            id.replace('_', " ")
        },
        amount: amount * 100,
        explanation: explanation.into(),
    });
    result.lines.push(CalculationLine {
        id: id.into(),
        amount: amount * 100,
        inputs: inputs.iter().map(|s| String::from(*s)).collect(),
        rule: alloc::format!("US-2026.{}", id),
        source: source.into(),
        explanation: explanation.into(),
    });
}
pub(crate) fn valid_birth(date: u32) -> bool {
    let y = date / 10000;
    let m = date / 100 % 100;
    let d = date % 100;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if y.is_multiple_of(400) || (y.is_multiple_of(4) && !y.is_multiple_of(100)) {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    (1850..=2026).contains(&y) && (1..=days).contains(&d)
}
fn age(date: u32) -> u32 {
    2026 - date / 10000
}
fn senior(date: u32) -> bool {
    date <= 19620101
}
fn joint(s: FederalStatus) -> bool {
    s == FederalStatus::Joint
}
fn joint_rates(s: FederalStatus) -> bool {
    matches!(s, FederalStatus::Joint | FederalStatus::Surviving)
}
pub(crate) fn brackets(s: FederalStatus) -> [i64; 6] {
    match s {
        FederalStatus::Single => [12400, 50400, 105700, 201775, 256225, 640600],
        FederalStatus::Joint | FederalStatus::Surviving => {
            [24800, 100800, 211400, 403550, 512450, 768700]
        }
        FederalStatus::Head => [17700, 67450, 105700, 201750, 256200, 640600],
        FederalStatus::Separate => [12400, 50400, 105700, 201775, 256225, 384350],
    }
}
// Return cents so ordinary and preferential components round once, together.
fn ordinary_cents(income: i64, status: FederalStatus) -> i64 {
    let mut previous = 0;
    let mut result = 0;
    for (limit, rate) in brackets(status)
        .into_iter()
        .chain([i64::MAX])
        .zip([10, 12, 22, 24, 32, 35, 37])
    {
        result += positive(min(income, limit) - previous) * rate;
        if income <= limit {
            break;
        }
        previous = limit;
    }
    result
}
pub(crate) fn preferential_tax(income: i64, preferential: i64, status: FederalStatus) -> i64 {
    let gain = min(positive(preferential), income);
    let ordinary = income - gain;
    let (zero, fifteen) = match status {
        FederalStatus::Joint | FederalStatus::Surviving => (98900, 613700),
        FederalStatus::Head => (66200, 579600),
        FederalStatus::Separate => (49450, 306850),
        FederalStatus::Single => (49450, 545500),
    };
    let free = min(gain, positive(zero - ordinary));
    let middle = min(gain - free, positive(fifteen - ordinary - free));
    dollars(min(
        ordinary_cents(income, status),
        ordinary_cents(ordinary, status) + middle * 15 + (gain - free - middle) * 20,
    ))
}
// Notice 2025-67 supplies 2026 boundaries; Form 8880 supplies the worksheet.
// Surviving-spouse status uses the "all other" Saver's Credit band, not joint brackets.
pub(crate) fn saver_rate(agi: i64, status: FederalStatus) -> i64 {
    let limits = match status {
        FederalStatus::Joint => [48500, 52500, 80500],
        FederalStatus::Head => [36375, 39375, 60375],
        _ => [24250, 26250, 40250],
    };
    if agi <= limits[0] {
        50
    } else if agi <= limits[1] {
        20
    } else if agi <= limits[2] {
        10
    } else {
        0
    }
}

pub(crate) fn saver_potential(
    facts: &FederalSaver,
    people: &FederalPeople,
    status: FederalStatus,
    agi: i64,
) -> Result<i64, &'static str> {
    let j = joint(status);
    if !yes(facts.eligible_contributions) || !yes(facts.reviewed_distributions) {
        return Err(
            "Confirm qualifying workplace contributions and the complete 2024–2027 distribution lookback, including expected amounts for an estimate.",
        );
    }
    if !j
        && (facts.spouse_contributions.amount() != 0
            || facts.distributions.iter().any(|d| d.owner == Owner::Spouse))
    {
        return Err(
            "Spouse contributions and distributions belong only on a joint return. Revisit Saver’s Credit after a filing-status change.",
        );
    }
    let mut offsets = [0, 0];
    for d in &facts.distributions {
        let owner = if d.owner == Owner::Taxpayer { 0 } else { 1 };
        offsets[owner] += d.amount;
        if j && d.year() != SaverDistributionYear::TaxYear
            && d.amount > 0
            && d.joint_with_current_spouse() == Answer::NotSure
        {
            return Err(
                "Confirm whether you filed or expect to file jointly with the current spouse for each distribution year.",
            );
        }
        // A recipient's own withdrawals always reduce their base. Cross-spouse reductions
        // depend on filing together in the distribution year, even if the recipient is ineligible.
        if j && (d.year() == SaverDistributionYear::TaxYear || yes(d.joint_with_current_spouse())) {
            offsets[1 - owner] += d.amount;
        }
    }
    let contributions = [
        facts.taxpayer_contributions,
        facts.spouse_contributions.amount(),
    ];
    let persons = [
        Some(&people.taxpayer),
        if j { people.spouse.as_ref() } else { None },
    ];
    let eligible = (0..2)
        .map(|i| {
            if persons[i].is_some_and(|p| {
                p.birth_date <= 20090101 && !p.full_time_student && p.claimable == Answer::No
            }) {
                // Aggregate source cents within each worksheet line before whole-dollar rounding.
                min(
                    positive(dollars(contributions[i]) - dollars(offsets[i])),
                    2000,
                )
            } else {
                0
            }
        })
        .sum();
    Ok(fraction(eligible, saver_rate(agi, status), 100))
}

// Schedule 1-A deliberately floors tips/overtime phaseout steps but CEILS vehicle steps.
pub(crate) fn additional_deductions(
    agi: i64,
    status: FederalStatus,
    tips: i64,
    overtime: i64,
    vehicle: i64,
) -> [i64; 3] {
    let j = joint(status);
    let work_reduction = positive(agi - if j { 300000 } else { 150000 }) / 1000 * 100;
    [
        if status == FederalStatus::Separate {
            0
        } else {
            positive(min(tips, 25000) - work_reduction)
        },
        if status == FederalStatus::Separate {
            0
        } else {
            positive(min(overtime, if j { 25000 } else { 12500 }) - work_reduction)
        },
        positive(min(vehicle, 10000) - ceiling(agi - if j { 200000 } else { 100000 }, 1000) * 200),
    ]
}

pub(crate) fn social_taxable(
    benefits: i64,
    other: i64,
    exempt: i64,
    status: FederalStatus,
    apart: bool,
) -> i64 {
    // Pub. 915 Worksheet 1 excludes the student-loan deduction from adjustments here.
    let provisional = positive((other + exempt) * 100 + benefits * 50);
    if status == FederalStatus::Separate && !apart {
        return dollars(min(benefits * 85, fraction(provisional, 85, 100)));
    }
    let (base, band) = if joint(status) {
        (32000, 12000)
    } else {
        (25000, 9000)
    };
    let excess = positive(provisional - base * 100);
    dollars(min(
        benefits * 85,
        min(benefits * 50, min(excess, band * 100) / 2)
            + fraction(positive(excess - band * 100), 85, 100),
    ))
}

impl FederalDraft {
    pub(crate) fn empty(cookie: String) -> Self {
        Self {
            tax_year: 2026,
            next_id: 1,
            cookie,
            revision: 0,
            setup: None,
            people: None,
            dependents: Vec::new(),
            dependents_complete: false,
            income: Vec::new(),
            income_complete: false,
            adjustments: None,
            deductions: None,
            credits: None,
            screening: None,
            payments: None,
            finished: false,
        }
    }
    pub(crate) fn first_incomplete(&self) -> i32 {
        if self.setup.is_none() {
            0
        } else if self.people.is_none() {
            1
        } else if !self.dependents_complete {
            2
        } else if !self.income_complete {
            3
        } else if self.adjustments.is_none() {
            4
        } else if self.deductions.is_none() {
            5
        } else if self.credits.is_none() {
            6
        } else if self.screening.is_none() {
            7
        } else if self.payments.is_none() {
            8
        } else {
            9
        }
    }
    pub(crate) fn erase_after(&mut self, page: i32) {
        if page < 1 {
            self.people = None;
        }
        if page < 2 {
            self.dependents.clear();
            self.dependents_complete = false;
        }
        if page < 3 {
            self.income.clear();
            self.income_complete = false;
        }
        if page < 4 {
            self.adjustments = None;
        }
        if page < 5 {
            self.deductions = None;
        }
        if page < 6 {
            self.credits = None;
        }
        if page < 7 {
            self.screening = None;
        }
        if page < 8 {
            self.payments = None;
        }
        self.finished = false;
    }
    pub(crate) fn valid_stored(&self) -> bool {
        if self.tax_year != 2026
            || self.cookie.is_empty()
            || self.cookie.len() > 128
            || self.revision < 0
            || self.revision == i64::MAX
            || self.next_id < 1
            || self.next_id == i64::MAX
            || self.income.len() > 100
            || self.dependents.len() > 30
        {
            return false;
        }
        let ids: Vec<i64> = self
            .income
            .iter()
            .map(|v| v.id)
            .chain(self.dependents.iter().map(|v| v.id))
            .collect();
        if ids
            .iter()
            .enumerate()
            .any(|(i, &id)| id <= 0 || id >= self.next_id || ids[..i].contains(&id))
        {
            return false;
        }
        if self.income.iter().any(|v| validate_income(v).is_err())
            || self
                .dependents
                .iter()
                .any(|v| validate_dependent(v).is_err())
        {
            return false;
        }
        if self.people.as_ref().is_some_and(|v| {
            v.taxpayer.name.trim().is_empty()
                || v.taxpayer.name.len() > 160
                || v.spouse
                    .as_ref()
                    .is_some_and(|p| p.name.trim().is_empty() || p.name.len() > 160)
                || !valid_birth(v.taxpayer.birth_date)
                || v.spouse
                    .as_ref()
                    .is_some_and(|p| !valid_birth(p.birth_date))
        }) {
            return false;
        }
        // Restored data is checked with the same amount invariants as accepted pages.
        if self.adjustments.as_ref().is_some_and(|v| {
            !amounts(&[
                v.student_loan_interest(),
                v.educator_taxpayer(),
                v.educator_spouse(),
            ]) || v.iras.as_ref().is_some_and(|f| {
                core::iter::once(&f.taxpayer)
                    .chain(f.spouse.iter())
                    .any(|v| !amounts(&[v.traditional, v.roth]))
            }) || v.hsa.len() > 10
                || v.hsa
                    .iter()
                    .any(|h| !amounts(&[h.personal, h.employer, h.distributions]))
        }) || self.deductions.as_ref().is_some_and(|v| {
            !amounts(&[
                v.medical(),
                v.state_local_taxes(),
                v.mortgage_interest(),
                v.cash_charity(),
            ]) || v.vehicle_loans.len() > 20
                || v.vehicle_loans
                    .iter()
                    .any(|v| v.label.is_empty() || v.label.len() > 320 || !amounts(&[v.interest]))
        }) || self.credits.as_ref().is_some_and(|v| {
            !amounts(&[v.care_expenses(), v.care_benefits()])
                || v.saver.as_ref().is_some_and(|f| {
                    !amounts(&[f.taxpayer_contributions, f.spouse_contributions.amount()])
                        || f.distributions.len() > 100
                        || f.distributions.iter().any(|d| !amounts(&[d.amount]))
                })
                || v.students.len() > 64
                || v.students.iter().enumerate().any(|(i, p)| {
                    p.id == 0
                        || p.id < -2
                        || p.id >= self.next_id
                        || v.students[..i].iter().any(|prior| prior.id == p.id)
                })
                || v.education.len() > 30
                || v.education
                    .iter()
                    .any(|e| !amounts(&[e.expenses]) || e.student as usize >= v.students.len())
        }) || self
            .payments
            .as_ref()
            .is_some_and(|v| !amounts(&[v.estimated, v.extension]))
        {
            return false;
        }
        true
    }
}
fn amounts(v: &[i64]) -> bool {
    v.iter().all(|n| (0..=MAX).contains(n))
}
pub(crate) fn validate_dependent(d: &FederalDependent) -> Result<(), &'static str> {
    if !valid_birth(d.birth_date) {
        return Err("Enter a valid birth date on or before December 31, 2026.");
    }
    if d.label.is_empty()
        || d.label.len() > 320
        || d.days_in_home() > 365
        || !amounts(&[d.care_expenses(), d.gross_income])
    {
        return Err("Review this person's label, residency days and amounts.");
    }
    if d.own_support_over_half && d.your_support_over_half {
        return Err(
            "Both you and this person cannot each provide more than half the same total support.",
        );
    }
    Ok(())
}
pub(crate) fn validate_income(e: &FederalIncomeEntry) -> Result<(), &'static str> {
    if e.label.is_empty() || e.label.len() > 320 {
        return Err("Enter a payer or document label.");
    }
    let valid = match &e.income {
        FederalIncome::Wage { data: w } => {
            amounts(&[
                w.wages,
                w.withholding,
                w.social_wages,
                w.social_tips,
                w.social_tax,
                w.medicare_wages,
                w.medicare_tax,
            ]) && w
                .work_deductions
                .as_ref()
                .is_none_or(|v| amounts(&[v.tips(), v.overtime()]))
        }
        FederalIncome::Interest { data: v } => amounts(&[v.taxable, v.exempt, v.withholding]),
        FederalIncome::Dividend { data: v } => {
            if v.qualified > v.ordinary {
                return Err(
                    "Qualified dividends must be included in, and cannot exceed, ordinary dividends.",
                );
            }
            amounts(&[
                v.ordinary,
                v.qualified,
                v.capital_distributions,
                v.withholding,
            ])
        }
        FederalIncome::Sale { data: v } => {
            if v.wash_sale > positive(v.basis - v.proceeds) {
                return Err("A disallowed wash-sale loss cannot exceed this sale's loss.");
            }
            amounts(&[v.proceeds, v.basis, v.wash_sale])
        }
        FederalIncome::Retirement { data: v } => {
            if v.taxable > v.gross {
                return Err(
                    "Taxable retirement income cannot exceed its gross distribution in this supported path.",
                );
            }
            amounts(&[v.gross, v.taxable, v.withholding])
        }
        FederalIncome::SocialSecurity { data: v } => amounts(&[v.benefits, v.withholding]),
        FederalIncome::Unemployment { data: v } => amounts(&[v.amount, v.withholding]),
        FederalIncome::Business { data: v } => amounts(&[v.receipts, v.expenses]),
    };
    if valid {
        Ok(())
    } else {
        Err("Review the document amounts; they must be within the supported input range.")
    }
}

#[derive(Default)]
struct Totals {
    wages: [i64; 2],
    social_wages: [i64; 2],
    social_tax: [i64; 2],
    tips: [i64; 2],
    overtime: [i64; 2],
    medicare_wages: [i64; 2],
    medicare_tax: [i64; 2],
    business: [i64; 2],
    interest: i64,
    exempt: i64,
    dividends: i64,
    qualified: i64,
    short: i64,
    long: i64,
    retirement: i64,
    social: i64,
    unemployment: i64,
    withholding: i64,
}
fn collect(d: &FederalDraft, r: &mut FederalResult) -> Totals {
    let mut t = Totals::default();
    for e in &d.income {
        let owner = usize::from(e.owner == Owner::Spouse);
        let source = alloc::format!("Income: {}", e.label);
        require(
            r,
            owner == 0 || d.setup.as_ref().is_some_and(|s| joint(s.filing_status())),
            &source,
            "Spouse-owned income requires a joint return in this prototype.",
        );
        match &e.income {
            FederalIncome::Wage { data: w } => {
                t.wages[owner] += w.wages;
                t.social_wages[owner] += w.social_wages + w.social_tips;
                t.social_tax[owner] += w.social_tax;
                t.medicare_wages[owner] += w.medicare_wages;
                t.medicare_tax[owner] += w.medicare_tax;
                t.withholding += w.withholding;
                if let Some(v) = w.work_deductions.as_ref() {
                    require(
                        r,
                        v.tips() == 0 || yes(v.tips_eligible()),
                        &source,
                        "Confirm the 2026 qualified-tip occupation, reporting and business eligibility.",
                    );
                    require(
                        r,
                        v.overtime() == 0 || yes(v.overtime_eligible()),
                        &source,
                        "Confirm the FLSA-qualified overtime premium and 2026 reporting.",
                    );
                    t.tips[owner] += v.tips();
                    t.overtime[owner] += v.overtime();
                }
                require(
                    r,
                    w.special_boxes == Answer::No,
                    &source,
                    "Special W-2 treatment needs a separate supported worksheet.",
                );
                require(
                    r,
                    w.social_wages + w.social_tips <= 18_450_000
                        && w.social_tax <= 1_143_900
                        && w.social_tax <= fraction(w.social_wages + w.social_tips, 62, 1000) + 1,
                    &source,
                    "Excess Social Security withholding by one employer requires employer correction, not the multiple-employer credit.",
                );
                require(
                    r,
                    w.medicare_tax + 1 >= fraction(w.medicare_wages, 145, 10000),
                    &source,
                    "Medicare withholding below ordinary payroll tax requires special payroll review.",
                );
            }
            FederalIncome::Interest { data: v } => {
                t.interest += v.taxable;
                t.exempt += v.exempt;
                t.withholding += v.withholding;
                require(
                    r,
                    v.special_treatment == Answer::No,
                    &source,
                    "Special interest treatment is not yet supported.",
                );
            }
            FederalIncome::Dividend { data: v } => {
                t.dividends += v.ordinary;
                t.qualified += v.qualified;
                t.long += v.capital_distributions;
                t.withholding += v.withholding;
                require(
                    r,
                    v.special_treatment == Answer::No,
                    &source,
                    "Special dividend or capital-gain distributions are not yet supported.",
                );
            }
            FederalIncome::Sale { data: v } => {
                let gain = v.proceeds - v.basis + v.wash_sale;
                if v.term == GainTerm::Short {
                    t.short += gain;
                } else {
                    t.long += gain;
                }
                require(
                    r,
                    v.special_treatment == Answer::No,
                    &source,
                    "This sale requires a special investment worksheet.",
                );
            }
            FederalIncome::Retirement { data: v } => {
                t.retirement += v.taxable;
                t.withholding += v.withholding;
                require(
                    r,
                    yes(v.ordinary_distribution),
                    &source,
                    "Retirement income needs a known ordinary taxable amount, without early-distribution tax, rollover or basis calculations.",
                );
            }
            FederalIncome::SocialSecurity { data: v } => {
                t.social += v.benefits;
                t.withholding += v.withholding;
                require(
                    r,
                    v.special_treatment == Answer::No,
                    &source,
                    "Prior-year lump sums, net repayments and special benefit treatment are not yet supported.",
                );
            }
            FederalIncome::Unemployment { data: v } => {
                t.unemployment += v.amount;
                t.withholding += v.withholding;
                require(
                    r,
                    v.repaid == Answer::No,
                    &source,
                    "Repaid unemployment requires a separate adjustment.",
                );
            }
            FederalIncome::Business { data: v } => {
                t.business[owner] += v.receipts - v.expenses;
                require(
                    r,
                    yes(v.material_participation)
                        && v.special_treatment == Answer::No
                        && v.receipts >= v.expenses,
                    &source,
                    "Only profitable, actively operated, simple cash-basis sole proprietorships are supported; losses and special business treatments need additional worksheets.",
                );
            }
        }
    }
    t
}

struct Family {
    dependent_ids: Vec<i64>,
    ctc: i64,
    odc: i64,
    eic_children: usize,
    care_people: i64,
    care_expenses: i64,
    qualifying_head: bool,
    qualifying_married_head: bool,
    qualifying_survivor: bool,
}
fn family(d: &FederalDraft, r: &mut FederalResult, people: &FederalPeople) -> Family {
    let mut f = Family {
        dependent_ids: Vec::new(),
        ctc: 0,
        odc: 0,
        eic_children: 0,
        care_people: 0,
        care_expenses: 0,
        qualifying_head: false,
        qualifying_married_head: false,
        qualifying_survivor: false,
    };
    let younger_than = people
        .spouse
        .as_ref()
        .map_or(people.taxpayer.birth_date, |p| {
            min(p.birth_date, people.taxpayer.birth_date)
        });
    let tin =
        yes(people.taxpayer.valid_tin) && people.spouse.as_ref().is_none_or(|p| yes(p.valid_tin));
    let ssn = (yes(people.taxpayer.valid_ssn)
        || people.spouse.as_ref().is_some_and(|p| yes(p.valid_ssn)))
        && tin;
    for c in &d.dependents {
        let section = alloc::format!("Children: {}", c.label);
        require(
            r,
            c.competing_claim == Answer::No,
            &section,
            "Custody, tiebreakers and competing dependency claims need additional review.",
        );
        require(
            r,
            c.us_citizen_resident != Answer::NotSure
                && c.valid_tin != Answer::NotSure
                && c.valid_ssn != Answer::NotSure
                && c.us_home_over_half_year != Answer::NotSure,
            &section,
            "Resolve citizenship, identification and U.S. residency eligibility.",
        );
        let related = matches!(
            c.relationship,
            Relationship::Child
                | Relationship::Foster
                | Relationship::Grandchild
                | Relationship::Sibling
                | Relationship::NieceNephew
        );
        let lived = c.days_in_home() >= 183;
        let whole = c.days_in_home() == 365;
        let age_ok = c.disabled
            || ((age(c.birth_date) < 19 || (age(c.birth_date) < 24 && c.student))
                && c.birth_date > younger_than);
        let child = related && lived && age_ok && !c.joint_return_nonrefund;
        let dependency_child = child && !c.own_support_over_half;
        let relative = !child
            && c.your_support_over_half
            && c.gross_income < 530_000
            && (c.relationship != Relationship::Unrelated || whole)
            && !c.joint_return_nonrefund;
        let dependent = (dependency_child || relative) && yes(c.us_citizen_resident);
        if dependent {
            f.dependent_ids.push(c.id);
        }
        if dependent && age(c.birth_date) < 17 && dependency_child && yes(c.valid_ssn) && ssn {
            f.ctc += 1;
        } else if dependent && yes(c.valid_tin) && tin {
            f.odc += 1;
        }
        // EITC has no support test: a qualifying child need not be claimed as a dependent.
        if child && yes(c.valid_ssn) && yes(c.us_home_over_half_year) {
            f.eic_children += 1;
        }
        f.qualifying_head |= dependent
            && c.relationship != Relationship::Unrelated
            && (lived || c.relationship == Relationship::Parent);
        f.qualifying_married_head |= dependency_child
            && matches!(c.relationship, Relationship::Child | Relationship::Foster);
        f.qualifying_survivor |= dependency_child && whole && c.relationship == Relationship::Child;
        // The two-person expense ceiling applies even if expenses were paid for only one person.
        if dependent && lived && (c.birth_date > 20130101 || c.incapable_self_care) {
            f.care_people += 1;
        }
        if c.care_expenses() > 0 {
            require(
                r,
                dependent && lived && yes(c.care_period_eligible()),
                &section,
                "Care expenses need an eligible dependent, residency and eligible age/self-care periods.",
            );
            f.care_expenses += c.care_expenses();
        }
    }
    f
}
fn household(s: &FederalSetup, p: &FederalPeople, f: &Family, r: &mut FederalResult) {
    let married = matches!(
        s.marital_state,
        MaritalState::Married { .. } | MaritalState::Widowed2026
    );
    let valid = match s.filing_status() {
        FederalStatus::Single => !married,
        FederalStatus::Joint | FederalStatus::Separate => married,
        FederalStatus::Head => {
            (!married
                || (matches!(s.marital_state, MaritalState::Married { .. })
                    && s.lived_apart_last_six_months()
                    && f.qualifying_married_head))
                && s.paid_over_half_home
                && f.qualifying_head
        }
        FederalStatus::Surviving => {
            matches!(
                s.marital_state,
                MaritalState::Widowed2025 { .. } | MaritalState::Widowed2024 { .. }
            ) && yes(s.could_file_joint_death_year())
                && s.paid_over_half_home
                && f.qualifying_survivor
        }
    };
    require(
        r,
        valid,
        "Filing status",
        "The household facts do not establish eligibility for the selected filing status; special dependency exceptions are not yet supported.",
    );
    require(
        r,
        s.community_property == Answer::No,
        "Filing status",
        "Community-property allocation is not yet supported.",
    );
    require(
        r,
        joint(s.filing_status()) == p.spouse.is_some(),
        "Filers",
        "Provide spouse facts for a joint return only.",
    );
    for person in core::iter::once(&p.taxpayer).chain(p.spouse.iter()) {
        require(
            r,
            yes(person.full_year_resident) && person.claimable == Answer::No,
            "Filers",
            "Part-year/nonresident and dependent-filer returns are not yet supported.",
        );
        require(
            r,
            person.another_eitc_child == Answer::No
                && person.alive_at_year_end
                && person.valid_ssn != Answer::NotSure
                && person.valid_tin != Answer::NotSure
                && person.us_home_over_half_year != Answer::NotSure,
            "Filers",
            "Resolve identification and U.S. residency eligibility. Deceased-filer returns and filers who may be another taxpayer’s EITC qualifying child need additional review.",
        );
    }
}
pub(crate) fn care_rate(agi: i64, joint: bool) -> i64 {
    max(
        20,
        max(35, 50 - ceiling(agi - 15000, 2000))
            - ceiling(
                agi - if joint { 150000 } else { 75000 },
                if joint { 4000 } else { 2000 },
            ),
    )
}
pub(crate) fn eic_amount(earned: i64, agi: i64, children: usize, joint: bool) -> i64 {
    let n = min(children, 3);
    // The rounded joint phase-out increment differs between childless and child claims.
    let start = match (n == 0, joint) {
        (true, false) => 10860,
        (true, true) => 18140,
        (false, false) => 23890,
        (false, true) => 31160,
    };
    let credit = min(
        [664, 4427, 7316, 8231][n] * 100,
        fraction(positive(earned) * 100, [765, 3400, 4000, 4500][n], 10000),
    );
    dollars(positive(
        credit
            - fraction(
                positive(max(earned, agi) - start) * 100,
                [765, 1598, 2106, 2106][n],
                10000,
            ),
    ))
}

pub(crate) fn estimate(d: &FederalDraft) -> FederalResult {
    let mut r = FederalResult {state:ResultState::Incomplete,revision:d.revision,issues:vec![],lines:vec![],breakdown:vec![],tax:None,refundable_credits:None,payments:None,refund:None,balance:None,
        method:"2026 federal rate-schedule estimate using whole-dollar tax lines. Final filing tax/EIC tables, penalties, interest, offsets, forms and e-filing are excluded. Synthetic data only.".into()};
    if !d.valid_stored() {
        issue(
            &mut r,
            "Saved return",
            "Accepted data is invalid; no calculation was performed.",
        );
        return r;
    }
    if d.first_incomplete() != 9 {
        issue(
            &mut r,
            "Interview",
            "Complete the remaining applicable pages to calculate the federal estimate.",
        );
        return r;
    }
    let (s, p, a, ded, c, screen, pay) = (
        d.setup.as_ref().unwrap(),
        d.people.as_ref().unwrap(),
        d.adjustments.as_ref().unwrap(),
        d.deductions.as_ref().unwrap(),
        d.credits.as_ref().unwrap(),
        d.screening.as_ref().unwrap(),
        d.payments.as_ref().unwrap(),
    );
    let j = joint(s.filing_status());
    let separate = s.filing_status() == FederalStatus::Separate;
    let family = family(d, &mut r, p);
    household(s, p, &family, &mut r);
    for (answer, message) in [
        (screen.foreign, "Foreign, treaty and territorial treatment"),
        (screen.property, "Rental, K-1, trust, farm or other income"),
        (screen.carryovers, "Prior-year carryovers"),
        (screen.special_taxes, "Special tax or AMT preferences"),
        (screen.uncertain, "Uncertain or unlisted tax treatment"),
        (a.other_adjustments, "Other income adjustments"),
        (ded.special_itemizing, "Special itemized deductions"),
        (
            ded.special_work_deductions,
            "Special tips, overtime or vehicle-interest treatment",
        ),
        (c.special_credits, "Other credits and recaptures"),
    ] {
        require(
            &mut r,
            answer == Answer::No,
            "Coverage",
            &alloc::format!(
                "{} requires an additional worksheet before this prototype can estimate the return.",
                message
            ),
        );
    }
    let t = collect(d, &mut r);
    let wages = dollars(t.wages.iter().sum());
    let interest = dollars(t.interest);
    let exempt = dollars(t.exempt);
    let dividends = dollars(t.dividends);
    let qualified = dollars(t.qualified);
    let short = dollars(t.short);
    let long = dollars(t.long);
    let capital = max(short + long, if separate { -1500 } else { -3000 });
    let pref = qualified + min(positive(long), positive(short + long));
    let mut se = [0; 2];
    let mut half = [0; 2];
    let mut net = [0; 2];
    let mut earned = [0; 2];
    for i in 0..2 {
        let profit = dollars(t.business[i]);
        net[i] = fraction(positive(profit), 9235, 10000);
        if net[i] >= 400 {
            se[i] = dollars(
                min(net[i], positive(184500 - dollars(t.social_wages[i]))) * 124 / 10
                    + net[i] * 29 / 10,
            );
            half[i] = fraction(se[i], 1, 2);
        } else {
            net[i] = 0;
        }
        earned[i] = dollars(t.wages[i]) + profit - half[i];
    }
    let se_tax = se.iter().sum::<i64>();
    let half_se = half.iter().sum::<i64>();
    let business = dollars(t.business.iter().sum());
    require(
        &mut r,
        a.educator_spouse() == 0 || j,
        "Adjustments",
        "Spouse educator expenses require a joint return.",
    );
    require(
        &mut r,
        (a.educator_taxpayer() == 0 && a.educator_spouse() == 0) || yes(a.eligible_educators()),
        "Adjustments",
        "Confirm eligible educator requirements.",
    );

    let educator = min(dollars(a.educator_taxpayer()), 350)
        + if j {
            min(dollars(a.educator_spouse()), 350)
        } else {
            0
        };
    let mut hsa = 0;
    let mut hsa_total = [0; 2];
    let mut hsa_catchup = [0; 2];
    let mut family_hsa = false;
    for h in &a.hsa {
        let i = usize::from(h.owner == Owner::Spouse);
        let person = if i == 0 {
            Some(&p.taxpayer)
        } else {
            p.spouse.as_ref()
        };
        require(
            &mut r,
            person.is_some() && yes(h.fully_eligible),
            "HSA",
            "Each HSA must have an eligible filer and full-year coverage without special contribution/distribution treatment.",
        );
        family_hsa |= h.coverage == HsaCoverage::Family;
        hsa_total[i] += h.personal + h.employer;
        hsa += h.personal;
        if person.is_some_and(|v| age(v.birth_date) >= 55) {
            hsa_catchup[i] = 100000;
        }
        require(
            &mut r,
            person.is_none_or(|v| !senior(v.birth_date)),
            "HSA",
            "Medicare-age HSA contributions require month-by-month eligibility and are not yet supported.",
        );
    }
    let hsa_ok = if family_hsa {
        (0..2)
            .map(|i| positive(hsa_total[i] - hsa_catchup[i]))
            .sum::<i64>()
            <= 875000
    } else {
        (0..2).all(|i| hsa_total[i] <= 440000 + hsa_catchup[i])
    };
    require(
        &mut r,
        hsa_ok,
        "HSA",
        "Contributions exceed the supported annual/shared family limit.",
    );
    let before_social = wages
        + interest
        + dividends
        + capital
        + dollars(t.retirement)
        + dollars(t.unemployment)
        + business
        - half_se
        - educator
        - dollars(hsa);
    // Pub. 590-A Appendix B computes IRA MAGI before the IRA deduction, then
    // recomputes taxable Social Security after that deduction. No fixed-point iteration.
    let ira_magi = before_social
        + social_taxable(
            dollars(t.social),
            before_social,
            exempt,
            s.filing_status(),
            s.lived_apart_all_year(),
        );
    let ira = match a.iras.as_ref() {
        Some(f) => match crate::ira::calculate(f, p, s, earned, ira_magi) {
            Ok(v) => v,
            Err(message) => {
                issue(&mut r, "IRA contributions", message);
                crate::ira::IraResult::default()
            }
        },
        None => crate::ira::IraResult::default(),
    };
    require(
        &mut r,
        ira.nondeductible.iter().sum::<i64>() == 0 || t.retirement == 0,
        "IRA contributions",
        "Nondeductible contributions together with retirement distributions require basis allocation before a complete estimate.",
    );
    let social = social_taxable(
        dollars(t.social),
        before_social - ira.deduction,
        exempt,
        s.filing_status(),
        s.lived_apart_all_year(),
    );
    if let Some(f) = a.iras.as_ref() {
        // With no conversions/excluded income in scope, Worksheet 2-1 adds back the
        // IRA and student-loan deductions to final AGI but retains final taxable benefits.
        if let Err(message) = crate::ira::validate_roth(f, p, s, earned, before_social + social) {
            issue(&mut r, "IRA contributions", message);
        }
    }
    // Form 8615 can apply even when the filer is not another person's dependent.
    // Its "more than half" test differs from the AOTC refund's "less than half" test.
    let unearned = positive(
        interest + dividends + capital + dollars(t.retirement) + social + dollars(t.unemployment),
    );
    let taxpayer_age = age(p.taxpayer.birth_date);
    let kiddie_age = taxpayer_age < 18
        || ((taxpayer_age == 18 || (taxpayer_age < 24 && p.taxpayer.full_time_student))
            && p.taxpayer.support_share != SupportShare::MoreThanHalf);
    require(
        &mut r,
        !(unearned > 2700 && !j && p.taxpayer.parent_alive && kiddie_age),
        "Investment tax",
        "This young filer's unearned income requires Form 8615, which is not yet supported.",
    );
    let before_loan = before_social - ira.deduction + social;
    let loan_start = if j { 175000 } else { 85000 };
    let loan_range = if j { 30000 } else { 15000 };
    require(
        &mut r,
        a.student_loan_interest() == 0 || yes(a.student_loan_eligible()),
        "Student loan",
        "Confirm that the interest and claimant satisfy the student-loan requirements.",
    );
    let loan = if separate {
        0
    } else {
        fraction(
            min(dollars(a.student_loan_interest()), 2500),
            min(positive(loan_start + loan_range - before_loan), loan_range),
            loan_range,
        )
    };
    let agi = before_loan - loan;
    line(
        &mut r,
        "wages",
        wages,
        &["income.wages"],
        "https://www.irs.gov/instructions/i1040gi",
        "Aggregate W-2 box 1 amounts, then round the tax line.",
    );
    line(
        &mut r,
        "capital_gain",
        capital,
        &["income.sales", "income.dividends"],
        "https://www.irs.gov/instructions/i1040sd",
        "Net short- and long-term gains; apply the current-year ordinary loss limit. Carryforward output is not a filed schedule.",
    );
    line(
        &mut r,
        "capital_loss_carryforward",
        positive(-short - long) - positive(-capital),
        &["capital_gain"],
        "https://www.irs.gov/instructions/i1040sd",
        "Unused current-year capital loss for later-year review; this prototype does not prepare next year's return.",
    );
    line(
        &mut r,
        "self_employment_tax",
        se_tax,
        &["income.business", "income.social_wages"],
        "https://www.irs.gov/instructions/i1040sse",
        "Per-filer net earnings, Social Security wage-base coordination and Medicare self-employment tax.",
    );
    line(
        &mut r,
        "taxable_social_security",
        social,
        &["income.social_security", "income", "adjustments"],
        "https://www.irs.gov/publications/p915",
        "Pub. 915 provisional-income calculation before the student-loan deduction.",
    );
    line(
        &mut r,
        "adjustments",
        half_se + educator + dollars(hsa) + ira.deduction + loan,
        &["self_employment_tax", "adjustments"],
        ANNUAL,
        "Half of self-employment tax, eligible educator/HSA/IRA amounts and phased student-loan interest.",
    );
    for (id, amount) in [
        ("ira_deduction", ira.deduction),
        ("taxpayer_nondeductible_ira", ira.nondeductible[0]),
        ("spouse_nondeductible_ira", ira.nondeductible[1]),
    ] {
        line(
            &mut r,
            id,
            amount,
            &["adjustments.iras", "income", "people"],
            "https://www.irs.gov/publications/p590a",
            "2026 regular IRA contribution treatment. Nondeductible traditional amounts require Form 8606 and basis records; this prototype prepares no forms.",
        );
    }
    line(
        &mut r,
        "agi",
        agi,
        &["income", "taxable_social_security", "adjustments"],
        "https://www.irs.gov/publications/p505",
        "Total income less supported adjustments.",
    );
    let mut standard = match s.filing_status() {
        FederalStatus::Single | FederalStatus::Separate => 16100,
        FederalStatus::Head => 24150,
        _ => 32200,
    };
    let additional = if matches!(
        s.filing_status(),
        FederalStatus::Single | FederalStatus::Head
    ) {
        2050
    } else {
        1650
    };
    for person in core::iter::once(&p.taxpayer).chain(p.spouse.iter()) {
        standard += additional * (i64::from(senior(person.birth_date)) + i64::from(person.blind));
    }
    if separate && s.spouse_itemizes() {
        standard = 0;
    }
    let senior_count = core::iter::once(&p.taxpayer)
        .chain(p.spouse.iter())
        .filter(|v| senior(v.birth_date) && yes(v.valid_ssn))
        .count() as i64;
    let senior_ded = if separate {
        0
    } else {
        senior_count
            * positive(6000 - fraction(positive(agi - if j { 150000 } else { 75000 }), 6, 100))
    };
    let salt_cap = max(
        if separate { 5000 } else { 10000 },
        if separate { 20200 } else { 40400 }
            - fraction(
                positive(agi - if separate { 252500 } else { 505000 }),
                30,
                100,
            ),
    );
    let salt = min(dollars(ded.state_local_taxes()), salt_cap);
    require(
        &mut r,
        ded.mortgage_interest() == 0 || yes(ded.mortgage_within_limit()),
        "Deductions",
        "Mortgage interest requires qualifying acquisition debt within the supported limit.",
    );
    require(
        &mut r,
        ded.cash_charity() == 0 || yes(ded.cash_charity_eligible()),
        "Deductions",
        "Confirm eligible charitable organizations and substantiation.",
    );
    require(
        &mut r,
        ded.cash_charity() <= positive(agi) * 60,
        "Deductions",
        "Charity above the AGI ceiling needs the carryover/floor coordination worksheet.",
    );
    // The same qualifying classroom expense is deducted above the line once; only its
    // remainder enters the new 2026 itemized educator deduction (P.L.119-21 section 70110).
    let educator_itemized = positive(dollars(a.educator_taxpayer()) - 350)
        + if j {
            positive(dollars(a.educator_spouse()) - 350)
        } else {
            0
        };
    let itemized = educator_itemized
        + positive(dollars(ded.medical()) - fraction(positive(agi), 75, 1000))
        + salt
        + dollars(ded.mortgage_interest())
        + positive(dollars(ded.cash_charity()) - fraction(positive(agi), 5, 1000));
    let cash_standard = if separate && s.spouse_itemizes() {
        0
    } else {
        min(dollars(ded.cash_charity()), if j { 2000 } else { 1000 })
    };
    // High-income itemized/QBI/AMT coordination is explicitly screened below before publishing a result.
    let use_itemized = match ded.choice() {
        DeductionChoice::Itemized => true,
        DeductionChoice::Standard => false,
        DeductionChoice::Automatic => itemized > standard + cash_standard,
    };
    require(
        &mut r,
        !(separate && s.spouse_itemizes() && ded.choice() == DeductionChoice::Standard),
        "Deductions",
        "You must itemize when a separately filing spouse itemizes.",
    );
    let deduction = if use_itemized {
        itemized
    } else {
        standard + cash_standard
    };
    let mut qualified_tips = 0;
    let mut qualified_overtime = 0;
    for (i, person) in [Some(&p.taxpayer), p.spouse.as_ref()]
        .into_iter()
        .enumerate()
    {
        if person.is_some_and(|p| yes(p.valid_ssn)) {
            qualified_tips += t.tips[i];
            qualified_overtime += t.overtime[i];
        }
    }
    for v in &ded.vehicle_loans {
        require(
            &mut r,
            v.interest == 0 || yes(v.eligible),
            "Vehicle interest",
            "Confirm eligible personal purchase debt and vehicle requirements before claiming interest.",
        );
    }
    let additional = additional_deductions(
        agi,
        s.filing_status(),
        dollars(qualified_tips),
        dollars(qualified_overtime),
        dollars(ded.vehicle_loans.iter().map(|v| v.interest).sum()),
    );
    for (id, amount) in [
        "tips_deduction",
        "overtime_deduction",
        "vehicle_interest_deduction",
    ]
    .into_iter()
    .zip(additional)
    {
        line(
            &mut r,
            id,
            amount,
            &["income", "deductions.vehicle_loans", "agi", "people"],
            "https://www.irs.gov/pub/irs-pdf/f1040s1a.pdf",
            "2026 Schedule 1-A deduction after claimant eligibility, annual limits and income phaseout. Does not reduce AGI or payroll tax.",
        );
    }
    let before_qbi = positive(agi - deduction - senior_ded - additional.iter().sum::<i64>());
    let qbi = positive(business - half_se);
    let qbi_threshold = if j {
        403500
    } else if separate {
        201775
    } else {
        201750
    };
    require(
        &mut r,
        qbi == 0 || before_qbi <= qbi_threshold,
        "Business",
        "QBI above the threshold needs wage/property/SSTB phase-in worksheets.",
    );
    let qbi_ded = if qbi > 0 {
        max(
            min(
                fraction(qbi, 20, 100),
                fraction(positive(before_qbi - pref), 20, 100),
            ),
            if qbi >= 1000 { 400 } else { 0 },
        )
    } else {
        0
    };
    let taxable = positive(before_qbi - qbi_ded);
    let income_tax = preferential_tax(taxable, pref, s.filing_status());
    // A conservative AMT upper bound proves zero AMT without silently ignoring it. It deliberately
    // adds back all deductions; any candidate AMT case requires the full worksheet before completion.
    let exemption = if joint_rates(s.filing_status()) {
        140200
    } else if separate {
        70100
    } else {
        90100
    };
    let amt_base = positive(agi - qbi_ded);
    let mut amt_exemption = positive(
        exemption
            - fraction(
                positive(
                    amt_base
                        - if joint_rates(s.filing_status()) {
                            1000000
                        } else {
                            500000
                        },
                ),
                1,
                2,
            ),
    );
    if !j && taxpayer_age < 24 {
        amt_exemption = min(amt_exemption, positive(earned[0]));
    }
    let amt_income = positive(amt_base - amt_exemption);
    let amt_break = if separate { 122250 } else { 244500 };
    let amt_upper =
        dollars(min(amt_income, amt_break) * 26 + positive(amt_income - amt_break) * 28);
    require(
        &mut r,
        amt_upper <= income_tax,
        "Alternative minimum tax",
        "Income and deductions may require Form 6251; the full AMT worksheet is not yet supported.",
    );
    require(
        &mut r,
        // Section 68 tests taxable income with itemized deductions added BACK.
        !use_itemized || taxable + itemized <= brackets(s.filing_status())[5],
        "Deductions",
        "The high-income itemized deduction limitation needs an additional worksheet.",
    );
    line(
        &mut r,
        "deduction",
        deduction,
        &["agi", "deductions"],
        LAW,
        if use_itemized {
            "Itemized medical, capped SALT, mortgage interest, cash gifts after the 0.5% AGI floor and remaining qualifying educator expenses."
        } else {
            "Standard deduction including age/blindness additions and eligible non-itemizer cash gifts."
        },
    );
    line(
        &mut r,
        "senior_deduction",
        senior_ded,
        &["agi", "people"],
        LAW,
        "Eligible SSN holders age 65 or older: phased deduction per person; unavailable to married separate filers.",
    );
    line(
        &mut r,
        "qbi_deduction",
        qbi_ded,
        &["income.business", "self_employment_tax", "agi", "deduction"],
        LAW,
        "Supported below-threshold QBI deduction with the 2026 active-business minimum.",
    );
    line(
        &mut r,
        "taxable_income",
        taxable,
        &[
            "agi",
            "deduction",
            "senior_deduction",
            "tips_deduction",
            "overtime_deduction",
            "vehicle_interest_deduction",
            "qbi_deduction",
        ],
        "https://www.irs.gov/publications/p505",
        "Taxable income is floored at zero.",
    );
    line(
        &mut r,
        "income_tax",
        income_tax,
        &["taxable_income", "income.dividends", "capital_gain"],
        ANNUAL,
        "Ordinary 2026 rate schedules with qualified-dividend and net long-term-gain stacking.",
    );
    // Education credits are ordered before the child credit, following Schedule 8812's credit limit.
    let education_factor = min(
        positive(if j { 180000 } else { 90000 } - agi),
        if j { 20000 } else { 10000 },
    );
    let mut aotc = 0;
    let mut llc_expenses = 0;
    for (index, e) in c.education.iter().enumerate() {
        require(
            &mut r,
            yes(e.eligible_student)
                && (e.method() != EducationMethod::AmericanOpportunity
                    || yes(e.aotc_requirements())),
            "Education",
            "Confirm student, expense and credit-specific eligibility.",
        );
        require(
            &mut r,
            !c.education[..index]
                .iter()
                .any(|prior| prior.student == e.student),
            "Education",
            "Use one credit entry per student; do not duplicate expenses across credits.",
        );
        let student = c.students.get(e.student as usize);
        require(
            &mut r,
            student.is_some_and(|v| {
                v.id == -1 || (v.id == -2 && j) || family.dependent_ids.contains(&v.id)
            }),
            "Education",
            "The selected student must be a filer or an eligible dependent on this return.",
        );
        let expense = dollars(e.expenses);
        if e.method() == EducationMethod::AmericanOpportunity {
            aotc += min(expense, 2000) + fraction(min(positive(expense - 2000), 2000), 1, 4);
        } else {
            llc_expenses += expense;
        }
    }
    aotc = if separate {
        0
    } else {
        fraction(aotc, education_factor, if j { 20000 } else { 10000 })
    };
    let llc = if separate {
        0
    } else {
        fraction(
            min(llc_expenses, 10000) * 20,
            education_factor,
            if j { 2000000 } else { 1000000 },
        )
    };
    let age = age(p.taxpayer.birth_date);
    let no_refund = !j
        && p.taxpayer.parent_alive
        && (age < 18
            || ((age == 18 || (age < 24 && p.taxpayer.full_time_student))
                && p.taxpayer.support_share == SupportShare::LessThanHalf));
    let education_refund = if no_refund {
        0
    } else {
        fraction(aotc, 40, 100)
    };
    let education_nonrefund = aotc - education_refund + llc;
    let mut care_people = family.care_people;
    let mut care_cost = dollars(family.care_expenses + c.care_expenses());
    if j && p.spouse.as_ref().is_some_and(|v| v.incapable_self_care) {
        care_people += 1;
    }
    if c.care_expenses() > 0 {
        require(
            &mut r,
            j && p.spouse.as_ref().is_some_and(|v| v.incapable_self_care),
            "Care",
            "Spouse care requires a disabled spouse on a joint return.",
        );
    }
    require(
        &mut r,
        care_cost == 0 || yes(c.care_eligible()),
        "Care",
        "Confirm eligible provider, residency and work-related care requirements.",
    );
    require(
        &mut r,
        c.care_benefits() == 0,
        "Care",
        "Employer dependent-care benefits require the exclusion and credit coordination worksheet.",
    );
    // Deemed earned income for full-time students/incapable spouses needs monthly facts.
    require(
        &mut r,
        care_cost == 0 || !j || (earned[0] > 0 && earned[1] > 0),
        "Care",
        "Care with a non-earning spouse requires monthly student/self-care earned-income rules.",
    );
    care_cost = min(
        care_cost,
        if care_people >= 2 {
            6000
        } else if care_people == 1 {
            3000
        } else {
            0
        },
    );
    care_cost = min(care_cost, positive(earned[0]));
    if j {
        care_cost = min(care_cost, positive(earned[1]));
    }
    require(
        &mut r,
        !(separate && care_cost > 0 && s.lived_apart_last_six_months()),
        "Care",
        "Separated-spouse care eligibility needs an additional worksheet.",
    );
    let care = if separate {
        0
    } else {
        fraction(care_cost, care_rate(agi, j), 100)
    };
    let prior_credits = min(income_tax, education_nonrefund + care);
    let saver = match c.saver.as_ref() {
        Some(facts) => {
            let mut combined = facts.clone();
            if combined.taxpayer_contributions == 0 && combined.spouse_contributions.amount() == 0 {
                combined.eligible_contributions = Answer::Yes;
            }
            // The same $2,000 per-person base covers IRA and workplace contributions.
            combined.taxpayer_contributions += ira.contributions[0];
            combined.spouse_contributions =
                (combined.spouse_contributions.amount() + ira.contributions[1]).into();
            match saver_potential(&combined, p, s.filing_status(), agi) {
                Ok(potential) => min(income_tax - prior_credits, potential),
                Err(message) => {
                    issue(&mut r, "Saver’s Credit", message);
                    0
                }
            }
        }
        None => 0,
    };
    let ira_saver_possible = saver_rate(agi, s.filing_status()) > 0
        && [Some(&p.taxpayer), p.spouse.as_ref()]
            .iter()
            .enumerate()
            .any(|(i, person)| {
                ira.contributions[i] > 0
                    && person.is_some_and(|v| {
                        v.birth_date <= 20090101
                            && !v.full_time_student
                            && v.claimable == Answer::No
                    })
            });
    require(
        &mut r,
        !ira_saver_possible || c.saver.is_some(),
        "Saver’s Credit",
        "IRA contributions may qualify for the Saver’s Credit. Complete its distribution-history worksheet in Credits.",
    );
    // Form 8880 follows care/education; Schedule 8812 also subtracts the allowed saver credit.
    // Applying it after CTC would understate ACTC when the nonrefundable child credit shrinks.
    let remaining_tax = income_tax - prior_credits - saver;
    let child_potential = positive(
        family.ctc * 2200 + family.odc * 500
            - ceiling(agi - if j { 400000 } else { 200000 }, 1000) * 50,
    );
    let child_nonrefund = min(remaining_tax, child_potential);
    let investment = interest + exempt + dividends + positive(short + long);
    let valid_eic_ssn =
        yes(p.taxpayer.valid_ssn) && p.spouse.as_ref().is_none_or(|v| yes(v.valid_ssn));
    let eic_age = core::iter::once(&p.taxpayer)
        .chain(p.spouse.iter())
        .any(|v| v.birth_date > 19611231 && v.birth_date <= 20020101);
    let childless_home = yes(p.taxpayer.us_home_over_half_year)
        && p.spouse
            .as_ref()
            .is_none_or(|v| yes(v.us_home_over_half_year));
    let eic_possible = valid_eic_ssn
        && investment <= 12200
        && (family.eic_children > 0 || (eic_age && childless_home));
    let eic = if !separate && eic_possible {
        eic_amount(earned.iter().sum(), agi, family.eic_children, j)
    } else {
        0
    };
    require(
        &mut r,
        !(separate && s.lived_apart_last_six_months() && family.eic_children > 0 && agi < 62974),
        "Earned income credit",
        "Separated-spouse EITC eligibility requires an additional worksheet.",
    );
    let payroll = dollars(t.social_tax.iter().sum::<i64>() + t.medicare_tax.iter().sum::<i64>());
    let excess_social = (0..2)
        .map(|i| dollars(positive(t.social_tax[i] - 1_143_900)))
        .sum::<i64>();
    let medicare_threshold = if j {
        250000
    } else if separate {
        125000
    } else {
        200000
    };
    let medicare_wages = dollars(t.medicare_wages.iter().sum());
    let additional_medicare = fraction(
        positive(medicare_wages + net.iter().sum::<i64>() - medicare_threshold),
        9,
        1000,
    );
    let medicare_withheld = dollars(positive(
        t.medicare_tax.iter().sum::<i64>() - fraction(t.medicare_wages.iter().sum(), 145, 10000),
    ));
    let actc_base = min(
        positive(child_potential - child_nonrefund),
        family.ctc * 1700,
    );
    let earned_refund = fraction(positive(earned.iter().sum::<i64>() - 2500), 15, 100);
    // Three-child alternative uses actual Additional Medicare tax, replacing its withholding.
    let alternate =
        positive(payroll - medicare_withheld + additional_medicare + half_se - eic - excess_social);
    let actc = min(
        actc_base,
        if family.ctc >= 3 {
            max(earned_refund, alternate)
        } else {
            earned_refund
        },
    );
    let nii = positive(interest + dividends + capital);
    let niit = fraction(min(nii, positive(agi - medicare_threshold)), 38, 1000);
    let senior_possible = core::iter::once(&p.taxpayer)
        .chain(p.spouse.iter())
        .any(|v| senior(v.birth_date));
    require(
        &mut r,
        !(senior_possible
            && agi
                < if j {
                    25000
                } else if separate {
                    12500
                } else {
                    17500
                }),
        "Credits",
        "This income may qualify for the credit for the elderly; Schedule R is not yet supported.",
    );
    let total = remaining_tax - child_nonrefund + se_tax + additional_medicare + niit;
    let refundable = eic + actc + education_refund;
    let payments = dollars(t.withholding)
        + dollars(pay.estimated)
        + dollars(pay.extension)
        + excess_social
        + medicare_withheld;
    for (id, value, source, explanation) in [
        (
            "education_credit",
            min(income_tax, education_nonrefund),
            "https://www.irs.gov/instructions/i8863",
            "Nonrefundable education credits, subject to available tax.",
        ),
        (
            "care_credit",
            min(positive(income_tax - education_nonrefund), care),
            LAW,
            "2026 care percentage and expense/earned-income limits; nonrefundable.",
        ),
        (
            "saver_credit",
            saver,
            "https://www.irs.gov/pub/irs-pdf/f8880.pdf",
            "Retirement Saver’s Credit using 2026 income bands, per-filer eligibility and distribution lookback; limited by tax after education and care credits.",
        ),
        (
            "child_credit",
            child_nonrefund,
            "https://www.irs.gov/instructions/i1040s8",
            "Child and other-dependent credits after MAGI phaseout and prior nonrefundable credits.",
        ),
        (
            "earned_income_credit",
            eic,
            ANNUAL,
            "2026 continuous phase-in/out estimate; final filing EIC table is not implemented.",
        ),
        (
            "additional_child_credit",
            actc,
            "https://www.irs.gov/instructions/i1040s8",
            "Unused credit subject to child count, earned-income and three-child payroll alternative limits.",
        ),
        (
            "refundable_education_credit",
            education_refund,
            "https://www.irs.gov/instructions/i8863",
            "American opportunity refundable portion using the taxpayer's age/support eligibility.",
        ),
        (
            "additional_medicare_tax",
            additional_medicare,
            "https://www.irs.gov/instructions/i8959",
            "Additional Medicare tax across wages and self-employment earnings.",
        ),
        (
            "net_investment_income_tax",
            niit,
            "https://www.irs.gov/instructions/i8960",
            "3.8% of supported net investment income or MAGI excess, whichever is smaller.",
        ),
        (
            "payments",
            payments,
            "https://www.irs.gov/publications/p505",
            "Income tax withholding, estimated/extension payments and eligible excess payroll withholding.",
        ),
        (
            "total_tax",
            total,
            "https://www.irs.gov/publications/p505",
            "Income tax after nonrefundable credits plus self-employment and additional taxes.",
        ),
    ] {
        line(
            &mut r,
            id,
            value,
            &["agi", "income_tax", "credits", "income", "payments"],
            source,
            explanation,
        );
    }
    if r.issues.is_empty() {
        r.state = ResultState::EstimatedForSupportedScope;
        r.tax = Some(total * 100);
        r.refundable_credits = Some(refundable * 100);
        r.payments = Some(payments * 100);
        r.refund = Some(positive(payments + refundable - total) * 100);
        r.balance = Some(positive(total - payments - refundable) * 100);
    } else {
        r.state = ResultState::Unsupported;
        r.lines.clear();
        r.breakdown.clear();
    }
    r
}

// Persisted selector positions belong to their roster snapshot, not today's person order.
// Rebuild editor choices by identity and retain only referenced missing people. This keeps
// incomplete relationships repairable without growing a history of every deleted person.
pub(crate) fn students(d: &FederalDraft) -> Vec<FederalStudent> {
    let mut current = vec![FederalStudent {
        id: -1,
        label: "You".into(),
    }];
    if d.people.as_ref().is_some_and(|p| p.spouse.is_some()) {
        current.push(FederalStudent {
            id: -2,
            label: "Your spouse".into(),
        });
    }
    current.extend(d.dependents.iter().map(|v| FederalStudent {
        id: v.id,
        label: v.label.clone(),
    }));
    if let Some(credits) = &d.credits {
        for entry in &credits.education {
            if let Some(old) = credits.students.get(entry.student as usize)
                && !current.iter().any(|p| p.id == old.id)
            {
                current.push(FederalStudent {
                    id: old.id,
                    label: alloc::format!("{} — no longer on this return", old.label),
                });
            }
        }
    }
    current
}
pub(crate) fn editable_credits(d: &FederalDraft, credits: &FederalCredits) -> FederalCredits {
    let mut result = credits.clone();
    result.students = students(d);
    for entry in &mut result.education {
        let id = credits.students[entry.student as usize].id;
        entry.student = result
            .students
            .iter()
            .position(|p| p.id == id)
            .expect("referenced student is retained") as u32;
    }
    result
}
pub(crate) fn student_present(d: &FederalDraft, id: i64) -> bool {
    id == -1
        || (id == -2 && d.people.as_ref().is_some_and(|p| p.spouse.is_some()))
        || d.dependents.iter().any(|v| v.id == id)
}
