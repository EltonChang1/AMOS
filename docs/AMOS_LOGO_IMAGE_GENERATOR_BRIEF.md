# AMOS Logo — Image Generator Brief

Use this document as the full instruction set when prompting an image generator to create AMOS logo marks. Paste the **Global constraints** block into every prompt, then append one **Concept prompt**.

## Product context (for the generator)

AMOS is an internally deployed analyst system: a customer-controlled control layer between an AI model and company data/tools. It admits plans, enforces policy, runs authoritative calculations outside the model, verifies claims, preserves evidence, and produces reviewable artifacts.

Brand promise in four words: **Private · Auditable · Analyst · Layer**.

## Brand feel

- Quiet confidence — institutional, not startup-neon
- Control over cleverness — structured, engineered, precise
- Analyst + audit — intelligence that leaves a trail, not magic
- Trustworthy infrastructure for enterprise analysis

The mark should read as “internally deployed, auditable analyst layer” at a glance.

## Must include

1. **Lettermark “A” as the primary mark** — clean, geometric, slightly architectural; readable as favicon, app icon, and slide watermark.
2. **A boundary / enclosure** (when using Governed A or Memory Seal) — soft shield, frame, rounded square, or closed ring signaling customer-controlled / internal deployment. Must not look like antivirus.
3. **A verification cue** (subtle) — one small check, seal notch, locked joint, or sealed corner. Suggests “claims are verified.” Do not make it a literal checklist icon.
4. **Layer or stack hint** (when using Layer A) — two thin planes or a stratified “A” suggesting AMOS sits between the model and company data/tools.
5. **Evidence / provenance thread** (when using Evidence A) — a thin continuous line through the mark, like a provenance edge; optional for other concepts.
6. **Wordmark only when explicitly requested** — default prompts are **mark-only**. If adding wordmark: tight modern sans, all-caps or small-caps “AMOS”, strong optical spacing. Optional tagline: `Private · Auditable · Analyst`.

## Must not include

