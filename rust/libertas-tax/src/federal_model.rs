use crate::{AmountBasis, Answer, CalculationLine, Owner, ResultState};
use alloc::{string::String, vec::Vec};
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Federal filing status
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum FederalStatus {
    /// Single
    Single,
    /// Married filing jointly
    Joint,
    /// Married filing separately
    Separate,
    /// Head of household
    Head,
    /// Qualifying surviving spouse
    Surviving,
}

/// Filing status to evaluate
/// Select a status; the backend checks eligibility from the household facts.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
#[libertas_ui_header]
pub enum FilingChoice {
    /// Single
    Single,
    /// Married filing jointly
    Joint,
    /// Married filing separately
    Separate {
        /// Your spouse will itemize deductions
        spouse_itemizes: bool,
    },
    /// Head of household
    Head,
    /// Qualifying surviving spouse
    Surviving,
}

/// Living arrangements with your spouse during 2026
/// Select the period that describes when your spouse lived apart from you.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum SpouseLiving {
    /// Lived together during some of the last six months
    Together,
    /// Apart for the last six months, but not all year
    LastSixMonths,
    /// Apart for the entire year
    AllYear,
}

/// Marital situation at the end of 2026
/// Only the selected situation asks its applicable spouse questions.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
#[libertas_ui_header]
pub enum MaritalState {
    /// Unmarried, divorced, or legally separated
    Unmarried,
    /// Married
    Married {
        /// Living arrangements with your spouse
        #[libertas_ui_header]
        living: SpouseLiving,
    },
    /// Spouse died in 2026; not remarried
    Widowed2026,
    /// Spouse died in 2025; not remarried
    Widowed2025 {
        /// Could file jointly for your spouse’s year of death
        could_file_joint: Answer,
    },
    /// Spouse died in 2024; not remarried
    Widowed2024 {
        /// Could file jointly for your spouse’s year of death
        could_file_joint: Answer,
    },
}

/// Relationship to you
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum Relationship {
    /// Child, adopted child or stepchild
    Child,
    /// Eligible foster child placed by an authorized agency or court
    Foster,
    /// Grandchild or other descendant
    Grandchild,
    /// Sibling or stepsibling
    Sibling,
    /// Niece or nephew
    NieceNephew,
    /// Biological or adoptive parent
    Parent,
    /// Grandparent, stepparent, aunt, uncle, or qualifying in-law (not a cousin)
    OtherRelative,
    /// Unrelated household member
    Unrelated,
}

/// Holding period
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum GainTerm {
    /// Held one year or less
    Short,
    /// Held more than one year
    Long,
}

/// Education credit to evaluate
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum EducationMethod {
    /// American opportunity credit
    AmericanOpportunity,
    /// Lifetime learning credit
    LifetimeLearning,
}

/// Deduction choice
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum DeductionChoice {
    /// Use the supported choice with the lower total tax
    Automatic,
    /// Standard deduction
    Standard,
    /// Itemized deductions
    Itemized,
}

/// HSA coverage
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum HsaCoverage {
    /// Self-only eligible coverage all year
    SelfOnly,
    /// Family eligible coverage all year
    Family,
}

/// Filer eligibility
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalPerson {
    /// Fictional full name
    /// Use synthetic names in this prototype.
    #[libertas_size(min = 1, max = 160)]
    pub name: String,
    /// Fictional date of birth
    #[libertas_date_only]
    pub birth_date: u32,
    /// Legally blind at year end
    pub blind: bool,
    /// U.S. citizen or resident for all of 2026
    pub full_year_resident: Answer,
    /// Alive at the end of 2026
    pub alive_at_year_end: bool,
    /// This filer may be another taxpayer’s qualifying child for EITC, even if not their dependent
    pub another_eitc_child: Answer,
    /// Someone else can claim this filer as a dependent
    pub claimable: Answer,
    /// Has a valid employment-authorized SSN issued by the return due date; do not enter the number
    pub valid_ssn: Answer,
    /// Has a valid SSN or ITIN issued by the return due date; do not enter it
    pub valid_tin: Answer,
    /// Main home was in the United States for more than half of 2026
    pub us_home_over_half_year: Answer,
    /// Full-time student during part of at least five calendar months
    /// Include qualifying full-time on-farm training; exclude on-the-job, correspondence and internet-only schools.
    pub full_time_student: bool,
    /// Permanently and totally disabled
    pub disabled: bool,
    /// Physically or mentally incapable of self-care
    pub incapable_self_care: bool,
    /// Earned income compared with this filer’s total support
    pub support_share: SupportShare,
    /// At least one parent was alive at year end
    pub parent_alive: bool,
}

/// About this return
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalSetup {
    /// Fictional return label
    #[libertas_size(min = 1, max = 80)]
    #[libertas_ui_header]
    pub label: String,
    /// Full-year actual amounts or your own full-year estimates
    pub basis: AmountBasis,
    /// Filing status to evaluate
    pub status: FilingChoice,
    /// Marital situation
    pub marital_state: MaritalState,
    /// Paid more than half the cost of maintaining the qualifying home
    pub paid_over_half_home: bool,
    /// Community-property allocation or income splitting applies
    pub community_property: Answer,
}

impl FederalSetup {
    pub(crate) fn filing_status(&self) -> FederalStatus {
        match self.status {
            FilingChoice::Single => FederalStatus::Single,
            FilingChoice::Joint => FederalStatus::Joint,
            FilingChoice::Separate { .. } => FederalStatus::Separate,
            FilingChoice::Head => FederalStatus::Head,
            FilingChoice::Surviving => FederalStatus::Surviving,
        }
    }

    pub(crate) fn spouse_itemizes(&self) -> bool {
        matches!(
            self.status,
            FilingChoice::Separate {
                spouse_itemizes: true
            }
        )
    }

    pub(crate) fn lived_apart_all_year(&self) -> bool {
        matches!(
            self.marital_state,
            MaritalState::Married {
                living: SpouseLiving::AllYear
            }
        )
    }

    pub(crate) fn lived_apart_last_six_months(&self) -> bool {
        matches!(
            self.marital_state,
            MaritalState::Married {
                living: SpouseLiving::AllYear | SpouseLiving::LastSixMonths
            }
        )
    }

