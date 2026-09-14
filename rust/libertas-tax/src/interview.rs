use crate::federal_rules::{estimate, valid_birth, validate_dependent, validate_income};
use crate::*;
use TaxInterviewProtocol as P;
use alloc::vec::Vec;
use libertas::{LibertasMessageArgument, libertas_formatted_text};

pub(crate) struct Interview {
    pub(crate) draft: FederalDraft,
    pub(crate) dirty: bool,
    blocked: bool,
}
#[derive(Debug)]
pub(crate) enum Reply {
    Response(P),
    Edit(P),
}
fn text(value: &str) -> Vec<u8> {
    libertas_formatted_text("TEXT", &[LibertasMessageArgument::LiteralText(value)])
}
fn problem(value: &str) -> Reply {
    Reply::Response(P::Problem {
        message: text(value),
    })
}
impl Interview {
    pub(crate) fn new(draft: FederalDraft) -> Self {
        let blocked = !draft.valid_stored();
        Self {
            draft,
            dirty: false,
            blocked,
        }
    }
    fn progress(&self, page: i32) -> Vec<u8> {
        text(&alloc::format!(
            "2026 federal · section {} of 9 · accepted pages are saved; unsent edits are discarded. Synthetic full-year amounts only.",
            page + 1
        ))
    }
    fn review(&self) -> Reply {
        Reply::Response(P::Review {
            cookie: self.draft.cookie.clone(),
            revision: self.draft.revision,
            result: estimate(&self.draft),
        })
    }
    fn page(&self, page: i32, review: bool) -> Reply {
        let d = &self.draft;
        if !(0..=9).contains(&page) || (page > d.first_incomplete() && page != 9) {
            return problem("Complete the earlier required section before opening this page.");
        }
        let cookie = d.cookie.clone();
        let revision = d.revision;
        let previous = if review { 9 } else { (page - 1).max(0) };
        let progress = self.progress(page);
        match page {
            0 => match &d.setup {
                Some(value) => Reply::Edit(P::SaveSetup {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                    value: value.clone(),
                    confirm_basis_change: false,
                }),
                None => Reply::Response(P::BeginSetup {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                }),
            },
            1 => match &d.people {
                Some(value) => Reply::Edit(P::SavePeople {
                    allowed_spouse: alloc::vec![if d.setup.as_ref().unwrap().filing_status()
                        == FederalStatus::Joint
                    {
                        1
                    } else {
                        0
                    }],
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                    value: value.clone(),
                }),
                None => Reply::Response(P::BeginPeople {
                    allowed_spouse: alloc::vec![if d.setup.as_ref().unwrap().filing_status()
                        == FederalStatus::Joint
                    {
                        1
                    } else {
                        0
                    }],
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                }),
            },
            4 => match &d.adjustments {
                Some(value) => Reply::Edit(P::SaveAdjustments {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                    value: value.clone(),
                }),
                None => Reply::Response(P::BeginAdjustments {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                }),
            },
            5 => match &d.deductions {
                Some(value) => Reply::Edit(P::SaveDeductions {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                    value: value.clone(),
                }),
                None => Reply::Response(P::BeginDeductions {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                }),
            },
            6 => match &d.credits {
                Some(value) => Reply::Edit(P::SaveCredits {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                    value: { crate::federal_rules::editable_credits(d, value) },
                }),
                None => Reply::Response(P::BeginCredits {
                    students: crate::federal_rules::students(d),
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                }),
            },
            7 => match &d.screening {
                Some(value) => Reply::Edit(P::SaveScreening {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                    value: value.clone(),
                }),
                None => Reply::Response(P::BeginScreening {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                }),
            },
            8 => match &d.payments {
                Some(value) => Reply::Edit(P::SavePayments {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                    value: value.clone(),
                }),
                None => Reply::Response(P::BeginPayments {
                    cookie,
                    revision,
                    page,
                    previous,
                    review,
                    progress,
                }),
            },
            2 => Reply::Response(P::Children {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                records: d.dependents.clone(),
            }),
            3 => Reply::Response(P::IncomeDocuments {
                cookie,
                revision,
                page,
                previous,
                review,
                progress,
                records: d.income.clone(),
            }),
            _ => self.review(),
        }
    }
    fn accept(&mut self, mut draft: FederalDraft, page: i32, review: bool) -> Reply {
        // A review-only Next is navigation: compare answers before touching saved
        // history, completion, or revision. Cookie/navigation metadata is not in draft.
        if draft == self.draft {
            return if review {
                self.review()
            } else {
                self.page((page + 1).min(self.draft.first_incomplete()), false)
            };
        }
        // Replace the accepted page and remove its downstream answers in the same
        // persisted record. Never reuse identities from erased dependent/document rows.
        draft.erase_after(page);
        if !draft.valid_stored() {
            return problem("Review the accepted page values before saving.");
        }
        let Some(revision) = draft.revision.checked_add(1).filter(|r| *r < i64::MAX) else {
            return problem("This return has reached the supported revision limit.");
        };
        draft.revision = revision;
        draft.finished = false;
        self.draft = draft;
        self.dirty = true;
        if review {
            self.review()
        } else {
            self.page(self.draft.first_incomplete(), false)
        }
    }
    pub(crate) fn handle(&mut self, request: P) -> Option<Reply> {
        let identity = match &request {
            P::OpenInterview => None,
            P::SaveSetup {
                cookie, revision, ..
            }
            | P::SavePeople {
                cookie, revision, ..
            }
            | P::SaveAdjustments {
                cookie, revision, ..
            }
            | P::SaveDeductions {
                cookie, revision, ..
            }
            | P::SaveCredits {
                cookie, revision, ..
            }
            | P::SaveScreening {
                cookie, revision, ..
            }
            | P::SavePayments {
                cookie, revision, ..
            }
            | P::SaveDependent {
                cookie, revision, ..
            }
            | P::SaveIncome {
                cookie, revision, ..
            }
            | P::AddDependent {
                cookie, revision, ..
            }
            | P::AddIncome {
                cookie, revision, ..
            }
            | P::SelectDependent {
                cookie, revision, ..
            }
            | P::SelectIncome {
                cookie, revision, ..
            }
            | P::ChooseDependent {
                cookie, revision, ..
            }
            | P::ChooseIncome {
                cookie, revision, ..
            }
            | P::ContinueSection {
                cookie, revision, ..
            }
            | P::Back {
                cookie, revision, ..
            }
            | P::Navigate {
                cookie, revision, ..
            }
            | P::StartFinish {
                cookie, revision, ..
            }
            | P::Finish {
                cookie, revision, ..
            }
            | P::PreviewIncome {
                cookie, revision, ..
            } => Some((cookie, *revision)),
            _ => return None,
        };
        if self.blocked {
            return Some(problem(
                "Stored accepted data is invalid. It has not been changed; use a fresh development task.",
            ));
        }
        if let Some((cookie, revision)) = identity {
            if cookie != &self.draft.cookie {
                return Some(problem(
                    "This return identity is not available. Reopen the endpoint.",
                ));
            }
            // Back and section navigation are identity-based; mutations, selections and previews are revision-based.
            if !matches!(request, P::Back { .. } | P::Navigate { .. })
                && revision != self.draft.revision
            {
                return Some(problem(
                    "Accepted answers changed. Reopen the editor before trying again.",
                ));
            }
        }
        let d = &self.draft;
        Some(match request {
            P::OpenInterview => {
                if d.finished {
                    self.review()
                } else {
                    self.page(d.first_incomplete(), false)
                }
            }
            P::Back { previous, .. } => self.page(previous, false),
            P::Navigate { section, .. } => self.page(section as i32, true),
            P::SaveSetup {
                value,
                page,
                review,
                confirm_basis_change,
                ..
            } => {
                if page != 0 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                let mut draft = d.clone();
                if let Some(old) = &d.setup
                    && old.basis != value.basis
                    && !confirm_basis_change
                {
                    return Some(problem(
                        "Confirm the basis change; saved later sections will be erased.",
                    ));
                }
                draft.setup = Some(value);
                self.accept(draft, page, review)
            }
            P::SavePeople {
                value,
                page,
                review,
                ..
            } => {
                if page != 1 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                if !valid_birth(value.taxpayer.birth_date)
                    || value
                        .spouse
                        .as_ref()
                        .is_some_and(|p| !valid_birth(p.birth_date))
                {
                    return Some(problem(
                        "Enter valid filer birth dates on or before December 31, 2026.",
                    ));
                }
                if d.setup.as_ref().is_some_and(|s| {
                    (s.filing_status() == FederalStatus::Joint) != value.spouse.is_some()
                }) {
                    return Some(problem("Provide spouse facts for a joint return only."));
                }
                let mut draft = d.clone();
                draft.people = Some(value);
                self.accept(draft, page, review)
            }
            P::SaveAdjustments {
                value,
                page,
                review,
                ..
            } => {
                if page != 4 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                let mut draft = d.clone();
                draft.adjustments = Some(value);
                self.accept(draft, page, review)
            }
            P::SaveDeductions {
                value,
                page,
                review,
                ..
            } => {
                if page != 5 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                let mut draft = d.clone();
                draft.deductions = Some(value);
                self.accept(draft, page, review)
            }
            P::SaveCredits {
                value,
                page,
                review,
                ..
            } => {
                if page != 6 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                if value.students != crate::federal_rules::students(d) {
                    return Some(problem(
                        "The student choices changed. Reopen the credits section.",
                    ));
                }
                if value.education.iter().any(|e| {
                    value
                        .students
                        .get(e.student as usize)
                        .is_none_or(|v| !crate::federal_rules::student_present(d, v.id))
                }) {
                    return Some(problem(
                        "An education entry refers to a person no longer on this return. Select the correct student or remove the entry.",
                    ));
                }
                let mut draft = d.clone();
                draft.credits = Some(value);
                self.accept(draft, page, review)
            }
            P::SaveScreening {
                value,
                page,
                review,
                ..
            } => {
                if page != 7 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                let mut draft = d.clone();
                draft.screening = Some(value);
                self.accept(draft, page, review)
            }
            P::SavePayments {
                value,
                page,
                review,
                ..
            } => {
                if page != 8 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                let mut draft = d.clone();
                draft.payments = Some(value);
                self.accept(draft, page, review)
            }
            P::AddDependent { .. } => {
                if d.first_incomplete() < 2 {
                    return Some(problem("Complete the earlier section first."));
                }
                if d.dependents.len() >= 30 {
                    return Some(problem("The prototype record limit has been reached."));
                }
                Reply::Response(P::BeginDependent {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    page: 2,
                    previous: 2,
                    review: false,
                    progress: self.progress(2),
                })
            }
            P::SelectDependent { review, .. } => {
                if d.dependents.is_empty() {
                    return Some(problem("There are no accepted records to edit."));
                }
                Reply::Response(P::BeginChooseDependent {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    page: 2,
                    previous: 2,
                    review,
                    progress: self.progress(2),
                    records: d.dependents.clone(),
                })
            }
            P::ChooseDependent {
                selection,
                remove,
                review,
                ..
            } => {
                let Some(value) = d.dependents.get(selection as usize) else {
                    return Some(problem("The selected record is no longer available."));
                };
                if remove {
                    let mut draft = d.clone();
                    draft.dependents.remove(selection as usize);
                    let reply = self.accept(draft, 2, true);
                    if matches!(reply, Reply::Response(P::Problem { .. })) {
                        reply
                    } else {
                        self.page(2, review)
                    }
                } else {
                    Reply::Edit(P::SaveDependent {
                        cookie: d.cookie.clone(),
                        revision: d.revision,
                        page: 2,
                        previous: 2,
                        review,
                        progress: self.progress(2),
                        value: value.clone(),
                    })
                }
            }
            P::SaveDependent {
                mut value,
                page,
                review,
                ..
            } => {
                if page != 2 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                if let Err(message) = validate_dependent(&value) {
                    return Some(problem(message));
                }
                let mut draft = d.clone();
                if value.id == 0 {
                    if draft.dependents.len() >= 30 {
                        return Some(problem("The prototype record limit has been reached."));
                    }
                    value.id = draft.next_id;
                    draft.next_id += 1;
                    draft.dependents.push(value);
                } else {
                    let Some(existing) = draft.dependents.iter_mut().find(|v| v.id == value.id)
                    else {
                        return Some(problem("This accepted record is no longer available."));
                    };
                    *existing = value;
                }
                let reply = self.accept(draft, 2, true);
                if matches!(reply, Reply::Response(P::Problem { .. })) || review {
                    reply
                } else {
                    self.page(2, false)
                }
            }
            P::AddIncome { .. } => {
                if d.first_incomplete() < 3 {
                    return Some(problem("Complete the earlier section first."));
                }
                if d.income.len() >= 100 {
                    return Some(problem("The prototype record limit has been reached."));
                }
                Reply::Response(P::BeginIncome {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    page: 3,
                    previous: 3,
                    review: false,
                    progress: self.progress(3),
                })
            }
            P::SelectIncome { review, .. } => {
                if d.income.is_empty() {
                    return Some(problem("There are no accepted records to edit."));
                }
                Reply::Response(P::BeginChooseIncome {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    page: 3,
                    previous: 3,
                    review,
                    progress: self.progress(3),
                    records: d.income.clone(),
                })
            }
            P::ChooseIncome {
                selection,
                remove,
                review,
                ..
            } => {
                let Some(value) = d.income.get(selection as usize) else {
                    return Some(problem("The selected record is no longer available."));
                };
                if remove {
                    let mut draft = d.clone();
                    draft.income.remove(selection as usize);
                    let reply = self.accept(draft, 3, true);
                    if matches!(reply, Reply::Response(P::Problem { .. })) {
                        reply
                    } else {
                        self.page(3, review)
                    }
                } else {
                    Reply::Edit(P::SaveIncome {
                        cookie: d.cookie.clone(),
                        revision: d.revision,
                        page: 3,
                        previous: 3,
                        review,
                        progress: self.progress(3),
                        value: value.clone(),
                        preview: None,
                    })
                }
            }
            P::SaveIncome {
                mut value,
                page,
                review,
                ..
            } => {
                if page != 3 || page > d.first_incomplete() {
                    return Some(problem("This section is not available yet."));
                }
                if let Err(message) = validate_income(&value) {
                    return Some(problem(message));
                }
                let mut draft = d.clone();
                if value.id == 0 {
                    if draft.income.len() >= 100 {
                        return Some(problem("The prototype record limit has been reached."));
                    }
                    value.id = draft.next_id;
                    draft.next_id += 1;
                    draft.income.push(value);
                } else {
                    let Some(existing) = draft.income.iter_mut().find(|v| v.id == value.id) else {
                        return Some(problem("This accepted record is no longer available."));
                    };
                    *existing = value;
                }
                let reply = self.accept(draft, 3, true);
                if matches!(reply, Reply::Response(P::Problem { .. })) || review {
                    reply
                } else {
                    self.page(3, false)
                }
            }
            P::ContinueSection { page, review, .. } => {
                if !matches!(page, 2 | 3) || page > d.first_incomplete() {
                    return Some(problem("This list is not ready to continue."));
                }
                let mut draft = d.clone();
                if page == 2 {
                    draft.dependents_complete = true;
                } else {
                    draft.income_complete = true;
                }
                self.accept(draft, page, review)
            }
            P::StartFinish { .. } => {
                if estimate(d).state != ResultState::EstimatedForSupportedScope {
                    return Some(problem(
                        "Resolve the remaining answers or coverage issues before completing the prototype review.",
                    ));
                }
                Reply::Response(P::BeginFinish {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    page: 9,
                    previous: 9,
                    review: true,
                    progress: text("Complete the prototype review. Nothing is signed or filed."),
                })
            }
            P::Finish { .. } => {
                let result = estimate(d);
                if result.state != ResultState::EstimatedForSupportedScope {
                    return Some(problem("The return is not ready to finish reviewing."));
                }
                let Some(revision) = self.draft.revision.checked_add(1).filter(|r| *r < i64::MAX)
                else {
                    return Some(problem(
                        "This return has reached the supported revision limit.",
                    ));
                };
                self.draft.revision = revision;
                self.draft.finished = true;
                self.dirty = true;
                Reply::Response(P::Finished {
                    message: text(
                        "Prototype review completed. No tax return has been signed or filed.",
                    ),
                    result: estimate(&self.draft),
                })
            }
            P::PreviewIncome { value, .. } => {
                if let Err(message) = validate_income(&value) {
                    return Some(problem(message));
                }
                if value.id != 0 && !d.income.iter().any(|v| v.id == value.id) {
                    return Some(problem("The preview's accepted document no longer exists."));
                }
                Reply::Response(P::IncomePreview {
                    preview: Some(document_amount(&value.income)),
                })
            }
            _ => return None,
        })
    }
}
pub(crate) fn document_amount(value: &FederalIncome) -> i64 {
    match value {
        FederalIncome::Wage { data: v } => v.wages,
        FederalIncome::Interest { data: v } => v.taxable,
        FederalIncome::Dividend { data: v } => v.ordinary + v.capital_distributions,
        FederalIncome::Sale { data: v } => v.proceeds - v.basis + v.wash_sale,
        FederalIncome::Retirement { data: v } => v.taxable,
        FederalIncome::SocialSecurity { data: v } => v.benefits,
        FederalIncome::Unemployment { data: v } => v.amount,
        FederalIncome::Business { data: v } => v.receipts - v.expenses,
    }
}
