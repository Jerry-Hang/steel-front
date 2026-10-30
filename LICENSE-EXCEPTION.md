# Steel Front — Additional Permission (revenue-capped closed-source use)

**Copyright (C) 2026 Huang Shaojie (黄少杰).**

This document grants an **additional permission** under section 7 of the GNU
Affero General Public License v3.0. It is **not** a restriction and **not** a
condition added to the AGPL: it only **relaxes** one obligation (§13 /
source disclosure) for small organizations, which is exactly what AGPL §7
allows a copyright holder to do.

If you do not need this permission — because you are publishing your source, or
because your use does not trigger §13 — you can ignore this file entirely and
just use the project under [`LICENSE`](./LICENSE).

---

## 1. The permission in plain language

**If the total revenue of your organization (including its corporate group) for
its most recently completed fiscal quarter is below CNY 30,000,000 (thirty
million yuan per quarter — an annualized CNY 120,000,000, ≈ USD 17 million),
you may ship closed-source builds of this software, and run modified versions as
a service, WITHOUT the source-disclosure obligations of AGPL §13.**

Concretely, for organizations under that line, all of the following are allowed
without paying anything and without publishing your code:

- compiling this engine into a commercial game and selling it, closed source;
- modifying `src/**` and shipping the modified engine inside your product;
- running a modified build as a hosted/network service (AGPL §13 territory);
- shipping on Steam / itch.io / consoles / mobile stores.

You must still:

- keep the copyright notice and [`LICENSE`](./LICENSE), [`NOTICE`](./NOTICE),
  and this file with any redistribution of the engine's source or binaries;
- not use the project's name or the authors' names to endorse your product;
- not claim you wrote the engine.

That is all. No application, no email, no registration, no fee.

---

## 2. Above the line

If your organization's quarterly revenue is **at or above CNY 30,000,000**
(annualized CNY 120,000,000),
you have two options, both fine:

1. **Publish your source** under AGPL-3.0 as [`LICENSE`](./LICENSE) requires —
   free, at any scale, forever; or
2. **Buy the commercial license** described in
   [`LICENSE-COMMERCIAL.md`](./LICENSE-COMMERCIAL.md) — a **symbolic** annual
   fee, agreed in writing, that covers maintenance of the project rather than
   extracting revenue from you.

Option 1 is not a consolation prize: a large studio that opens its engine
modifications pays nothing.

---

## 3. How "revenue" is measured

- **Measured per quarter, annualized: the line is CNY 30,000,000 of total
  revenue per fiscal quarter (CNY 120,000,000 per year).** The quarterly figure
  is what is tested; the annual number is just the same line stated yearly.
- The number is the **total revenue of the legal entity and its corporate group
  for that quarter**, from **all** business activities — not just the product
  that embeds this engine.
- Multi-entity groups aggregate. If a subsidiary ships the product, the parent
  group's revenue is what counts.
- **Once you are over the line, you stay over it for that fiscal year** — a
  single strong quarter is enough; you do not get to average it away.
- A newly formed entity with no completed quarter is assessed on its run-rate
  (revenue received so far, annualized). If that run-rate is under the line, you
  are under the line.
- **Funding is not revenue.** A pre-revenue startup that raised a large round is
  still under the line; what matters is revenue actually received for goods or
  services. (This is deliberate: the target is companies already extracting
  money with the engine, not companies that merely have capital.)
- If you are unsure, ask: <huangshaojie925@gmail.com>. Asking is free, and the
  answer is given in good faith.

---

## 4. What this permission does *not* do

- It does **not** relicense third-party material. CC0 assets, the OFL-derived
  glyph bitmaps, and the Rust dependencies keep their own terms — see
  [`THIRD-PARTY-NOTICES.md`](./THIRD-PARTY-NOTICES.md).
- It does **not** transfer copyright, and it does **not** create any warranty.
  The software is provided **as is**, exactly as AGPL §15–§17 state.
- It does **not** apply retroactively to a party that already exceeded the line
  in the fiscal year in which the infringing use began.
- It does **not** prevent the copyright holder from granting the same or a
  broader permission to anyone else later.

---

## 5. Why this shape

The author's intent, stated plainly: **individuals, hobbyists, students, and
small-to-medium studios should be able to use this engine for real products
without asking anyone, and without a lawyer.** Large organizations that already
have revenue should either contribute back by opening their source, or pay a
symbolic fee that keeps the project alive.

Expressing that as an AGPL §7 **additional permission** is what makes it real:
the small developer is not relying on the author "looking the other way" — they
hold an actual, irrevocable grant. Nobody has to guess at intent, and nobody
has to litigate it later.

> Recommended (not required): if your product does well, consider opening the
> engine-side changes you made, or sponsoring the project. Neither is a
> condition of the permission above.

---

## 6. Version and permanence

This permission is granted for **AGPL-3.0-or-later** versions of this project.
The copyright holder may publish updated versions of this document (for example
to adjust the threshold for inflation), but **a permission once granted for a
given release is not withdrawn retroactively** — code you obtained under this
permission stays usable on those terms.

*End of additional permission.*