    pub(crate) fn could_file_joint_death_year(&self) -> Answer {
        match self.marital_state {
            MaritalState::Widowed2025 { could_file_joint }
            | MaritalState::Widowed2024 { could_file_joint } => could_file_joint,
            _ => Answer::No,
        }
    }
}

/// About the filers
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalPeople {
    /// You
    pub taxpayer: FederalPerson,
    /// Spouse — required only for a joint return
    #[libertas_constrained_by("$.allowed_spouse")]
    pub spouse: SpouseFiler,
}

/// Children and people you support
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalDependent {
    /// Accepted record identity
    #[libertas_hidden]
    #[libertas_read_only]
    #[libertas_default(0)]
    pub id: i64,
    /// Fictional dependent label
    #[libertas_size(min = 1, max = 80)]
    #[libertas_ui_header]
    pub label: String,
    /// Fictional birth date
    #[libertas_date_only]
    pub birth_date: u32,
    /// Relationship
    pub relationship: Relationship,
    /// Time in your home during 2026
    pub residency: DependentResidency,
    /// Main home with you was in the United States for more than half the year
    pub us_home_over_half_year: Answer,
    /// Full-time student for at least five months
    pub student: bool,
    /// Permanently and totally disabled
    pub disabled: bool,
    /// Physically or mentally incapable of self-care
    pub incapable_self_care: bool,
    /// Provided more than half their own support
    pub own_support_over_half: bool,
    /// You provided more than half their total support
    pub your_support_over_half: bool,
    /// Work-related care expenses for this dependent
    pub care: DependentCare,
    /// Person’s gross income for the qualifying-relative test
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub gross_income: i64,
    /// U.S. citizen, U.S. national or U.S. resident alien
    pub us_citizen_resident: Answer,
    /// Valid employment-authorized SSN issued by the due date; do not enter it
    pub valid_ssn: Answer,
    /// Valid SSN, ITIN or ATIN issued by the due date; do not enter it
    pub valid_tin: Answer,
    /// Filed a joint return other than only to obtain a refund
    pub joint_return_nonrefund: bool,
    /// Another person may claim this child or a custody/tiebreaker agreement applies
    pub competing_claim: Answer,
}

/// W-2 wages
/// Enter annual totals once per employer. Combine multiple W-2s from that employer; use corrected amounts instead of duplicating the original.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalWage {
    /// Box 1 wages
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub wages: i64,
    /// Box 2 federal income tax withheld
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub withholding: i64,
    /// Box 3 Social Security wages
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub social_wages: i64,
    /// Box 7 Social Security tips — do not add these to box 1 again
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    #[libertas_default(0)]
    pub social_tips: i64,
    /// Qualified tips and overtime deductions
    pub work_deductions: WorkDeductionChoice,
    /// Box 4 Social Security tax withheld
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub social_tax: i64,
    /// Box 5 Medicare wages
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub medicare_wages: i64,
    /// Box 6 Medicare tax withheld
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub medicare_tax: i64,
    /// Unreported/allocated tips, railroad wages, stock compensation, dependent-care benefits or another special W-2 treatment applies
    pub special_boxes: Answer,
}

/// Interest
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalInterest {
    /// Ordinary taxable bank interest
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub taxable: i64,
    /// Tax-exempt interest, excluding private-activity bond interest
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub exempt: i64,
    /// Federal income tax withheld
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub withholding: i64,
    /// Bond discounts, premium amortization, private-activity bonds, foreign interest or savings-bond exclusions apply
    pub special_treatment: Answer,
}

/// Dividends
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalDividend {
    /// Ordinary dividends, including qualified dividends
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub ordinary: i64,
    /// Qualified dividends meeting the holding-period rules
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub qualified: i64,
    /// Ordinary long-term capital gain distributions
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub capital_distributions: i64,
    /// Federal income tax withheld
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub withholding: i64,
    /// Collectibles, unrecaptured section 1250 gains, foreign tax, REIT/PTP distributions or another special treatment applies
    pub special_treatment: Answer,
}

/// Investment sale
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalSale {
    /// Holding period
    pub term: GainTerm,
    /// Sale proceeds
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub proceeds: i64,
    /// Adjusted tax basis, including known commissions
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub basis: i64,
    /// Disallowed wash-sale loss to add to gain or loss
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub wash_sale: i64,
    /// Unknown basis, stock compensation, options, collectibles, business property, home sale or another special treatment applies
    pub special_treatment: Answer,
}

/// Qualified tips and overtime on this W-2
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalWorkDeductions {
    /// Qualified tips to evaluate
    pub tips_claim: TipsClaim,
    /// Qualified overtime to evaluate
    pub overtime_claim: OvertimeClaim,
}

/// Personal vehicle loan interest
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalVehicleInterest {
    /// Fictional vehicle label
    #[libertas_ui_header]
    #[libertas_size(min = 1, max = 80)]
    pub label: String,
    /// Qualified 2026 personal-use interest paid on this loan
    /// Exclude principal, fees and interest deducted for business use or elsewhere.
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub interest: i64,
    /// Eligible new personal vehicle and purchase loan originated after 2024, secured by a first lien
    /// Original use began with you; eligible vehicle under 14,000 pounds assembled in the United States.
    /// No related-party loan or lease. Refinancing is limited to qualifying outstanding purchase debt.
    /// Retain VIN and lender records for eventual filing; enter no real identifiers in this prototype.
    pub eligible: Answer,
}

/// Retirement distribution
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalRetirement {
    /// Gross distribution
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub gross: i64,
    /// Taxable amount known from the statement and applicable rules
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub taxable: i64,
    /// Federal income tax withheld
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub withholding: i64,
    /// Normal pension or IRA distribution with known taxable amount and no early-distribution tax, rollover, QCD, inherited-account or basis calculation
    pub ordinary_distribution: Answer,
}

/// Social Security
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalSocial {
    /// SSA-1099 box 5 net benefits
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub benefits: i64,
    /// Federal income tax withheld
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub withholding: i64,
    /// Prior-year lump sums, benefit repayments exceeding receipts, railroad benefits or treaty treatment applies
    pub special_treatment: Answer,
}

/// Unemployment compensation
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalUnemployment {
    /// Taxable unemployment compensation
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub amount: i64,
    /// Federal income tax withheld
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub withholding: i64,
    /// Benefits were repaid or another special treatment applies
    pub repaid: Answer,
}

