# Steel Front — Commercial License (large organizations, symbolic fee)

**Copyright (C) 2026 Huang Shaojie (黄少杰).**

This document covers **one narrow case**: an organization whose **quarterly**
revenue is **at or above CNY 30,000,000** (annualized CNY 120 million) and that
wants to ship the engine **closed source**. Everyone else does not need this file — see the table below.

---

## 1. Which track applies to you

| Your situation | What applies | Cost |
|---|---|---|
| You publish your source under AGPL-3.0 | [`LICENSE`](./LICENSE) | Free, any company size, forever |
| Closed source **and** quarterly revenue **below** CNY 30M (annualized 120M) | [`LICENSE-EXCEPTION.md`](./LICENSE-EXCEPTION.md) — an AGPL §7 **additional permission** | Free, no application needed |
| Closed source **and** quarterly revenue **at or above** CNY 30M (annualized 120M) | **This file** | Symbolic annual fee |

The middle row is the important one: small studios, solo developers, students
and hobbyists have a real, written permission to keep their code closed. They
are not relying on anyone's goodwill.

---

## 2. What the commercial license adds

- the §13 source-disclosure obligation does **not** apply to your product;
- the engine may be modified and embedded in proprietary code, shipped on any
  platform, as a service or as a binary;
- a grant that survives acquisition of your studio (transferable with the
  product, not the company).

It does **not** add: exclusivity, trademark rights, warranty, support SLA, or
ownership of copyright. Third-party material keeps its own terms
([`THIRD-PARTY-NOTICES.md`](./THIRD-PARTY-NOTICES.md)) — in practice a non-issue
for the CC0 assets, the OFL glyph data, and the permissive Rust crates.

---

## 3. The fee

**Symbolic, and deliberately small.** The point is to keep the project
maintained, not to extract money from you.

- Annual fee, **negotiated by project scale**; the author's stated intent is a
  token amount that a funded studio will not have to think about.
- No per-seat, per-title, or revenue-percentage component.
- No fee at all for the open-source path (§1, row 1) — the author explicitly
  prefers that studios take it: *opening your engine changes is worth more to
  this project than a fee.*
- Payment terms, currency and invoicing are agreed in writing before use.

---

## 4. How to obtain one

Email **<huangshaojie925@gmail.com>** with:

1. legal entity name, country, and the most recent fiscal **quarter's** total
   revenue band (a band is enough — no audited statements required);
2. the product the engine will ship in;
3. whether you need binary distribution, network-service use, or both.

Nothing in this document is itself a grant. The license exists once a written
agreement is signed by the copyright holder. Until then, this project remains
available to you under AGPL-3.0 (plus the small-organization permission, if it
applies).

---

## 5. Why the earlier single-file version was replaced

Before 2026-10-30 the repository shipped one `LICENSE` file that declared
"AGPL-3.0 plus additional commercial terms which prevail over the AGPL where
they conflict". That construction cannot work:

- AGPL **§7** permits additional terms only from a closed list (warranty
  disclaimers, notice preservation, no misrepresentation of origin, limits on
  the use of names/trademarks), and every permitted additional term is at most
  neutral — none may take a right away;
- AGPL **§10** forbids imposing any further restriction on recipients.

A revenue-triggered restriction is neither, so read one way it is void, and read
the other way the document is not AGPL at all — leaving the project unlicensed,
i.e. "all rights reserved", which is strictly worse for everyone.

The current structure fixes that by splitting the intent into the two shapes the
GPL family actually supports:

1. **relaxing** the license for small organizations ⇒ an *additional
   permission* ([`LICENSE-EXCEPTION.md`](./LICENSE-EXCEPTION.md)), which §7
   explicitly allows; and
2. **selling** a separate license to parties who want out of §13 ⇒ a true dual
   license (this file), which never narrows the AGPL granted to anyone else.

Same business intent as before. Now it is enforceable, and a small developer can
point to a document instead of trusting an intention.

---

*End of commercial license terms.*
