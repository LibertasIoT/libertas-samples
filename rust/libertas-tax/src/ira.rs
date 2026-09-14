// 2026 limits: https://www.irs.gov/pub/irs-drop/n-25-67.pdf
// Worksheet ownership: https://www.irs.gov/publications/p590a (1-2, 2-2 and Appendix B).
use crate::federal_rules::dollars;
use crate::*;

#[derive(Default)]
pub(crate) struct IraResult {
    pub deduction: i64,
    pub nondeductible: [i64; 2],
    pub contributions: [i64; 2], // cents, for the shared Saver's Credit base
}

// Partial IRA limits round UP to $10 with a $200 minimum, but vanish at the endpoint.
// This differs from ordinary tax-line rounding and from deductions reduced in $1,000 steps.
pub(crate) fn phase_limit(cap: i64, magi: i64, start: i64, width: i64) -> i64 {
    if magi <= start {
        return cap;
    }
    if magi >= start + width {
        return 0;
    }
    let numerator = cap * (start + width - magi);
    ((numerator + width * 10 - 1) / (width * 10) * 10)
        .max(200)
        .min(cap)
}

pub(crate) fn calculate(
    f: &FederalIras,
    p: &FederalPeople,
    s: &FederalSetup,
    compensation: [i64; 2],
    traditional_magi: i64,
) -> Result<IraResult, &'static str> {
    let joint = s.filing_status() == FederalStatus::Joint;
    if f.regular_contributions != Answer::Yes {
        return Err(
            "Confirm regular timely IRA contributions and the absence of special basis, rollover, excess or compensation treatment.",
        );
    }
    if joint != f.spouse.is_some() || joint != p.spouse.is_some() {
        return Err(
            "Complete both IRA coverage records for a joint return; remove spouse IRA facts for any other filing status.",
        );
    }
    let facts = [Some(&f.taxpayer), f.spouse.as_ref()];
    let people = [Some(&p.taxpayer), p.spouse.as_ref()];
    let separate_together =
        s.filing_status() == FederalStatus::Separate && !s.lived_apart_all_year();
    if separate_together != matches!(f.spouse, IraSpouse::SeparateTogether { .. }) {
        return Err(
            "Choose the IRA spouse situation matching filing status and whether you lived together during the year.",
        );
    }
    let mut result = IraResult::default();
    for (i, fact) in facts.iter().enumerate() {
        if let Some(v) = fact {
            result.contributions[i] = v.traditional + v.roth;
        }
    }
    for i in 0..2 {
        let (Some(v), Some(person)) = (facts[i], people[i]) else {
            continue;
        };
        if result.contributions[i] == 0 {
            continue;
        }
        let cap = if person.birth_date <= 19770101 {
            8600
        } else {
            7500
        };
        let available = if joint && compensation[i] < compensation[1 - i] {
            (compensation.iter().sum::<i64>() * 100 - result.contributions[1 - i]).max(0)
        } else {
            compensation[i].max(0) * 100
        };
        if result.contributions[i] > (cap * 100).min(available) {
            return Err(
                "IRA contributions exceed this filer's age-based or available compensation limit; excess-contribution correction is outside this estimate.",
            );
        }
        if v.traditional == 0 {
            continue;
        }
        if v.covered_at_work == Answer::NotSure {
            return Err(
                "Confirm workplace retirement-plan coverage before calculating the traditional IRA deduction.",
            );
        }
        let spouse_covered = if joint {
            facts[1 - i].unwrap().covered_at_work
        } else if separate_together {
            match f.spouse {
                IraSpouse::SeparateTogether { covered_at_work } => covered_at_work,
                _ => Answer::No,
            }
        } else {
            Answer::No
        };
        if v.covered_at_work == Answer::No && spouse_covered == Answer::NotSure {
            return Err(
                "Confirm the spouse's workplace coverage; it can change the IRA deduction even when the spouse contributed nothing.",
            );
        }
        let covered = v.covered_at_work == Answer::Yes;
        let limit = if separate_together && (covered || spouse_covered == Answer::Yes) {
            phase_limit(cap, traditional_magi, 0, 10000)
        } else if covered {
            if matches!(
                s.filing_status(),
                FederalStatus::Joint | FederalStatus::Surviving
            ) {
                phase_limit(cap, traditional_magi, 129000, 20000)
            } else {
                phase_limit(cap, traditional_magi, 81000, 10000)
            }
        } else if joint && spouse_covered == Answer::Yes {
            phase_limit(cap, traditional_magi, 242000, 10000)
        } else {
            cap
        };
        let deduction = dollars(v.traditional).min(limit).min(dollars(available));
        result.deduction += deduction;
        result.nondeductible[i] = dollars(v.traditional) - deduction;
    }
    Ok(result)
}

pub(crate) fn validate_roth(
    f: &FederalIras,
    p: &FederalPeople,
    s: &FederalSetup,
    compensation: [i64; 2],
    magi: i64,
) -> Result<(), &'static str> {
    let joint = s.filing_status() == FederalStatus::Joint;
    // The engine still checks Roth coverage after reporting an invalid traditional worksheet.
    // Missing spouse facts must remain an ordinary coverage error, never an unwrap panic.
    if joint != f.spouse.is_some() || joint != p.spouse.is_some() {
        return Err(
            "Complete both IRA coverage records for a joint return; remove spouse IRA facts for any other filing status.",
        );
    }
    let (start, width) = if matches!(
        s.filing_status(),
        FederalStatus::Joint | FederalStatus::Surviving
    ) {
        (242000, 10000)
    } else if s.filing_status() == FederalStatus::Separate && !s.lived_apart_all_year() {
        (0, 10000)
    } else {
        (153000, 15000)
    };
    let facts = [Some(&f.taxpayer), f.spouse.as_ref()];
    let people = [Some(&p.taxpayer), p.spouse.as_ref()];
    for i in 0..2 {
        let (Some(v), Some(person)) = (facts[i], people[i]) else {
            continue;
        };
        if v.roth == 0 {
            continue;
        }
        let available = if joint && compensation[i] < compensation[1 - i] {
            let other = facts[1 - i].unwrap();
            (compensation.iter().sum::<i64>() * 100 - other.traditional - other.roth).max(0)
        } else {
            compensation[i].max(0) * 100
        };
        let cap = (if person.birth_date <= 19770101 {
            8600
        } else {
            7500
        })
        .min(dollars(available));
        // Worksheet 2-2 compares the phased total limit with the unphased limit less
        // traditional contributions. Do not subtract traditional contributions twice.
        let limit =
            (phase_limit(cap, magi, start, width) * 100).min((cap * 100 - v.traditional).max(0));
        if v.roth > limit {
            return Err(
                "Roth IRA contributions exceed the 2026 income-based limit; correct or recharacterize the excess before using this estimate.",
            );
        }
    }
    Ok(())
}