/// Sole-proprietor business
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalBusiness {
    /// Cash-basis gross receipts including reported 1099 income once
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub receipts: i64,
    /// Ordinary deductible business expenses, excluding personal and separately claimed deductions
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub expenses: i64,
    /// Actively and materially participated in this business
    pub material_participation: Answer,
    /// Inventory, depreciation, home office, employees, farming, partnership/S-corporation, foreign activity, prior losses or a special tax election applies
    pub special_treatment: Answer,
}

/// Income document
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub enum FederalIncome {
    /// W-2
    Wage {
        /// Document amounts and facts
        data: FederalWage,
    },
    /// Interest
    Interest {
        /// Document amounts and facts
        data: FederalInterest,
    },
    /// Dividends
    Dividend {
        /// Document amounts and facts
        data: FederalDividend,
    },
    /// Investment sale
    Sale {
        /// Document amounts and facts
        data: FederalSale,
    },
    /// Pension or IRA
    Retirement {
        /// Document amounts and facts
        data: FederalRetirement,
    },
    /// Social Security
    SocialSecurity {
        /// Document amounts and facts
        data: FederalSocial,
    },
    /// Unemployment
    Unemployment {
        /// Document amounts and facts
        data: FederalUnemployment,
    },
    /// Self-employment
    Business {
        /// Document amounts and facts
        data: FederalBusiness,
    },
}

/// Income source
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalIncomeEntry {
    /// Accepted record identity
    #[libertas_hidden]
    #[libertas_read_only]
    #[libertas_default(0)]
    pub id: i64,
    /// Fictional employer, payer or business label
    #[libertas_size(min = 1, max = 80)]
    #[libertas_ui_header]
    pub label: String,
    /// Income owner
    pub owner: Owner,
    /// Document type and facts
    pub income: FederalIncome,
}

/// HSA contribution
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalHsa {
    /// Account owner
    pub owner: Owner,
    /// Eligible coverage for the entire year
    pub coverage: HsaCoverage,
    /// Personal contributions not excluded from wages
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub personal: i64,
    /// Employer and payroll-excluded contributions
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub employer: i64,
    /// Distributions used entirely for qualified medical expenses
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub distributions: i64,
    /// Eligible all year with no disqualifying coverage, Medicare, excess contributions, last-month testing or nonmedical distributions
    pub fully_eligible: Answer,
}

/// Annual IRA contributions for one filer
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalIraContribution {
    /// Regular traditional IRA contributions designated for 2026
    /// Aggregate all accounts. Include planned contributions made by the filing deadline without extensions.
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub traditional: i64,
    /// Regular Roth IRA contributions designated for 2026
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub roth: i64,
    /// Covered by a retirement plan at work during 2026
    /// Check all employers and self-employment plans, even if no IRA contribution was made.
    pub covered_at_work: Answer,
}

/// IRA contributions
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalIras {
    /// Your contributions and workplace coverage
    pub taxpayer: FederalIraContribution,
    /// Spouse situation for the IRA worksheet
    pub spouse: IraSpouse,
    /// Regular timely contributions only; no rollovers, conversions, recharacterizations, excess carryovers, repayments or special compensation
    /// Compensation comes from the wages and supported business earnings entered in Income.
    /// No traditional IRA basis or distributions requiring basis allocation, and no 501(c)(18) plan deductions.
    /// The estimate takes the maximum allowed traditional deduction; choosing a smaller deduction is not supported.
    pub regular_contributions: Answer,
}

/// Adjustments to income
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalAdjustments {
    /// Student-loan interest to evaluate
    pub student_loan: StudentLoanClaim,
    /// Educator expenses to evaluate
    pub educators: EducatorClaim,
    /// Health savings accounts
    /// ----
    /// Hsa entry
    pub hsa: Vec<FederalHsa>,
    /// Traditional and Roth IRA contributions
    pub iras: IraChoice,
    /// Self-employed health insurance/retirement contributions, alimony, moving expenses or another adjustment applies
    pub other_adjustments: Answer,
}

/// Deductions
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalDeductions {
    /// Standard or itemized deduction
    pub choice: DeductionSelection,
    /// Cash charitable gifts to evaluate
    pub charity: CharityClaim,
    /// Noncash gifts, carryovers, casualty losses, investment interest, gambling losses, coaching-only or other expanded educator expenses not entered in Adjustments, or another deduction applies
    pub special_itemizing: Answer,
    /// Personal vehicle loan interest
    /// ----
    /// Vehicle loan
    #[libertas_size(max = 20)]
    pub vehicle_loans: Vec<FederalVehicleInterest>,
    /// Self-employed tips, overtime outside W-2 income, unreported tips or special vehicle-interest treatment applies
    pub special_work_deductions: Answer,
}

/// Education credit facts
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalEducation {
    /// Student
    #[libertas_enum_source("^^^.students")]
    #[libertas_ui_header]
    pub student: u32,
    /// Credit to evaluate for this student
    pub method: EducationChoice,
    /// Eligible expenses after scholarships, grants, refunds and amounts used for any other tax benefit
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub expenses: i64,
    /// Eligible institution, student and taxpayer; no double benefit or competing claim
    pub eligible_student: Answer,
}

/// Retirement distribution period
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum SaverDistributionYear {
    /// 2024
    PriorTwo,
    /// 2025
    PriorOne,
    /// 2026
    TaxYear,
    /// 2027, before the 2026 return due date including extensions
    FollowingYear,
}

/// Retirement distribution for the Saver’s Credit
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalSaverDistribution {
    /// Recipient
    #[libertas_ui_header]
    pub owner: Owner,
    /// When received
    pub year: SaverDistributionPeriod,
    /// Distribution amount that counts against the credit
    /// Include retirement-plan, traditional/Roth IRA and ABLE distributions. Exclude rollovers,
    /// conversions, deemed plan loans, timely returned or excess contributions and associated earnings,
    /// ESOP dividends, military pensions other than TSP, and nonspousal inherited-IRA distributions.
    /// Enter 2026 taxable income separately in Income; this entry only affects the credit.
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub amount: i64,
}

