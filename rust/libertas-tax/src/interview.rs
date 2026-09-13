use crate::rules::{QUESTIONS, TOPICS, estimate, income_totals};
use crate::*;
use alloc::vec::Vec;
use libertas::{LibertasMessageArgument, libertas_formatted_text};

pub(crate) struct Interview {
    pub(crate) draft: DraftV2,
    pub(crate) dirty: bool,
    blocked: bool,
}
#[derive(Debug)]
pub(crate) enum Reply {
    Response(TaxInterviewProtocol),
    Edit(TaxInterviewProtocol),
}
use TaxInterviewProtocol as P;
fn text(value: &str) -> Vec<u8> {
    libertas_formatted_text("TEXT", &[LibertasMessageArgument::LiteralText(value)])
}
fn problem(value: &str) -> Reply {
    Reply::Response(P::ProblemV2 {
        message: text(value),
    })
}

impl Interview {
    pub(crate) fn new(draft: DraftV2) -> Self {
        let blocked = !draft.valid_stored();
        Self {
            draft,
            blocked,
            dirty: false,
        }
    }
    pub(crate) fn blocked() -> Self {
        let mut state = Self::new(DraftV2::empty("unavailable".into()));
        state.blocked = true;
        state
    }
    fn progress(&self, title: &str) -> Vec<u8> {
        let d = &self.draft;
        let basis = match d.setup.as_ref().map(|s| s.basis) {
            Some(AmountBasisV2::Actual) => "full-year actual amounts",
            Some(AmountBasisV2::Estimate) => "full-year estimates; no automatic annualization",
            None => "choose a full-year amount basis",
        };
        let accepted = usize::from(d.setup.is_some())
            + usize::from(d.taxpayer.is_some())
            + usize::from(d.joint() && d.spouse.is_some())
            + d.coverage.len()
            + usize::from(d.income_complete)
            + usize::from(d.charity.is_some())
            + usize::from(d.charity == Some(AnswerV2::Yes) && d.charity_qualified.is_some())
            + usize::from(d.charity_qualified == Some(AnswerV2::Yes) && d.charity_amount.is_some())
            + usize::from(d.payments.is_some());
        let applicable = 17
            + usize::from(d.joint())
            + usize::from(d.charity == Some(AnswerV2::Yes))
            + usize::from(d.charity_qualified == Some(AnswerV2::Yes));
        text(&alloc::format!(
            "{} · {} of {} currently applicable pages accepted · 2026 federal · {}. Accepted pages are saved; leaving an unfinished editor discards its changes. Synthetic data only.",
            title,
            accepted,
            applicable,
            basis
        ))
    }

