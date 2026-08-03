# Store creative generation and evaluation loops: research and evidence survey

Date: 2026-08-02
Scope: reusable generation, rendering, and evaluation infrastructure for app-store screenshots, phone/tablet variants, locales, and feature graphics—not a design recommendation for one app.

## Executive conclusion

Existing tools are strong at one or two jobs: generating store art, capturing/exporting it, validating pixels, or reviewing a finished set. None of the reviewed projects closes the full production and epistemic loop from raw product captures through reproducible multi-device rendering, blind strategic critique, revision, and a controlled market experiment. The suitable Loop-Suite contribution is therefore a **generation-and-learning loop** with an editable plan and deterministic renderer:

1. accept ordered raw product captures and declared product truth;
2. produce several structured creative plans covering copy, sequence, palette, and layout;
3. render real phone, tablet, and feature-graphic PNGs at exact target sizes;
4. block objectively invalid files before subjective review;
5. show anonymized, thumbnail-scale sets to mutually independent lenses;
6. aggregate with visible arithmetic while retaining dissent and provider warnings;
7. feed the winning plan and concrete weaknesses into the next generation round;
8. call the result only an offline recommendation and hand one hypothesis to Apple or Google experimentation.

## Research method

The survey triangulated five kinds of evidence: Loop-Suite repositories, open-source store tooling, snapshot/visual-regression infrastructure, academic literature, and product/design team practices. Official Apple and Google documentation was used for platform constraints. Stars are only ecosystem signals observed during the survey, not quality scores, and can change after this date.

## Loop-Suite pattern review

