# Steel Front — Commercial License (optional alternative to AGPL-3.0)

**Copyright (C) 2026 Huang Shaojie (黄少杰).**

This document describes the **commercial license** that the copyright holder
offers as an **alternative** to the free software license. It is not a
modification of the AGPL, and it does not restrict anyone who uses the project
under the AGPL.

---

## 1. Two independent grants

The project is available under **either** of the following, at your option:

| Option | License | When to choose it |
|---|---|---|
| **A. Free software** | GNU Affero GPL v3.0-or-later — full text in [`LICENSE`](./LICENSE) | You can live with AGPL §13: if you distribute the software, or run a modified version over a network, you publish the corresponding source under the AGPL. No fee, at any company size. |
| **B. Commercial** | A separate written license from the copyright holder, described below | You want to ship closed-source builds, embed the engine in a proprietary product, or otherwise cannot accept AGPL §13. |

Option A is a complete license: choosing it means you never need to talk to
the copyright holder. Option B exists because some users cannot use Option A,
not because Option A is limited.

---

## 2. What the commercial license grants

A commercial license grants, for the term stated in the signed agreement:

- the right to use, modify, and distribute the software **in closed-source
  form**, without the source-disclosure obligations of AGPL §13;
- the right to combine it with proprietary code and ship the result;
- a non-exclusive, non-transferable, worldwide right, limited to the products
  and legal entity named in the agreement.

It does **not** grant ownership of the copyright, and it does not grant rights
to third-party material listed in [`THIRD-PARTY-NOTICES.md`](./THIRD-PARTY-NOTICES.md)
(those remain under their own terms — for practical purposes this is a non-issue
for the CC0 assets, the OFL font data, and the permissive Rust crates).

---

## 3. Pricing policy

This is a **pricing policy**, not a condition attached to the AGPL:

- **Quarterly revenue below CNY 10,000,000** (ten million Chinese yuan, or the
  equivalent in another currency, measured on the licensee's most recently
  completed fiscal quarter, aggregated across the licensee's whole
  organization): a commercial license is granted **free of charge on request**.
- **At or above that threshold**: the fee is agreed in writing before
  commercial use begins.

Requesting a free commercial license below the threshold is a courtesy that
takes one email. It is **not** required for using the project under the AGPL —
under the AGPL you owe nothing regardless of revenue, forever.

---

## 4. How to obtain one

Write to **<huangshaojie925@gmail.com>** with:

1. legal entity name and country;
2. the product the engine will be embedded in;
3. whether you need closed-source distribution, or only the network-service
   exception removed from §13.

Alternatively open an issue at
<https://github.com/Jerry-Hang/steel-front/issues> and ask for the commercial
contact. Nothing in this document is itself a grant: the license exists once a
written agreement is signed by the copyright holder.

---

## 5. Why the earlier single-file version was replaced

Before 2026-10-30 the repository shipped one `LICENSE` file containing AGPL-3.0
plus "additional commercial terms" that were declared to prevail over the AGPL
where they conflicted. That construction does not work:

- AGPL-3.0 **§7** permits additional terms only in a closed list — warranty
  disclaimers, preservation of notices, no misrepresentation of origin,
  limitations on the use of names/trademarks;
- AGPL-3.0 **§10** forbids imposing any further restriction on the exercise of
  the rights the license grants.

A revenue-triggered restriction on closed-source use is neither. Read one way it
is void (recipients simply fall back on the AGPL grant), read the other way the
document is not AGPL at all — and a work that is not validly licensed is
"all rights reserved" by default, which is worse for everyone including the
copyright holder. Splitting the offer into a verbatim AGPL text plus this
separate, optional commercial grant gives exactly the same business outcome
with a valid legal structure.
