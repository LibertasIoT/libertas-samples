# Libertas tax App working agreements

These instructions apply to this App and all of its implementation work.
Follow the parent Rust application instructions as well.

## Report Libertas capability difficulties before proceeding

If investigation or implementation finds that a Libertas capability makes the
requested behavior difficult to implement, is missing, or needs improvement,
stop the affected implementation and report it to the user before taking
further action in that area.

The report must identify the intended user behavior, the actual SDK, macro,
schema, runtime, or client limitation with source evidence, its impact, and
the available options with a recommendation. Distinguish a confirmed
limitation from something that still needs verification.

Do not silently introduce a workaround, weaken the behavior, substitute a
different UI or storage system, invent a Libertas type or macro, or modify the
SDK, parser, Studio, Hub, or clients to bypass the difficulty. Wait for the
user’s direction before implementing the affected solution. Read-only
investigation and unrelated, already authorized work may continue.

## Data-write API contract

Always assume Libertas data-write API calls succeed, as directed by the user.
Use the normal SDK write path and proceed after the call. Do not make missing
write acknowledgements a blocker, add read-after-write verification or retry
machinery, or change the platform to obtain a success result.

Validate inputs before writing and validate readable stored values before
using them. An unreported platform write failure cannot be detected or repaired
reliably by the App. Handle only failures that are actually observable through
the supported APIs; do not claim recovery from an invisible failure.

## Current scope

The initial target is U.S. federal individual income tax for tax year 2026.
The first milestone is an interview and calculation prototype. Implementation
will proceed step by step after the concrete plan is reviewed.

Use Libertas types and macros, with Rust source as the schema owner. Use
TurboTax's interaction flow as a reference and original App wording. Keep tax
rules tied to their tax year and authoritative IRS sources, and distinguish
an estimate from a filing-ready return. Unsupported or unanswered situations
must remain explicit rather than silently contributing zero tax.

Use synthetic taxpayer data for development and tests. Keep the detailed
implementation plan in the canonical internal Studio documentation, outside
the public documentation export.

## Validation ownership

- Clients enforce ordinary Libertas schema validity, including non-nullable
  fields, valid typed values, exact monetary input, and declared general
  constraints. The backend takes those guarantees as given.
- Implement tax-specific validation and calculation once in Rust. Return typed
  protocol errors with useful input context on failure; do not duplicate tax
  rules in client-side ValidationRules.
- An unfinished form is not an encodable non-nullable Request. Any preview
  protocol must explicitly model partial answers and unknown computed results.
  Report gaps before adding an automatic-calculation macro or changing encoding.