/// Saver’s Credit for retirement contributions
/// Optional: complete this if either filer made qualifying workplace or IRA contributions.
/// This page calculates a credit only; do not subtract contributions from W-2 box 1 wages again.
/// IRA contributions are included automatically from Adjustments. ABLE claims remain outside this worksheet.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalSaver {
    /// Your qualifying 2026 workplace retirement contributions
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub taxpayer_contributions: i64,
    /// Spouse’s qualifying workplace retirement contributions
    pub spouse_contributions: SpouseAmount,
    /// These are eligible employee contributions within the plan limits, after returned excess amounts
    /// Include elective deferrals (including designated Roth) to 401(k), 403(b), governmental 457(b),
    /// salary-reduction SEP/SIMPLE or TSP plans, or voluntary after-tax employee plan contributions.
    /// Exclude employer matches, mandatory employer-treated contributions, rollovers and IRA/ABLE contributions.
    /// Special 501(c)(18)(D) deduction cases remain outside this worksheet.
    pub eligible_contributions: Answer,
    /// Distributions in the lookback period
    /// List each recipient and year separately, including a spouse with no eligible contribution.
    /// ----
    /// Distribution
    #[libertas_size(max = 100)]
    pub distributions: Vec<FederalSaverDistribution>,
    /// All counting distributions from January 1, 2024 through the 2026 return due date, including extensions, are included
    /// For an estimate, include expected distributions through that date. Revisit this page if they change.
    /// Choose Not sure if eligibility, exclusions or the distribution history still need review.
    pub reviewed_distributions: Answer,
}

/// Other common credits
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalCredits {
    /// Students on this return
    /// ----
    /// Student
    #[libertas_read_only]
    #[libertas_copy_from("$.students")]
    pub students: Vec<FederalStudent>,
    /// Education students
    /// ----
    /// Education entry
    pub education: Vec<FederalEducation>,
    /// Work-related dependent-care credit to evaluate
    pub care: CareClaim,
    /// Saver’s Credit for workplace retirement contributions
    pub saver: SaverChoice,
    /// ABLE Saver’s Credit, adoption, foreign tax, Marketplace premium tax credit, business credits, prior credit disallowance/recapture or another credit applies
    pub special_credits: Answer,
}

/// Additional coverage checks
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalScreening {
    /// Foreign income/accounts, tax treaties, nonresident periods or territories apply
    pub foreign: Answer,
    /// Rentals, royalties, trusts, estates, K-1s, farming, carried interests or other income not entered applies
    pub property: Answer,
    /// Capital losses from earlier years, NOLs, AMT credits or other carryovers apply
    pub carryovers: Answer,
    /// AMT adjustments/preferences, disability retirement, retirement penalties, household employment, unreported tips or another special tax applies
    pub special_taxes: Answer,
    /// Any other federal income, deduction, credit, tax or uncertain tax treatment applies
    pub uncertain: Answer,
}

/// Federal payments
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalPayments {
    /// Estimated federal payments
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub estimated: i64,
    /// Extension payment
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub extension: i64,
}

/// Item needing review
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalIssue {
    /// Section
    pub section: String,
    /// Explanation
    pub message: String,
}

/// 2026 federal estimate
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalResult {
    /// Coverage status
    pub state: ResultState,
    /// Accepted revision
    pub revision: i64,
    /// Items needing review
    /// ----
    /// Issues entry
    pub issues: Vec<FederalIssue>,
    /// Estimate method and limitations
    pub method: String,
    /// Calculation provenance
    /// ----
    /// Calculation line
    #[libertas_hidden]
    pub lines: Vec<CalculationLine>,
    /// Calculation breakdown
    /// ----
    /// Lines entry
    pub breakdown: Vec<FederalAmount>,
    /// Total estimated federal tax
    #[libertas_money("USD", 2)]
    pub tax: Option<i64>,
    /// Refundable credits
    #[libertas_money("USD", 2)]
    pub refundable_credits: Option<i64>,
    /// Withholding and payments
    #[libertas_money("USD", 2)]
    pub payments: Option<i64>,
    /// Estimated refund before penalties, interest and offsets
    #[libertas_money("USD", 2)]
    pub refund: Option<i64>,
    /// Estimated balance before penalties and interest
    #[libertas_money("USD", 2)]
    pub balance: Option<i64>,
}

/// Accepted federal return
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalDraft {
    /// Tax year
    pub tax_year: i32,
    /// Next document or person identity
    pub next_id: i64,
    /// Return identity
    pub cookie: String,
    /// Accepted revision
    pub revision: i64,
    /// Return setup
    pub setup: Option<FederalSetup>,
    /// Filer facts
    pub people: Option<FederalPeople>,
    /// Accepted children and dependents
    pub dependents: Vec<FederalDependent>,
    /// Children and dependents reviewed
    pub dependents_complete: bool,
    /// Accepted income documents
    pub income: Vec<FederalIncomeEntry>,
    /// Income documents reviewed
    pub income_complete: bool,
    /// Accepted adjustments
    pub adjustments: Option<FederalAdjustments>,
    /// Accepted deductions
    pub deductions: Option<FederalDeductions>,
    /// Accepted common credits
    pub credits: Option<FederalCredits>,
    /// Accepted coverage review
    pub screening: Option<FederalScreening>,
    /// Accepted payments
    pub payments: Option<FederalPayments>,
    /// Review completed
    pub finished: bool,
}

/// Calculation amount
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalAmount {
    /// Calculation
    #[libertas_ui_header]
    pub label: String,
    /// Amount
    #[libertas_money("USD", 2)]
    pub amount: i64,
    /// How this amount was calculated
    pub explanation: String,
}

/// Student choice
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct FederalStudent {
    /// Person identity
    #[libertas_hidden]
    pub id: i64,
    /// Student
    #[libertas_ui_header]
    pub label: String,
}

/// Earned income share of support
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
pub enum SupportShare {
    /// Less than half
    LessThanHalf,
    /// Exactly half
    ExactlyHalf,
    /// More than half
    MoreThanHalf,
}

/// Work-related care expenses for this dependent
/// Choose Yes to enter the amounts and eligibility facts; No has no additional fields.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum DependentCare {
    /// No
    No,
    /// Yes
    Yes {
        /// Eligible work-related care costs for this person, after reimbursements
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        care_expenses: i64,
        /// Care costs include only periods before age 13, or while incapable of self-care
        care_period_eligible: Answer,
    },
}

