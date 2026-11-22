# Architecture decision records

An architecture decision record (ADR) states **why** GiroCrab made a lasting technical
decision, which alternatives were considered and which drawbacks we accept. The current
structure is described in the [architecture notes](../ARCHITECTURE.md), practical steps
in the guides (e.g. [CONTRIBUTING.md](../../CONTRIBUTING.md)). An ADR links to them
instead of repeating them.

## When to write an ADR

Reserve ADRs for decisions that set a lasting direction for GiroCrab. A
proposal should meet both criteria.

1. Will future work in another component, connector or front end rely on the
   decision, or would reversing it have substantial costs for users or the project?
2. Were there reasonable alternatives whose trade-offs future contributors
   will need to understand?

Examples include vault compatibility, security boundaries, shared data models
and contracts between the core and front ends. Capture one decision with its
scope, alternatives, rationale and consequences. When documenting an existing
decision, link the evidence and flag reasons that are still unknown.

Keep routine bug fixes, local refactorings and small UI changes in the PR.
Put implementation steps in guides, bank-specific behaviour in connector docs,
and test results in audits. Use issues for plans and open questions.

## Process

1. Copy the [template](0000-template.md) to the next free numbered file
   (`NNNN-short-title.md`). Keep its status **Proposed**, link the relevant
   discussion and add the draft to the index below.
2. Submit the draft in a PR. It can accompany an implementation or document
   an existing decision. Review the decision and its alternatives in the PR.
3. The maintainers decide whether to accept or reject the proposal. Update the
   status and date in the ADR and the status in the index as part of that review.
4. To replace an accepted decision, submit a new ADR. Once accepted, mark the
   previous ADR **Superseded** and link to its successor. Update its index entry
   to **Superseded by NNNN**. Also set its status date to the date of that
   change.

| Status | Meaning |
| --- | --- |
| Proposed | Draft under discussion. |
| Accepted | The decision applies. |
| Rejected | Retained with a short reason. Revisit the proposal only with new evidence or arguments. |
| Superseded by NNNN | Follow the linked successor. Revisit the old option only with new evidence or arguments. |

## Index

| ADR | Status |
| --- | --- |
| [0001 – Bank connector architecture](0001-connector-architecture.md) | Proposed |
| [0002 – Refund links, stable split identities and shared projection](0002-refund-links-and-projection.md) | Proposed |