| Source | Reusable pattern | What changes here |
|---|---|---|
| [Loop-Suite](https://github.com/Loop-Suite) | Independent perspectives, anonymization, blind cross-checking, local deterministic arithmetic, persisted state, prior/refine cycles | Preserve the boundary that deterministic aggregation of model judgments is not externally verified truth. |
| [icon-loop](https://github.com/Loop-Suite/icon-loop) | Render/policy gates, blind critics, cyclic candidate order, Borda count, provider-diversity and unanimity warnings, minority reports | Expand from one icon at several sizes to ordered, multi-target contact sheets. The vision adapter is ported under Apache-2.0 and attributed in `NOTICE`. |
| [aso-loop](https://github.com/Loop-Suite/aso-loop) | Deterministic length/term checks, rubric judging, de-anchoring, held-out checking | Keep ASO text out of scope except where copy and imagery interact visibly. |
| [marketing-loop](https://github.com/Loop-Suite/marketing-loop) | Evidence-aware marketing review and survey structure | Transfer claim/truth scrutiny; do not invent evidence from polished visuals. |
| [research-loop](https://github.com/Loop-Suite/research-loop) | Source verification, explicit limitations, reproducible state | Make the research basis and verdict boundary part of the repository, not hidden prompt lore. |
| [Code-Review-Loop](https://github.com/Loop-Suite/Code-Review-Loop) | Multiple independent review viewpoints and auditable outputs | Apply to creative evidence while keeping file-policy gates separate from judgment. |

## Open-source landscape

| Project | What it demonstrates | Gap this loop addresses |
|---|---|---|
| [app-store-screenshots](https://github.com/ParthJadhav/app-store-screenshots) (MIT) | Browser editor, connected canvases, locales, exact-size exports, agent-oriented workflow | Its quality guidance is largely prescriptive; it does not independently verify strategic judgments or hand off causal testing. |
| [auto-image](https://github.com/Hyunsang-coder/auto-image) (MIT) | The closest concrete repair loop found: manifest → headless render → geometry report → deterministic auto-fix → rerender. It exposes stable issue codes and suggested edits. | Geometry cannot determine value proposition, sequence quality, truthfulness, differentiation, or conversion. This repository adopts the validate/rerun concept, not its code. |
| [Storeshots](https://github.com/eralpozcan/storeshots) (AGPL-3.0) | AI-assisted headlines, palettes, ordering, locales, editor, and exact exports | Generation and editing are not independent evaluation. No Storeshots source is included because its license and product role differ. |
| [fastlane](https://github.com/fastlane/fastlane) (MIT) | Mature capture, framing, localization, and upload automation | Excellent upstream/downstream automation; intentionally not a design-judgment system. |
| [Paparazzi](https://github.com/cashapp/paparazzi), [Roborazzi](https://github.com/takahirom/roborazzi), [swift-snapshot-testing](https://github.com/pointfreeco/swift-snapshot-testing), [ios-snapshot-test-case](https://github.com/uber/ios-snapshot-test-case), [pixelmatch](https://github.com/mapbox/pixelmatch), [screenshot-tests-for-android](https://github.com/facebook/screenshot-tests-for-android) | Deterministic capture and pixel-diff infrastructure across Android, iOS, and generic images | A diff says that pixels changed, not whether the new store story is clearer or causally better. These are compatible upstream regression gates. |

The architecture remains interoperable: external generators and snapshot systems can feed candidate directories, while `storeloop` now also provides a minimal structured planner and deterministic multi-target renderer. Teams can use the entire creation loop or enter at `render`, `validate`, or `review` without abandoning an existing pipeline.

## Academic evidence and design implications

### Rapid first impressions

[Tuch et al.](https://research.google/pubs/the-role-of-visual-complexity-and-prototypicality-regarding-first-impression-of-websites-working-towards-understanding-aesthetic-judgments/) found that visual complexity and prototypicality shape very rapid aesthetic judgments, with stable effects at extremely short exposures. The transfer is not “copy web layouts into app stores”; it is to create thumbnail contact sheets and make first-glance comprehension a separate observation before a critic rationalizes intent.

Marketplace research associates visual listing factors with user decisions: [app-marketplace factor analysis](https://journal.hep.com.cn/fcs/EN/10.1007/s11704-016-5022-8), [icon aesthetics and downloads](https://research.polyu.edu.hk/en/publications/effects-of-the-aesthetic-design-of-icons-on-app-downloads-evidenc/), and [reviews of mobile visual design](https://journals.sagepub.com/doi/10.1177/2050157916639348). These are reasons to inspect the visual surface, not licenses to predict a universal conversion winner from appearance alone.

### Judge bias and correlation

[Large Language Models are not Fair Evaluators](https://arxiv.org/abs/2406.07791) documents position-related evaluation bias. Candidate order therefore rotates by critic. Newer multimodal work also reports evaluation bias in visual judges ([arXiv:2604.18164](https://arxiv.org/abs/2604.18164)). The result should preserve first impressions, evidence, and dissent rather than collapse everything into one unexplained score.

[Nine Judges, Two Effective Votes](https://doi.org/10.48550/arXiv.2605.29800) argues that correlated judge panels can have far less effective diversity than the nominal judge count. This motivates multiple provider families, an explicit single-provider warning, and an additional unanimity warning. Three calls to one model are not presented as three independent human opinions.

### Critique process

Figma's team accounts of [design critiques](https://www.figma.com/blog/design-critiques-at-figma/) and [engineering critiques](https://www.figma.com/blog/how-we-run-eng-crits-at-figma/) emphasize structured feedback, useful context, broad participation, and silent/independent inspection before discussion. In this loop, critics never read one another. Aggregation happens in code, which avoids the first confident reviewer becoming the anchor.

### Personalization and experiments

Netflix's [artwork personalization](https://medium.com/netflix-techblog/artwork-personalization-c589f074ad76) shows why “one objectively best artwork” is often the wrong model: context and audience can change which image works. Related current work continues toward contextual creative selection ([arXiv:2601.02764](https://arxiv.org/abs/2601.02764)). The spec therefore makes audience and locale explicit, and the output recommends a treatment for that declared context only.

Microsoft's experimentation guidance on [trustworthy controlled experiments](https://www.microsoft.com/en-us/research/?p=651963) and [experiment analysis](https://www.microsoft.com/en-us/research/?p=680556) supports pre-declared metrics, guardrails, and careful causal interpretation. Accordingly, model consensus ends at “offline recommendation”; the experiment template owns the conversion claim.

## Official platform and accessibility sources

- Apple: [product page overview](https://developer.apple.com/app-store/product-page/), [screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/app-information/screenshot-specifications/), and [Product Page Optimization](https://developer.apple.com/help/app-store-connect-analytics/acquisition/product-page-optimization/).
- Google Play: [preview asset requirements](https://support.google.com/googleplay/android-developer/answer/9866151?hl=en), [store listing best practices](https://support.google.com/googleplay/android-developer/answer/13393723?hl=en), and [Store Listing Experiments](https://play.google.com/console/about/store-listing-experiments/).
- Large screens: [Android adaptive/large-screen quality guidance](https://developer.android.com/docs/quality-guidelines/archive/adaptive/large-screen-app-quality). Tablet creative should demonstrate a credible tablet experience, not merely scale a phone composition.
- Accessibility: [W3C design-system contrast guidance](https://design-system.w3.org/settings/) is a useful implementation reference, but automated contrast measurement over a composite screenshot is not yet claimed by this MVP. Visual critics can flag risk; deterministic WCAG claims require explicit foreground/background extraction or source design tokens.

Platform rules change. The example spec is a starting configuration, not a substitute for re-checking current official requirements at submission time.

## Resulting architecture

| Stage | Authority | Output |
|---|---|---|
| Source ingest | Local filesystem | Ordered raw-capture manifest and contact sheet |
| Creative planning | Model-assisted, constrained by spec | Multiple editable plans for copy, sequence, palette, and target-aware layouts |
| Multi-target render | Deterministic code | Real phone, tablet, and feature-graphic PNG sets at exact canvas sizes |
| Policy gate | Deterministic code | PASS/BLOCK plus file-level evidence |
| Blind render | Deterministic code | Anonymized per-target contact sheets at controlled thumbnail width |
| Critique | Independent multimodal model calls | First-glance read, sequence read, rubric evidence, target/frame findings, full ranking |
| Quantify | Deterministic code | Hard-gate exclusion, Borda rank, criterion means, dissent, correlation warnings, corroborated risks |
| Regenerate | Winning plan + persisted critique | New variants that preserve strengths and respond to concrete weaknesses |
| Handoff | Human + platform experiment | Registered control/treatment, metric, guardrails, and live result |
| Refine | Persisted state + new independent panel | `STILL_OPEN`, `NEW`, or `NOT_REOBSERVED` observations |

## Non-goals and limitations

- The renderer is intentionally plan-driven rather than a full WYSIWYG editor. The tool does not upload listings, scrape competitors, or choose a universal house style.
- Raw captures and product truth remain required inputs; generation does not invent missing application screens or verified capabilities.
- It does not OCR or fact-check every word against a running application; product truths and prohibited claims must be supplied honestly.
- Critics can miss small text, misread UI, or share training-data biases. Contact sheets reduce presentation variance but do not remove model error.
- Borda count is a transparent tie-breaking rule, not a calibrated probability of market success.
- Corroboration means two critics emitted the same normalized category/target/frame key. Semantically similar findings with different keys may not merge in this MVP.
- `NOT_REOBSERVED` is not `FIXED`. Only targeted source inspection or market/user evidence can close a risk.
- A live experiment can still be underpowered, contaminated, seasonal, or poorly segmented. The generated handoff is a checklist, not an experimentation platform.

## License and reuse notes

The repository is Apache-2.0. `src/llm.rs` is derived from the Apache-2.0 `icon-loop` adapter and is identified in `NOTICE`. Other open-source projects above informed capability boundaries and interoperability; their code was not copied. In particular, AGPL-licensed Storeshots code is not included.