impl FederalDependent {
    pub(crate) fn care_expenses(&self) -> i64 {
        match &self.care {
            DependentCare::No => 0,
            DependentCare::Yes { care_expenses, .. } => *care_expenses,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_care_expenses(&mut self, value: i64) {
        if matches!(self.care, DependentCare::No) {
            self.care = DependentCare::Yes {
                care_expenses: 0,
                care_period_eligible: Answer::No,
            };
        }
        if let DependentCare::Yes { care_expenses, .. } = &mut self.care {
            *care_expenses = value;
        }
    }
    pub(crate) fn care_period_eligible(&self) -> Answer {
        match &self.care {
            DependentCare::No => Answer::No,
            DependentCare::Yes {
                care_period_eligible,
                ..
            } => *care_period_eligible,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_care_period_eligible(&mut self, value: Answer) {
        if matches!(self.care, DependentCare::No) {
            self.care = DependentCare::Yes {
                care_expenses: 0,
                care_period_eligible: Answer::No,
            };
        }
        if let DependentCare::Yes {
            care_period_eligible,
            ..
        } = &mut self.care
        {
            *care_period_eligible = value;
        }
    }
}

/// Qualified tips to evaluate
/// Choose Yes to enter the amounts and eligibility facts; No has no additional fields.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum TipsClaim {
    /// No
    No,
    /// Yes
    Yes {
        /// Qualified tips reported by this employer for 2026
        /// Include only voluntary cash/charged or shared tips in an IRS-listed tipped occupation.
        /// Use the separately reported 2026 qualified amount; do not duplicate it as another income document.
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        tips: i64,
        /// occupation, reporting and applicable 2026 specified-service-business requirements are satisfied; choose Not sure when unresolved
        tips_eligible: Answer,
    },
}

impl FederalWorkDeductions {
    pub(crate) fn tips(&self) -> i64 {
        match &self.tips_claim {
            TipsClaim::No => 0,
            TipsClaim::Yes { tips, .. } => *tips,
        }
    }

    pub(crate) fn tips_eligible(&self) -> Answer {
        match &self.tips_claim {
            TipsClaim::No => Answer::No,
            TipsClaim::Yes { tips_eligible, .. } => *tips_eligible,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_tips_eligible(&mut self, value: Answer) {
        if matches!(self.tips_claim, TipsClaim::No) {
            self.tips_claim = TipsClaim::Yes {
                tips: 0,
                tips_eligible: Answer::No,
            };
        }
        if let TipsClaim::Yes { tips_eligible, .. } = &mut self.tips_claim {
            *tips_eligible = value;
        }
    }
}

/// Qualified overtime to evaluate
/// Choose Yes to enter the amounts and eligibility facts; No has no additional fields.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum OvertimeClaim {
    /// No
    No,
    /// Yes
    Yes {
        /// Qualified FLSA overtime premium reported by this employer for 2026
        /// Enter only the qualifying premium above the regular rate, not total overtime wages.
        /// The amount must already be included in this W-2's wages.
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        overtime: i64,
        /// Employee and premium qualify under federal FLSA rules and the 2026 reporting requirements; choose Not sure when unresolved
        overtime_eligible: Answer,
    },
}

impl FederalWorkDeductions {
    pub(crate) fn overtime(&self) -> i64 {
        match &self.overtime_claim {
            OvertimeClaim::No => 0,
            OvertimeClaim::Yes { overtime, .. } => *overtime,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_overtime(&mut self, value: i64) {
        if matches!(self.overtime_claim, OvertimeClaim::No) {
            self.overtime_claim = OvertimeClaim::Yes {
                overtime: 0,
                overtime_eligible: Answer::No,
            };
        }
        if let OvertimeClaim::Yes { overtime, .. } = &mut self.overtime_claim {
            *overtime = value;
        }
    }
    pub(crate) fn overtime_eligible(&self) -> Answer {
        match &self.overtime_claim {
            OvertimeClaim::No => Answer::No,
            OvertimeClaim::Yes {
                overtime_eligible, ..
            } => *overtime_eligible,
        }
    }
}

/// Student-loan interest to evaluate
/// Choose Yes to enter the amounts and eligibility facts; No has no additional fields.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum StudentLoanClaim {
    /// No
    No,
    /// Yes
    Yes {
        /// Qualified student-loan interest paid, excluding reimbursed amounts
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        student_loan_interest: i64,
        /// legally liable on an eligible education loan and not claimable as a dependent; choose Not sure when unresolved
        student_loan_eligible: Answer,
    },
}

impl FederalAdjustments {
    pub(crate) fn student_loan_interest(&self) -> i64 {
        match &self.student_loan {
            StudentLoanClaim::No => 0,
            StudentLoanClaim::Yes {
                student_loan_interest,
                ..
            } => *student_loan_interest,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_student_loan_interest(&mut self, value: i64) {
        if matches!(self.student_loan, StudentLoanClaim::No) {
            self.student_loan = StudentLoanClaim::Yes {
                student_loan_interest: 0,
                student_loan_eligible: Answer::No,
            };
        }
        if let StudentLoanClaim::Yes {
            student_loan_interest,
            ..
        } = &mut self.student_loan
        {
            *student_loan_interest = value;
        }
    }
    pub(crate) fn student_loan_eligible(&self) -> Answer {
        match &self.student_loan {
            StudentLoanClaim::No => Answer::No,
            StudentLoanClaim::Yes {
                student_loan_eligible,
                ..
            } => *student_loan_eligible,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_student_loan_eligible(&mut self, value: Answer) {
        if matches!(self.student_loan, StudentLoanClaim::No) {
            self.student_loan = StudentLoanClaim::Yes {
                student_loan_interest: 0,
                student_loan_eligible: Answer::No,
            };
        }
        if let StudentLoanClaim::Yes {
            student_loan_eligible,
            ..
        } = &mut self.student_loan
        {
            *student_loan_eligible = value;
        }
    }
}

/// Educator expenses to evaluate
/// Choose Yes to enter the amounts and eligibility facts; No has no additional fields.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum EducatorClaim {
    /// No
    No,
    /// Yes
    Yes {
        /// Your unreimbursed eligible educator expenses
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        educator_taxpayer: i64,
        /// Spouse’s unreimbursed eligible educator expenses
        educator_spouse: SpouseAmount,
        /// each claimant meets the classroom educator requirements; choose Not sure when unresolved
        /// K–12 teacher, instructor, counselor, principal or aide working at least 900 hours.
        /// Expenses are eligible professional development or classroom supplies, net of reimbursements and other tax benefits.
        /// Exclude nonathletic health/physical-education supplies and coaching-only expenses.
        /// The backend allocates the above-line limit and the remainder to itemized deductions.
        eligible_educators: Answer,
    },
}

impl FederalAdjustments {
    pub(crate) fn educator_taxpayer(&self) -> i64 {
        match &self.educators {
            EducatorClaim::No => 0,
            EducatorClaim::Yes {
                educator_taxpayer, ..
            } => *educator_taxpayer,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_educator_taxpayer(&mut self, value: i64) {
        if matches!(self.educators, EducatorClaim::No) {
            self.educators = EducatorClaim::Yes {
                educator_taxpayer: 0,
                educator_spouse: SpouseAmount::No,
                eligible_educators: Answer::No,
            };
        }
        if let EducatorClaim::Yes {
            educator_taxpayer, ..
        } = &mut self.educators
        {
            *educator_taxpayer = value;
        }
    }
    pub(crate) fn educator_spouse(&self) -> i64 {
        match &self.educators {
            EducatorClaim::No => 0,
            EducatorClaim::Yes {
                educator_spouse, ..
            } => educator_spouse.amount(),
        }
    }

    pub(crate) fn eligible_educators(&self) -> Answer {
        match &self.educators {
            EducatorClaim::No => Answer::No,
            EducatorClaim::Yes {
                eligible_educators, ..
            } => *eligible_educators,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_eligible_educators(&mut self, value: Answer) {
        if matches!(self.educators, EducatorClaim::No) {
            self.educators = EducatorClaim::Yes {
                educator_taxpayer: 0,
                educator_spouse: SpouseAmount::No,
                eligible_educators: Answer::No,
            };
        }
        if let EducatorClaim::Yes {
            eligible_educators, ..
        } = &mut self.educators
        {
            *eligible_educators = value;
        }
    }
}

/// Home mortgage interest to evaluate
/// Choose Yes to enter the amounts and eligibility facts; No has no additional fields.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum MortgageClaim {
    /// No
    No,
    /// Yes
    Yes {
        /// Qualified acquisition mortgage interest
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        mortgage_interest: i64,
        /// qualified home acquisition debt is within the applicable $750,000/$1,000,000 limits (half for separate returns), with no mixed-use debt; choose Not sure when unresolved
        mortgage_within_limit: Answer,
    },
}

impl ItemizedExpenses {
    pub(crate) fn mortgage_interest(&self) -> i64 {
        match &self.mortgage {
            MortgageClaim::No => 0,
            MortgageClaim::Yes {
                mortgage_interest, ..
            } => *mortgage_interest,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_mortgage_interest(&mut self, value: i64) {
        if matches!(self.mortgage, MortgageClaim::No) {
            self.mortgage = MortgageClaim::Yes {
                mortgage_interest: 0,
                mortgage_within_limit: Answer::No,
            };
        }
        if let MortgageClaim::Yes {
            mortgage_interest, ..
        } = &mut self.mortgage
        {
            *mortgage_interest = value;
        }
    }
    pub(crate) fn mortgage_within_limit(&self) -> Answer {
        match &self.mortgage {
            MortgageClaim::No => Answer::No,
            MortgageClaim::Yes {
                mortgage_within_limit,
                ..
            } => *mortgage_within_limit,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_mortgage_within_limit(&mut self, value: Answer) {
        if matches!(self.mortgage, MortgageClaim::No) {
            self.mortgage = MortgageClaim::Yes {
                mortgage_interest: 0,
                mortgage_within_limit: Answer::No,
            };
        }
        if let MortgageClaim::Yes {
            mortgage_within_limit,
            ..
        } = &mut self.mortgage
        {
            *mortgage_within_limit = value;
        }
    }
}

/// Cash charitable gifts to evaluate
/// Choose Yes to enter the amounts and eligibility facts; No has no additional fields.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum CharityClaim {
    /// No
    No,
    /// Yes
    Yes {
        /// Current-year cash gifts to eligible public charities, excluding goods/services received
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        cash_charity: i64,
        /// eligible organizations and substantiation, no donor-advised fund or supporting organization; choose Not sure when unresolved
        cash_charity_eligible: Answer,
    },
}

impl FederalDeductions {
    pub(crate) fn cash_charity(&self) -> i64 {
        match &self.charity {
            CharityClaim::No => 0,
            CharityClaim::Yes { cash_charity, .. } => *cash_charity,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_cash_charity(&mut self, value: i64) {
        if matches!(self.charity, CharityClaim::No) {
            self.charity = CharityClaim::Yes {
                cash_charity: 0,
                cash_charity_eligible: Answer::No,
            };
        }
        if let CharityClaim::Yes { cash_charity, .. } = &mut self.charity {
            *cash_charity = value;
        }
    }
    pub(crate) fn cash_charity_eligible(&self) -> Answer {
        match &self.charity {
            CharityClaim::No => Answer::No,
            CharityClaim::Yes {
                cash_charity_eligible,
                ..
            } => *cash_charity_eligible,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_cash_charity_eligible(&mut self, value: Answer) {
        if matches!(self.charity, CharityClaim::No) {
            self.charity = CharityClaim::Yes {
                cash_charity: 0,
                cash_charity_eligible: Answer::No,
            };
        }
        if let CharityClaim::Yes {
            cash_charity_eligible,
            ..
        } = &mut self.charity
        {
            *cash_charity_eligible = value;
        }
    }
}

/// Work-related dependent-care credit to evaluate
/// Choose Yes to enter the amounts and eligibility facts; No has no additional fields.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum CareClaim {
    /// No
    No,
    /// Yes
    Yes {
        /// Work-related care expenses for a disabled spouse, net of reimbursements; child expenses belong on each dependent
        care_expenses: SpouseAmount,
        /// residency, earned-income and eligible-provider requirements are met; choose Not sure when unresolved
        care_eligible: Answer,
        /// Employer-provided dependent-care benefits excluded from income
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        care_benefits: i64,
    },
}

impl FederalCredits {
    pub(crate) fn care_expenses(&self) -> i64 {
        match &self.care {
            CareClaim::No => 0,
            CareClaim::Yes { care_expenses, .. } => care_expenses.amount(),
        }
    }

    pub(crate) fn care_eligible(&self) -> Answer {
        match &self.care {
            CareClaim::No => Answer::No,
            CareClaim::Yes { care_eligible, .. } => *care_eligible,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_care_eligible(&mut self, value: Answer) {
        if matches!(self.care, CareClaim::No) {
            self.care = CareClaim::Yes {
                care_expenses: SpouseAmount::No,
                care_eligible: Answer::No,
                care_benefits: 0,
            };
        }
        if let CareClaim::Yes { care_eligible, .. } = &mut self.care {
            *care_eligible = value;
        }
    }
    pub(crate) fn care_benefits(&self) -> i64 {
        match &self.care {
            CareClaim::No => 0,
            CareClaim::Yes { care_benefits, .. } => *care_benefits,
        }
    }
}

/// Education credit to evaluate
/// Credit-specific questions belong only to the selected credit.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
#[libertas_ui_header]
pub enum EducationChoice {
    /// American opportunity credit
    AmericanOpportunity {
        /// First four years, fewer than four prior claims, at least half-time degree study, and no disqualifying drug conviction
        requirements: Answer,
    },
    /// Lifetime learning credit
    LifetimeLearning,
}
impl FederalEducation {
    pub(crate) fn method(&self) -> EducationMethod {
        match self.method {
            EducationChoice::AmericanOpportunity { .. } => EducationMethod::AmericanOpportunity,
            EducationChoice::LifetimeLearning => EducationMethod::LifetimeLearning,
        }
    }
    pub(crate) fn aotc_requirements(&self) -> Answer {
        match self.method {
            EducationChoice::AmericanOpportunity { requirements } => requirements,
            EducationChoice::LifetimeLearning => Answer::No,
        }
    }
}

/// When the retirement distribution was received
/// For 2026 the backend uses this return's filing status; other years ask about joint filing.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport,
)]
#[libertas_ui_header]
pub enum SaverDistributionPeriod {
    /// 2024
    PriorTwo {
        /// Filed jointly with the current spouse for this year
        joint_with_current_spouse: Answer,
    },
    /// 2025
    PriorOne {
        /// Filed jointly with the current spouse for this year
        joint_with_current_spouse: Answer,
    },
    /// 2026
    TaxYear,
    /// 2027, before the 2026 return due date including extensions
    FollowingYear {
        /// Filed or expect to file jointly with the current spouse for this year
        joint_with_current_spouse: Answer,
    },
}
impl FederalSaverDistribution {
    pub(crate) fn year(&self) -> SaverDistributionYear {
        match self.year {
            SaverDistributionPeriod::PriorTwo { .. } => SaverDistributionYear::PriorTwo,
            SaverDistributionPeriod::PriorOne { .. } => SaverDistributionYear::PriorOne,
            SaverDistributionPeriod::TaxYear => SaverDistributionYear::TaxYear,
            SaverDistributionPeriod::FollowingYear { .. } => SaverDistributionYear::FollowingYear,
        }
    }
    pub(crate) fn joint_with_current_spouse(&self) -> Answer {
        match self.year {
            SaverDistributionPeriod::PriorTwo {
                joint_with_current_spouse,
            }
            | SaverDistributionPeriod::PriorOne {
                joint_with_current_spouse,
            }
            | SaverDistributionPeriod::FollowingYear {
                joint_with_current_spouse,
            } => joint_with_current_spouse,
            SaverDistributionPeriod::TaxYear => Answer::No,
        }
    }
}

/// Spouse on a joint return
/// Select Yes to provide the applicable facts; No carries no additional data.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum SpouseFiler {
    /// No
    No,
    /// Yes
    Yes {
        /// Applicable facts
        #[libertas_ui_header]
        facts: FederalPerson,
    },
}
impl SpouseFiler {
    pub fn as_ref(&self) -> Option<&FederalPerson> {
        match self {
            Self::No => None,
            Self::Yes { facts } => Some(facts),
        }
    }
    pub fn as_mut(&mut self) -> Option<&mut FederalPerson> {
        match self {
            Self::No => None,
            Self::Yes { facts } => Some(facts),
        }
    }
    pub fn is_some(&self) -> bool {
        self.as_ref().is_some()
    }
    pub fn is_none(&self) -> bool {
        self.as_ref().is_none()
    }
    pub fn iter(&self) -> core::option::IntoIter<&FederalPerson> {
        self.as_ref().into_iter()
    }
}
impl From<Option<FederalPerson>> for SpouseFiler {
    fn from(value: Option<FederalPerson>) -> Self {
        match value {
            None => Self::No,
            Some(facts) => Self::Yes { facts },
        }
    }
}

/// Qualified tips or overtime
/// Select Yes to provide the applicable facts; No carries no additional data.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum WorkDeductionChoice {
    /// No
    No,
    /// Yes
    Yes {
        /// Applicable facts
        #[libertas_ui_header]
        facts: FederalWorkDeductions,
    },
}
impl WorkDeductionChoice {
    pub fn as_ref(&self) -> Option<&FederalWorkDeductions> {
        match self {
            Self::No => None,
            Self::Yes { facts } => Some(facts),
        }
    }
    pub fn as_mut(&mut self) -> Option<&mut FederalWorkDeductions> {
        match self {
            Self::No => None,
            Self::Yes { facts } => Some(facts),
        }
    }
    pub fn is_some(&self) -> bool {
        self.as_ref().is_some()
    }
    pub fn is_none(&self) -> bool {
        self.as_ref().is_none()
    }
    pub fn iter(&self) -> core::option::IntoIter<&FederalWorkDeductions> {
        self.as_ref().into_iter()
    }
}
impl From<Option<FederalWorkDeductions>> for WorkDeductionChoice {
    fn from(value: Option<FederalWorkDeductions>) -> Self {
        match value {
            None => Self::No,
            Some(facts) => Self::Yes { facts },
        }
    }
}

/// Traditional or Roth IRA contributions
/// Select Yes to provide the applicable facts; No carries no additional data.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum IraChoice {
    /// No
    No,
    /// Yes
    Yes {
        /// Applicable facts
        #[libertas_ui_header]
        facts: FederalIras,
    },
}
impl IraChoice {
    pub fn as_ref(&self) -> Option<&FederalIras> {
        match self {
            Self::No => None,
            Self::Yes { facts } => Some(facts),
        }
    }
    pub fn as_mut(&mut self) -> Option<&mut FederalIras> {
        match self {
            Self::No => None,
            Self::Yes { facts } => Some(facts),
        }
    }
    pub fn is_some(&self) -> bool {
        self.as_ref().is_some()
    }
    pub fn is_none(&self) -> bool {
        self.as_ref().is_none()
    }
    pub fn iter(&self) -> core::option::IntoIter<&FederalIras> {
        self.as_ref().into_iter()
    }
}
impl From<Option<FederalIras>> for IraChoice {
    fn from(value: Option<FederalIras>) -> Self {
        match value {
            None => Self::No,
            Some(facts) => Self::Yes { facts },
        }
    }
}

/// Workplace retirement contributions for Saver’s Credit
/// Select Yes to provide the applicable facts; No carries no additional data.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum SaverChoice {
    /// No
    No,
    /// Yes
    Yes {
        /// Applicable facts
        #[libertas_ui_header]
        facts: FederalSaver,
    },
}
impl SaverChoice {
    pub fn as_ref(&self) -> Option<&FederalSaver> {
        match self {
            Self::No => None,
            Self::Yes { facts } => Some(facts),
        }
    }
    pub fn as_mut(&mut self) -> Option<&mut FederalSaver> {
        match self {
            Self::No => None,
            Self::Yes { facts } => Some(facts),
        }
    }
    pub fn is_some(&self) -> bool {
        self.as_ref().is_some()
    }
    pub fn is_none(&self) -> bool {
        self.as_ref().is_none()
    }
    pub fn iter(&self) -> core::option::IntoIter<&FederalSaver> {
        self.as_ref().into_iter()
    }
}
impl From<Option<FederalSaver>> for SaverChoice {
    fn from(value: Option<FederalSaver>) -> Self {
        match value {
            None => Self::No,
            Some(facts) => Self::Yes { facts },
        }
    }
}

/// Spouse situation for IRA contributions and workplace coverage
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum IraSpouse {
    /// Not filing jointly or separately while living together
    NotApplicable,
    /// Joint return
    Joint {
        /// Spouse’s contributions and workplace coverage, including when contributions are zero
        #[libertas_ui_header]
        facts: FederalIraContribution,
    },
    /// Separate return; lived together during the year
    SeparateTogether {
        /// Spouse covered by a workplace retirement plan
        covered_at_work: Answer,
    },
}
impl IraSpouse {
    pub fn as_ref(&self) -> Option<&FederalIraContribution> {
        match self {
            Self::Joint { facts } => Some(facts),
            _ => None,
        }
    }
    pub fn as_mut(&mut self) -> Option<&mut FederalIraContribution> {
        match self {
            Self::Joint { facts } => Some(facts),
            _ => None,
        }
    }
    pub fn is_some(&self) -> bool {
        self.as_ref().is_some()
    }
    pub fn iter(&self) -> core::option::IntoIter<&FederalIraContribution> {
        self.as_ref().into_iter()
    }
}

/// Expenses for an itemized deduction
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub struct ItemizedExpenses {
    /// Unreimbursed eligible medical and dental expenses, excluding HSA/FSA reimbursements
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub medical: i64,
    /// Eligible state/local income OR sales tax, plus real-estate and personal-property taxes; never both income and sales tax
    #[libertas_money("USD", 2)]
    #[libertas_number(min = 0, max = 100000000000)]
    pub state_local_taxes: i64,
    /// Home mortgage interest to evaluate
    pub mortgage: MortgageClaim,
}
/// Itemized expenses to compare with the standard deduction
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum ItemizedChoice {
    /// No
    No,
    /// Yes
    Yes {
        /// Itemized expenses
        #[libertas_ui_header]
        facts: ItemizedExpenses,
    },
}
/// Deduction to evaluate
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum DeductionSelection {
    /// Choose the larger allowed deduction
    Automatic {
        /// Have itemized expenses to compare
        expenses: ItemizedChoice,
    },
    /// Standard deduction
    Standard,
    /// Itemized deduction
    Itemized {
        /// Itemized expenses
        #[libertas_ui_header]
        facts: ItemizedExpenses,
    },
}
impl FederalDeductions {
    pub(crate) fn choice(&self) -> DeductionChoice {
        match self.choice {
            DeductionSelection::Automatic { .. } => DeductionChoice::Automatic,
            DeductionSelection::Standard => DeductionChoice::Standard,
            DeductionSelection::Itemized { .. } => DeductionChoice::Itemized,
        }
    }
    fn itemized(&self) -> Option<&ItemizedExpenses> {
        match &self.choice {
            DeductionSelection::Automatic {
                expenses: ItemizedChoice::Yes { facts },
            }
            | DeductionSelection::Itemized { facts } => Some(facts),
            _ => None,
        }
    }
    pub(crate) fn medical(&self) -> i64 {
        self.itemized().map_or(0, |f| f.medical)
    }
    pub(crate) fn state_local_taxes(&self) -> i64 {
        self.itemized().map_or(0, |f| f.state_local_taxes)
    }
    pub(crate) fn mortgage_interest(&self) -> i64 {
        self.itemized().map_or(0, |f| f.mortgage_interest())
    }
    pub(crate) fn mortgage_within_limit(&self) -> Answer {
        self.itemized()
            .map_or(Answer::No, |f| f.mortgage_within_limit())
    }
}

/// Applicable spouse amount
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum SpouseAmount {
    /// No
    No,
    /// Yes
    Yes {
        /// Amount for the spouse
        #[libertas_money("USD", 2)]
        #[libertas_number(min = 0, max = 100000000000)]
        amount: i64,
    },
}
impl SpouseAmount {
    pub fn amount(&self) -> i64 {
        match self {
            Self::No => 0,
            Self::Yes { amount } => *amount,
        }
    }
}
impl From<i64> for SpouseAmount {
    fn from(amount: i64) -> Self {
        if amount == 0 {
            Self::No
        } else {
            Self::Yes { amount }
        }
    }
}

/// Dependent residence period
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
#[libertas_ui_header]
pub enum DependentResidency {
    /// Count days in the home
    Days {
        /// Days in your home in 2026, including qualifying temporary absences
        #[libertas_number(min = 0, max = 365)]
        days: u32,
    },
    /// Qualifying whole-year residency exception
    /// Newborn, death or temporary-absence rule treats residency as the whole year.
    WholeYearException,
}
impl FederalDependent {
    pub(crate) fn days_in_home(&self) -> u32 {
        match self.residency {
            DependentResidency::Days { days } => days,
            DependentResidency::WholeYearException => 365,
        }
    }
}