    fn previous(&self, page: i32, purpose: EditPurposeV2) -> i32 {
        if purpose == EditPurposeV2::Review {
            return 20;
        }
        match page {
            0 => 0,
            3 if !self.draft.joint() => 1,
            19 if self.draft.charity != Some(AnswerV2::Yes) => 16,
            19 if self.draft.charity_qualified != Some(AnswerV2::Yes) => 17,
            _ => (page - 1).max(0),
        }
    }
    fn page(&self, page: i32, purpose: EditPurposeV2) -> Reply {
        let d = &self.draft;
        // Only accepted pages and the first uncompleted page are reachable.
        if !(0..=20).contains(&page)
            || (page > d.first_incomplete() && page != 20)
            || (page == 2 && !d.joint())
        {
            return problem("This page is not available yet. Reopen the interview to continue.");
        }
        let previous = self.previous(page, purpose);
        let progress = self.progress(match page {
            0 => "About this return",
            1 => "About you",
            2 => "About your spouse",
            3..=14 => "Check your tax situation",
            15 => "Income documents",
            16..=18 => "Cash donations",
            19 => "Federal payments",
            _ => "Review",
        });
        match page {
            0 => match &d.setup {
                Some(setup) => Reply::Edit(P::SaveSetupV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    setup: setup.clone(),
                    confirm_basis_change: false,
                }),
                None => Reply::Response(P::BeginSetupV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                }),
            },
            1 | 2 => match if page == 1 { &d.taxpayer } else { &d.spouse } {
                Some(person) => Reply::Edit(P::SavePersonV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    person: person.clone(),
                }),
                None => Reply::Response(P::BeginPersonV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                }),
            },
            3..=14 | 16 | 17 => {
                let topic = if page <= 14 {
                    TOPICS[(page - 3) as usize]
                } else {
                    CoverageTopicV2::Other
                };
                let question = text(if page <= 14 {
                    QUESTIONS[(page - 3) as usize]
                } else if page == 16 {
                    "Did you make cash donations in 2026 that you want considered for the non-itemizer deduction?"
                } else {
                    "Were all these donations current-year cash gifts to eligible U.S. public charities, with required records, no goods or services received, and no donor-advised fund, supporting organization, private foundation, foreign charity or carryover? If unsure, choose Not sure."
                });
                let answer = if page <= 14 {
                    d.coverage
                        .iter()
                        .find(|a| a.topic == topic)
                        .map(|a| a.answer)
                } else if page == 16 {
                    d.charity
                } else {
                    d.charity_qualified
                };
                match answer {
                    Some(answer) => Reply::Edit(P::SaveQuestionV2 {
                        cookie: d.cookie.clone(),
                        revision: d.revision,
                        purpose,
                        page,
                        previous,
                        progress,
                        question,
                        topic,
                        answer,
                    }),
                    None => Reply::Response(P::BeginQuestionV2 {
                        cookie: d.cookie.clone(),
                        revision: d.revision,
                        purpose,
                        page,
                        previous,
                        progress,
                        question,
                        topic,
                    }),
                }
            }
            15 => Reply::Response(P::IncomeOverviewV2 {
                cookie: d.cookie.clone(),
                revision: d.revision,
                purpose,
                page,
                previous,
                progress,
                records: d.records.clone(),
            }),
            18 => match d.charity_amount {
                Some(amount) => Reply::Edit(P::SaveCharityAmountV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    amount,
                }),
                None => Reply::Response(P::BeginCharityAmountV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                }),
            },
            19 => match &d.payments {
                Some(payments) => Reply::Edit(P::SavePaymentsV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    payments: payments.clone(),
                }),
                None => Reply::Response(P::BeginPaymentsV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                }),
            },
            _ => {
                let purpose = EditPurposeV2::Review;
                Reply::Response(P::ReviewV2 { cookie: d.cookie.clone(), revision: d.revision, purpose, page, previous, notice: self.progress("Review your estimate. It is not a filed return; penalties, interest and offsets are excluded"), estimate: estimate(d), records: d.records.clone() })
            }
        }
    }
    fn document(&self, kind: IncomeKindV2, id: i64, purpose: EditPurposeV2) -> Reply {
        let d = &self.draft;
        let page = 15;
        let previous = if purpose == EditPurposeV2::Review {
            20
        } else {
            15
        };
        let progress = self.progress("Income document · preview totals do not save changes");
        if let Some(r) = d.records.iter().find(|r| r.id == id && r.kind == kind) {
            let (income, tax) = income_totals(d, None).expect("validated stored bounds");
            let preview_income = Some(income);
            let preview_withholding = Some(tax);
            return match kind {
                IncomeKindV2::Wage => Reply::Edit(P::SaveWageV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    id,
                    owner: r.owner,
                    payer: r.payer.clone(),
                    amount: r.amount,
                    withholding: r.withholding,
                    preview_income,
                    preview_withholding,
                    medicare_wages: r.medicare_wages,
                    social_security_withheld: r.social_security_withheld,
                }),
                IncomeKindV2::Interest => Reply::Edit(P::SaveInterestV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    progress,
                    id,
                    owner: r.owner,
                    payer: r.payer.clone(),
                    amount: r.amount,
                    withholding: r.withholding,
                    preview_income,
                    preview_withholding,
                }),
            };
        }
        if id != d.next_id || d.records.len() >= 100 {
            return problem(
                "This document is unavailable or the 100-document prototype limit has been reached.",
            );
        }
        match kind {
            IncomeKindV2::Wage => Reply::Response(P::BeginWageV2 {
                cookie: d.cookie.clone(),
                revision: d.revision,
                purpose,
                page,
                previous,
                progress,
                id,
            }),
            IncomeKindV2::Interest => Reply::Response(P::BeginInterestV2 {
                cookie: d.cookie.clone(),
                revision: d.revision,
                purpose,
                page,
                previous,
                progress,
                id,
            }),
        }
    }
    fn accept(&mut self, mut candidate: DraftV2) -> Result<(), &'static str> {
        candidate.revision = self
            .draft
            .revision
            .checked_add(1)
            .ok_or("This return cannot accept another revision.")?;
        candidate.finished = false;
        if !candidate.valid_stored() {
            return Err(
                "These answers cannot be saved. Check the return label, document details and amounts.",
            );
        }
        self.draft = candidate;
        self.dirty = true;
        Ok(())
    }
    fn after(&self, next: i32, purpose: EditPurposeV2) -> Reply {
        let first = self.draft.first_incomplete();
        self.page(
            if purpose == EditPurposeV2::Review && first == 20 {
                20
            } else {
                next.min(first)
            },
            purpose,
        )
    }
    fn save_document(&mut self, record: IncomeRecordV2, purpose: EditPurposeV2) -> Reply {
        if self.draft.first_incomplete() < 15 {
            return problem("Complete the earlier interview pages before saving income.");
        }
        if record.owner == OwnerV2::Spouse && !self.draft.joint() {
            return problem(
                "Spouse income requires a joint return. Choose You or change the filing status.",
            );
        }
        let mut candidate = self.draft.clone();
        let is_new = record.id == candidate.next_id;
        if let Some(old) = candidate.records.iter_mut().find(|r| r.id == record.id) {
            if old.kind != record.kind {
                return problem("The document type changed. Reopen it from the document list.");
            }
            *old = record;
        } else if record.id == candidate.next_id && candidate.records.len() < 100 {
            candidate.next_id += 1;
            candidate.records.push(record);
        } else {
            return problem("This document no longer exists. Reopen the document list.");
        }
        if let Err(reply) = self.accept(candidate) {
            return problem(reply);
        }
        if is_new {
            self.page(15, purpose)
        } else {
            self.after(15, purpose)
        }
    }
    fn preview(&self, kind: IncomeKindV2, id: i64, amount: i64, withholding: i64) -> Reply {
        if !self
            .draft
            .records
            .iter()
            .any(|r| r.id == id && r.kind == kind)
            && id != self.draft.next_id
        {
            return problem("This document changed. Reopen the editor before calculating again.");
        }
        match income_totals(&self.draft, Some((id, amount, withholding))) {
            Some((income, withholding)) => Reply::Response(P::IncomePreviewV2 {
                income: Some(income),
                withholding: Some(withholding),
            }),
            None => problem("The preview amounts are outside the supported range."),
        }
    }
    pub(crate) fn handle(&mut self, request: P) -> Option<Reply> {
        let identity = match &request {
            P::SaveSetupV2 {
                cookie, revision, ..
            }
            | P::SavePersonV2 {
                cookie, revision, ..
            }
            | P::SaveQuestionV2 {
                cookie, revision, ..
            }
            | P::ContinueIncomeV2 {
                cookie, revision, ..
            }
            | P::SelectDocumentV2 {
                cookie, revision, ..
            }
            | P::SaveWageV2 {
                cookie, revision, ..
            }
            | P::SaveInterestV2 {
                cookie, revision, ..
            }
            | P::PreviewWageV2 {
                cookie, revision, ..
            }
            | P::PreviewInterestV2 {
                cookie, revision, ..
            }
            | P::SaveCharityAmountV2 {
                cookie, revision, ..
            }
            | P::SavePaymentsV2 {
                cookie, revision, ..
            }
            | P::FinishV2 {
                cookie, revision, ..
            }
            | P::StartFinishV2 {
                cookie, revision, ..
            } => Some((cookie.as_str(), Some(*revision))),
            P::AddWageV2 { cookie, .. }
            | P::AddInterestV2 { cookie, .. }
            | P::ChooseDocumentV2 { cookie, .. }
            | P::NavigateV2 { cookie, .. }
            | P::BackV2 { cookie, .. } => Some((cookie.as_str(), None)),
            P::OpenInterviewV1 | P::ReloadV2 => None,
            P::SaveAboutV1 { .. }
            | P::SaveIncomeV1 { .. }
            | P::BackToAboutV1 { .. }
            | P::BackToIncomeV1 { .. } => {
                return Some(Reply::Response(P::ProblemV1 {
                    message: text("This earlier interview is retired. Reopen the endpoint."),
                }));
            }
            _ => return None,
        };
        if self.blocked {
            return Some(problem(
                "The stored return uses an unsupported or invalid record. It has been preserved; use a new task for this prototype.",
            ));
        }
        if let Some((cookie, revision)) = identity {
            if cookie != self.draft.cookie {
                return Some(problem(
                    "This return identity is no longer available. Reopen the endpoint.",
                ));
            }
            if revision.is_some_and(|r| r != self.draft.revision) {
                return Some(problem(
                    "Accepted answers changed. Reopen the editor before submitting again.",
                ));
            }
        }
        let reply = match request {
            P::OpenInterviewV1 | P::ReloadV2 => {
                self.page(self.draft.first_incomplete(), EditPurposeV2::Interview)
            }
            P::SaveSetupV2 {
                setup,
                purpose,
                confirm_basis_change,
                ..
            } => {
                let mut c = self.draft.clone();
                if c.setup.as_ref().is_some_and(|s| s.basis != setup.basis) {
                    if !confirm_basis_change {
                        return Some(problem(
                            "Changing the amount basis affects accepted amounts. Confirm they are appropriate for the new basis, then review income, donations and payments.",
                        ));
                    }
                    c.income_complete = false;
                    c.payments = None;
                    c.charity_amount = None;
                }
                if c.setup
                    .as_ref()
                    .is_some_and(|s| s.filing_status != setup.filing_status)
                {
                    c.spouse = None;
                    c.income_complete = false;
                    c.coverage.clear();
                }
                c.setup = Some(setup);
                if let Err(r) = self.accept(c) {
                    problem(r)
                } else {
                    self.after(1, purpose)
                }
            }
            P::SavePersonV2 {
                page,
                person,
                purpose,
                ..
            } => {
                if self.draft.setup.is_none()
                    || (page != 1
                        && !(page == 2 && self.draft.joint() && self.draft.taxpayer.is_some()))
                {
                    return Some(problem("This person page is not currently available."));
                }
                let mut c = self.draft.clone();
                if page == 1 {
                    c.taxpayer = Some(person);
                } else {
                    c.spouse = Some(person);
                }
                if let Err(r) = self.accept(c) {
                    problem(r)
                } else {
                    self.after(
                        if page == 1 && self.draft.joint() {
                            2
                        } else {
                            3
                        },
                        purpose,
                    )
                }
            }
            P::SaveQuestionV2 {
                page,
                topic,
                answer,
                purpose,
                ..
            } => {
                if page > self.draft.first_incomplete()
                    || !matches!(page, 3..=14 | 16 | 17)
                    || (page == 17 && self.draft.charity != Some(AnswerV2::Yes))
                    || (page <= 14 && topic != TOPICS[(page - 3) as usize])
                {
                    return Some(problem(
                        "This question is not currently available. Reopen the interview.",
                    ));
                }
                let mut c = self.draft.clone();
                if page <= 14 {
                    if let Some(a) = c.coverage.iter_mut().find(|a| a.topic == topic) {
                        a.answer = answer;
                    } else {
                        c.coverage.push(CoverageAnswerV2 { topic, answer });
                    }
                } else if page == 16 {
                    if c.charity != Some(answer) {
                        c.charity_qualified = None;
                        c.charity_amount = None;
                    }
                    c.charity = Some(answer);
                } else {
                    if c.charity_qualified != Some(answer) {
                        c.charity_amount = None;
                    }
                    c.charity_qualified = Some(answer);
                }
                if let Err(r) = self.accept(c) {
                    problem(r)
                } else {
                    self.after(
                        if page == 16 && answer != AnswerV2::Yes
                            || page == 17 && answer != AnswerV2::Yes
                        {
                            19
                        } else {
                            page + 1
                        },
                        purpose,
                    )
                }
            }
            P::AddWageV2 { purpose, .. } => {
                self.document(IncomeKindV2::Wage, self.draft.next_id, purpose)
            }
            P::AddInterestV2 { purpose, .. } => {
                self.document(IncomeKindV2::Interest, self.draft.next_id, purpose)
            }
            P::ChooseDocumentV2 { purpose, .. } => {
                if self.draft.records.is_empty() {
                    return Some(problem("Add an income document first."));
                }
                let d = &self.draft;
                let page = 15;
                let previous = if purpose == EditPurposeV2::Review {
                    20
                } else {
                    15
                };
                Reply::Response(P::BeginSelectV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                    records: d.records.clone(),
                })
            }
            P::SelectDocumentV2 {
                records,
                selection,
                remove,
                purpose,
                ..
            } => {
                if records != self.draft.records {
                    return Some(problem("The document list changed. Reopen the selection."));
                }
                let Some(r) = self.draft.records.get(selection as usize).cloned() else {
                    return Some(problem("Select a document from the current list."));
                };
                if remove {
                    let mut c = self.draft.clone();
                    c.records.remove(selection as usize);
                    if let Err(r) = self.accept(c) {
                        problem(r)
                    } else {
                        self.after(15, purpose)
                    }
                } else {
                    self.document(r.kind, r.id, purpose)
                }
            }
            P::ContinueIncomeV2 { purpose, .. } => {
                if self.draft.first_incomplete() < 15 {
                    return Some(problem("Complete the earlier interview pages first."));
                }
                let mut c = self.draft.clone();
                c.income_complete = true;
                if let Err(r) = self.accept(c) {
                    problem(r)
                } else {
                    self.after(16, purpose)
                }
            }
            P::SaveWageV2 {
                id,
                owner,
                payer,
                amount,
                withholding,
                medicare_wages,
                social_security_withheld,
                purpose,
                ..
            } => self.save_document(
                IncomeRecordV2 {
                    id,
                    kind: IncomeKindV2::Wage,
                    owner,
                    payer,
                    amount,
                    withholding,
                    medicare_wages,
                    social_security_withheld,
                },
                purpose,
            ),
            P::SaveInterestV2 {
                id,
                owner,
                payer,
                amount,
                withholding,
                purpose,
                ..
            } => self.save_document(
                IncomeRecordV2 {
                    id,
                    kind: IncomeKindV2::Interest,
                    owner,
                    payer,
                    amount,
                    withholding,
                    medicare_wages: 0,
                    social_security_withheld: 0,
                },
                purpose,
            ),
            P::PreviewWageV2 {
                id,
                amount,
                withholding,
                ..
            } => self.preview(IncomeKindV2::Wage, id, amount, withholding),
            P::PreviewInterestV2 {
                id,
                amount,
                withholding,
                ..
            } => self.preview(IncomeKindV2::Interest, id, amount, withholding),
            P::SaveCharityAmountV2 {
                amount, purpose, ..
            } => {
                if self.draft.first_incomplete() < 18
                    || self.draft.charity_qualified != Some(AnswerV2::Yes)
                {
                    return Some(problem("Confirm cash donation eligibility first."));
                }
                let mut c = self.draft.clone();
                c.charity_amount = Some(amount);
                if let Err(r) = self.accept(c) {
                    problem(r)
                } else {
                    self.after(19, purpose)
                }
            }
            P::SavePaymentsV2 {
                payments, purpose, ..
            } => {
                if self.draft.first_incomplete() < 19 {
                    return Some(problem("Complete the earlier interview pages first."));
                }
                let mut c = self.draft.clone();
                c.payments = Some(payments);
                if let Err(r) = self.accept(c) {
                    problem(r)
                } else {
                    self.after(20, purpose)
                }
            }
            P::NavigateV2 {
                section,
                topic,
                purpose,
                ..
            } => {
                let page = match section {
                    SectionV2::Setup => 0,
                    SectionV2::Taxpayer => 1,
                    SectionV2::Spouse => 2,
                    SectionV2::Coverage => topic
                        .and_then(|t| TOPICS.iter().position(|v| *v == t))
                        .map_or(3, |n| n as i32 + 3),
                    SectionV2::Income => 15,
                    SectionV2::Charity => 16,
                    SectionV2::Payments => 19,
                    SectionV2::Review => 20,
                };
                self.page(page, purpose)
            }
            // Going back is identity-based and discards the unfinished editor. Load accepted data at the latest revision.
            P::BackV2 {
                previous, purpose, ..
            } => self.page(previous, purpose),
            P::StartFinishV2 { .. } => {
                if estimate(&self.draft).state != ResultStateV2::EstimatedForSupportedScope {
                    return Some(problem(
                        "Finish is available only after all answers are complete and supported. Review the listed limitations.",
                    ));
                }
                let d = &self.draft;
                let purpose = EditPurposeV2::Review;
                let page = 20;
                let previous = 20;
                Reply::Response(P::BeginFinishV2 {
                    cookie: d.cookie.clone(),
                    revision: d.revision,
                    purpose,
                    page,
                    previous,
                })
            }
            P::FinishV2 { confirmed, .. } => {
                if !confirmed {
                    return Some(problem(
                        "Confirm that this is an estimate, not a filed return, to finish the prototype.",
                    ));
                }
                if estimate(&self.draft).state != ResultStateV2::EstimatedForSupportedScope {
                    return Some(problem(
                        "The estimate cannot finish until the listed incomplete or unsupported situations are resolved.",
                    ));
                }
                let c = self.draft.clone();
                if let Err(r) = self.accept(c) {
                    problem(r)
                } else {
                    self.draft.finished = true;
                    Reply::Response(P::FinishedV2 {
                        notice: text(
                            "Prototype complete. Your accepted answers are saved. Nothing has been filed. Reopen the endpoint to review or edit this estimate.",
                        ),
                        estimate: estimate(&self.draft),
                    })
                }
            }
            _ => return None,
        };
        Some(reply)
    }
}