- Brains, robots, faces, sparkles, chat bubbles
- Neural-net blobs, circuit boards, or generic “AI” motifs
- Literal databases, SQL glyphs, or dashboard chrome
- Overly ornate crests / security-theater heraldry
- Purple, neon glow, multi-layer drop shadows, glossy metallic chrome
- Warm cream (#F4F1EA) + terracotta serif looks
- Purple-on-white or purple-to-indigo gradient themes
- Emojis, pill clusters, badge stickers floating on the mark
- Photorealism, 3D renders, skeuomorphism

## Composition rules

| Variant | Contents |
|---|---|
| Primary | Monogram in a simple enclosure |
| Secondary | Monogram + wordmark “AMOS” |
| Tertiary | Wordmark alone (dense UI) |

- Flat vector style; high contrast; print-safe
- Centered, balanced, works at small sizes
- Sharp enough to feel engineered; not cold or military
- White or transparent background unless a dark variant is requested
- No gradients required; prefer solid fills

## Color palette

| Role | Hex | Notes |
|---|---|---|
| Ink / primary | `#1B2430` or `#182028` | Deep slate for the main geometry |
| Accent A (steel-teal) | `#2F6F7E` or `#2E6672` | Sealed corners, ring notches |
| Accent B (steel-blue) | `#3A5F7A` | Second plane in Layer A |
| Accent C (olive-teal) | `#3D6B5A` | Evidence thread / check |
| Background | `#FFFFFF` | Default; use near-black only for dark variants |

Use **one** restrained accent per mark. Do not rainbow the logo.

---

## Global constraints (paste into every prompt)

```text
Minimal enterprise software logo mark for AMOS, a private auditable AI analyst
control layer. Flat vector style, quiet confidence, institutional tech brand,
precise engineered geometry, high contrast, print-safe. Colors limited to deep
slate ink (#1B2430 / #182028) plus at most one restrained accent from
steel-teal (#2F6F7E / #2E6672), steel-blue (#3A5F7A), or olive-teal (#3D6B5A).
White background. Centered, balanced, suitable as app icon / favicon.
No gradients, no glow, no purple, no neon, no robots, no brains, no neural
networks, no chat bubbles, no sparkles, no databases, no SQL glyphs, no
metallic chrome, no ornate crests, no photorealism, no 3D. Logo mark only
unless wordmark is explicitly requested.
```

---

## Concept prompts

### 1. Governed A (recommended primary)

Best for: product UI, app icon, “customer-controlled” story.

```text
[PASTE GLOBAL CONSTRAINTS]

Concept: "Governed A" — a geometric capital letter A constructed from precise
architectural strokes, centered inside a rounded square enclosure/frame. One
corner of the frame has a subtle sealed notch suggesting control and security.
Deep slate ink for the A and frame; steel-teal accent on the sealed corner
detail only. No wordmark text.
```

### 2. Layer A

Best for: architecture decks, “AMOS as control layer.”

```text
[PASTE GLOBAL CONSTRAINTS]

Concept: "Layer A" — a stylized capital letter A formed from two thin
overlapping geometric planes/layers, suggesting a control layer between an AI
model and company data. Precise engineered geometry, slight offset between
planes creating depth without 3D shading. Deep slate ink for one plane, muted
steel-blue for the second plane. No wordmark text.
```

### 3. Evidence A

Best for: trust/audit messaging, review and claims.

```text
[PASTE GLOBAL CONSTRAINTS]

Concept: "Evidence A" — a clean geometric capital A with a thin continuous
provenance thread/line that runs through the letter and subtly closes into a
small verification check at the tip or base. Suggests auditable analysis and
evidence trails. Deep charcoal slate for the A; olive-teal accent for the thin
evidence thread and check detail only. No wordmark text.
```

### 4. Memory Seal

Best for: docs, approval/publication, enterprise seal feel.

```text
[PASTE GLOBAL CONSTRAINTS]

Concept: "Memory Seal" — a compact circular seal/stamp monogram containing a
geometric capital A, suggesting governed memory, review approval, and
publication authority. Thin outer ring with one small notch or tick mark like
an audit seal. Deep ink slate primary; steel-teal accent on the ring notch
only. No wordmark text.
```

---

## Optional follow-up prompts

### Wordmark lockup (after a mark is chosen)

```text
[PASTE GLOBAL CONSTRAINTS]

Create a horizontal logo lockup: the approved AMOS monogram on the left and
the wordmark "AMOS" on the right in a tight modern geometric sans, all-caps,
strong optical spacing. Optional small tagline under the wordmark in lighter
weight: "Private · Auditable · Analyst". Keep the same color system as the
monogram. Generous clear space; no tagline crowding the mark.
```

### Dark variant

```text
[PASTE GLOBAL CONSTRAINTS]

Recreate the same AMOS monogram for dark UI: near-black background (#0E141A),
primary geometry in light slate/off-white, keep the single accent color
unchanged. Same geometry and proportions; do not restyle.
```

### Favicon / 32px readiness

```text
[PASTE GLOBAL CONSTRAINTS]

Simplify the chosen AMOS monogram for 16–32px favicon use: fewer inner lines,
thicker strokes, retain only the strongest silhouette (A + enclosure or seal).
Still recognizable as the same brand mark. No wordmark.
```

### Blend (example: Governed A + Evidence check)

```text
[PASTE GLOBAL CONSTRAINTS]

Blend concepts: geometric capital A inside a rounded square enclosure (Governed
A), with one sealed corner in steel-teal, plus a very subtle olive-teal
provenance check integrated into the crossbar or sealed corner — not a floating
badge. Keep the mark simple enough for app-icon use. No wordmark.
```

---

## Selection guidance

| Priority | Concept | Why |
|---|---|---|
| Primary default | Governed A | Clearest at small sizes; strongest “private / controlled” read |
| Architecture story | Layer A | Most explicit layer metaphor |
| Trust / audit story | Evidence A | Best for verification and claims |
| Docs / authority | Memory Seal | Stamp/approval feel for publication and review |

Prefer one primary monogram. Use secondary concepts only as diagram motifs if needed — do not ship multiple competing logos.
