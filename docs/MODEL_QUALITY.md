# Exploratory completion quality

On 2026-10-23, the production completion pipeline was exercised with 16 invented
contexts, each repeated twice in shuffled order, on a 16 GB Apple M1. The cases
covered contextual facts, English and multilingual grammar, partial words,
existing text after the caret, technical identifiers and writing instructions.
No personal writing, learned profile, clipboard or screen context was used.

| Installed model | Median request time | First request | Observations |
| --- | ---: | ---: | --- |
| Gemma4 E4B (`gemma4:e4b`) | 1.89 s | 6.66 s | Nonempty insertions in 15 cases; supplied the appropriate Spanish verb and preserved the stated venue. One technical continuation was generic. |
| Qwen3 1.7B | 0.45 s | 2.16 s | Fastest; no usable insertion in three additional mid-line cases. Some continuations repeated context, introduced an unstated time or used an unwarrantedly intimate tone. |
| Qwen3 4B | 1.04 s | 3.51 s | Filled the missing gaps, but used Spanish `son` where the intended meaning required `están`, and introduced an unstated meeting time. |

All models correctly returned no insertion when the caret split the already
complete word `documentation`. All 16 outputs per model were identical in the
two rounds, with no transport errors. That repetition is not 32 independent
quality observations. Review was qualitative and not blinded; no aggregate
accuracy score is claimed. A French case also allowed a grammatical reading
without an inserted adjective, so an empty result alone is not a quality failure.

The Qwen3 1.7B missing-word responses were inspected separately: the model omitted
the words, rather than generating valid insertions that LibreType discarded.
Adding an explicit grammar reminder did not change the eight mid-line outputs
tested. That candidate was not adopted. Validation continues to require exact
preservation of existing characters around the caret.

These results support retaining an existing Gemma4 E4B choice for this writing
sample, while acknowledging the smaller models' speed advantage. They do not
establish a universal best model, and did not change the application default or
the user's selected model. The newer catalog tag `gemma4:e4b-it-q4_K_M` was not
tested and must not inherit these quality results merely from its name.

The measured interval is one `model::generate` call, including local metadata
checks and any fallback, but excluding typing debounce, Accessibility capture
and display. Requests used the production defaults: temperature 0, seed 42,
2,048-token context and at most eight output words. Models ran in separate
blocks, without controlled unload, cache isolation or workload isolation.
First-request timings are observations, not a verified cold-start benchmark.
This check does not measure battery use, personalization quality or native
editor insertion, and makes no comparison against another application.

Exact downloaded model digests:

- `gemma4:e4b`: `c6eb396dbd5992bbe3f5cdb947e8bbc0ee413d7c17e2beaae69f5d569cf982eb`
- `qwen3:1.7b`: `8f68893c685c3ddff2aa3fffce2aa60a30bb2da65ca488b61fff134a4d1730e7`
- `qwen3:4b`: `359d7dd4bcdab3d86b87d73ac27966f4dbb9f5efdfcc75d34a8764a09474fae7`

The Qwen3 4B catalog download completed through the production worker and was
verified as local GGUF. Its downloaded size was 2,497,293,931 bytes, consistent
with the [registry's approximate 2.5 GB size](https://ollama.com/library/qwen3:4b).
Weights are separate from the application and are not publication inputs.

## Exploratory personalization check

On 2026-10-24, `examples/style_probe.rs` exercised the production profile builder
and completion pipeline with three invented samples and matching American- and
British-spelling versions. It never reads application settings, stored writing or
Keychain. The initial check used five contexts and five conditions: influence Off,
each learned profile at Strong, a direct spelling-only profile as a diagnostic
control, and an explicit American-spelling override of the British-sample profile.

With Gemma4 E4B, the original builder omitted spelling conventions from both
profiles. For the unseen words colour, favourite and cancelled, the learned
British-sample profile returned American spellings; the direct British-spelling
control returned the intended British spellings. This distinguishes an extraction
weakness from the model's ability to follow that spelling preference directly.

Two extraction-prompt candidates added spelling analysis. They improved some
outputs through recurring vocabulary but still omitted the British convention
from guidance and still returned `color`. Two extra contexts, honour and cheque,
were added before the first candidate run and retained for the second; they are
fresh observations only for that first run. The candidates also exposed an
override failure: the learned British-sample profile returned `cheque` despite
an explicit American-spelling instruction. With no profile, the same explicit
instruction returned `check`; a British instruction returned `cheque`.

Neither prompt candidate was adopted. The retained probe makes these limitations
reproducible for further work. The labour context was ambiguous enough to allow
`laboratory equipment`, so it is not scored as a spelling failure. The unrelated
dinner context produced plausible but different meals; this does not establish
topic independence across arbitrary writing.

The study comprised 25 original completions, 35 per candidate, two explicit-only
controls and six profile builds, without transport errors. These are exploratory,
unblinded development observations on one model, not an accuracy score or proof
of broad style quality. All data were synthetic and transient to the production
pipeline; history recording and the installed application were unchanged.

### Frozen-profile conflict checks

`examples/preference_conflict_probe.rs` isolates completion behavior from profile
extraction with fixed invented profiles and seven contexts/conditions. On the
same installed model digests listed above, all three models returned `cheque`
despite an American-spelling instruction when the spelling profile was active.
With influence Off, Gemma4 E4B and Qwen3 4B returned `check`; Qwen3 1.7B returned
no insertion. All three respected an explicit Morgan recipient and retained
Ariadne when compatible with a concise-writing instruction. Gemma4 E4B and
Qwen3 1.7B honored an explicit `Kind regards` sign-off; Qwen3 4B instead selected
the profile's recurring sign-off.

Three Gemma4 E4B experiments tested a separate system message, moving observations
into reference context, and reducing profile influence to a tie-breaker whenever
explicit preferences were present. None fixed the spelling conflict. The reference
variant also regressed the sign-off override. All three were reverted; no profile
strength, request format or model-selection change was adopted. The 42 completions
(seven baseline, seven per candidate, seven per Qwen model) had no transport errors.
This remains a known model-compliance limitation, not a claim that every explicit
preference is unreliable or that any model is universally preferable.

### Larger-model follow-up

Qwen3 8B completed the same seven frozen-profile cases after a production-worker
download and local GGUF verification. It still returned `cheque` despite the
American-spelling instruction and additionally returned `colour`, where the
three previously tested models returned `color`. Influence Off returned `check`;
recipient, sign-off and compatible-name cases returned the expected terms.
All seven requests completed without transport errors. The median request took
1.87 seconds and the first took 6.57 seconds; these are observations from one
ordered run, not a controlled performance comparison.

This gate provided no reason to replace the selected Gemma4 E4B model, so no
broader writing benchmark or application model change followed. The evaluation
model was unloaded after the check; its downloaded files remain available.
The artifact was 5,225,388,164 bytes, Q4_K_M, with exact digest
`500a1f067a9f782620b40bee6f7b0c89e17ae61f686b92c24933e4ca4b2b8b41`.
Application settings, history and privacy controls were unchanged.

### Two-pass preference experiments

On 2026-10-25, two isolated Gemma4 E4B experiments reused the seven frozen cases.
A post-generation review supplied the preliminary completed passage alongside
the explicit instruction and asked for only necessary revisions. It preserved
the compatible-name control but still returned `cheque` under the American
spelling instruction. This added a second request without resolving the failure.

A separate pre-generation filter asked the model to select compatible guidance
and vocabulary indices. The program accepted only unique, in-range indices and
retained the original entries verbatim; it could not invent new profile entries.
This fixed the cheque example, but the filter dropped Ariadne under the unrelated
concise-writing instruction. The resulting completion invented an email address
instead of preserving that compatible name. The selector also retained several
regional spellings despite the explicit spelling preference. Valid JSON and valid
indices therefore did not establish correct semantic conflict resolution.

Neither approach was adopted. The review experiment made 12 generation requests;
the filter experiment made 17, including its five selection requests. No transport
errors occurred. Eight new holdout contexts were prepared before inspecting the
filter results but were not run because the frozen preservation control failed.
These were synthetic, transient tests, with no history access, configuration
changes or installed-app changes. Personalization quality remains unresolved;
discarding compatible learned preferences is not an acceptable fix.

### Explicit-preference reminder with personalization

A later check found that personalization bypassed the final completion reminder
used for explicit instructions. Simply applying the existing reminder to learned
profiles fixed the frozen Gemma4 E4B cheque conflict and retained the six other
controls. However, one of eight previously prepared holdouts regressed: instead
of completing `behav` as `behavior`, it added `ior after travelling`. The unchanged
baseline returned only `ior`. That first candidate was rejected.

The adopted reminder explicitly names the scoped writing instructions and limits
learned style/vocabulary to compatible, necessary uses. It retains all learned
entries, uses the existing strength, and adds no model request. Gemma4 E4B passed
the seven frozen cases and the eight expanded cases. Eight further contexts,
defined before the expanded results were inspected, retained the expected names,
sign-offs, device names and dates. One spelling case still returned `neighbour`
despite an American-spelling instruction; the unchanged baseline failed the same
case. This is a remaining limitation, not a solved spelling-precedence guarantee.

A Qwen3 4B cross-check retained the compatible-name controls and corrected its
previous sign-off override failure to `Kind regards,`. Its cheque conflict remains.
These observations support an incremental improvement on the two tested models,
not universal instruction compliance or comparative personalization quality.
There were 61 new completion requests across the rejected candidate, adopted
candidate and baseline checks, with no transport errors. Optional terminal
punctuation was allowed for end-of-text sign-offs. Review was unblinded and each
case/condition ran once; no aggregate accuracy claim is made.

The reproducible synthetic probes are:

```sh
cargo run --release --locked --example preference_conflict_probe -- gemma4:e4b
cargo run --release --locked --example preference_holdout_probe -- gemma4:e4b
cargo run --release --locked --example preference_holdout_probe -- gemma4:e4b fresh
```

Request-level regression tests cover global, app and website instructions with
personalization on and off, retaining learned vocabulary in the outgoing prompt.
Those deterministic tests verify routing and preservation, not model semantics.
These experiments did not load history, change application preferences or replace
the installed application. Profile extraction quality remains separate work.

### Bounded spelling-observation experiments

Two later experiments supplemented model-built guidance with a conservative
word-family detector operating only on the profile builder's eligible samples.
It required at least three distinct spelling families, each in two distinct
samples, and rejected mixed evidence. Repeated case/spacing/punctuation variants,
URLs and identifiers did not supply independent evidence. Deterministic checks
also covered source exclusions, retention and the profile length bound.

For each experiment, two profiles were built from the existing invented British
and American samples. Each completion had a paired control with only the added
spelling observation removed; model-generated guidance and vocabulary stayed
identical. The first observation improved default colour/favourite/honour output
and the American check context, but worsened explicit American overrides for
favourite, honour and cheque. Conditional fallback wording restored the favourite
and cheque overrides but still returned honour where its paired control returned
honor. Neither detector variant was adopted.

The two experiments made four profile builds and 84 completions on Gemma4 E4B,
without transport errors. The labour sentence allowed a laboratory-equipment
reading and was not scored as a spelling error; dinner suggestions varied
plausibly and do not establish topic independence. Additional centre, flavour,
neighbour and behaviour contexts were prepared before inspecting results but
were not executed because the override gate failed. All writing was synthetic;
application history, settings and the installed build were untouched. This
narrows the next investigation to combining learned conventions with explicit
instructions, rather than merely extracting more spelling evidence.

### Vocabulary spelling priority

A source review found that the recurring-vocabulary prompt told the model to
preserve every entry's spelling, even when explicit writing preferences requested
a different convention. A candidate that distinguished ordinary vocabulary from
proper names fixed the earlier neighbour conflict, but in a new recipient check
it replaced the baseline's Leonie with a venue. That candidate was rejected.

The smaller adopted change replaces only the blanket spelling directive with
“Preserve names and identifiers.” It keeps every learned entry, the existing
influence strength, scoped instructions and completion format, without adding a
model request. On Gemma4 E4B it changed the earlier American-spelling case from
`neighbour` to `neighbor` and preserved the other frozen/expanded controls.

Twenty additional contexts compared the changed prompt with the unchanged
baseline. Twelve were defined before the first candidate; eight further contexts
were defined before inspecting the smaller candidate's results. The smaller
candidate's outputs were identical to baseline across these twenty contexts.
They retained the tested proper names and compatible person, but a profile that
explicitly described British spelling still defeated some American-spelling
requests. Both versions also invented a generic variable name and missed a
recurring sign-off in the longer vocabulary profile. These are unresolved limits,
not passing checks or evidence of universal instruction priority. Reference
insertions in the probe guide manual review; grammatical number, quotation marks
and optional punctuation can differ without proving a preference failure.

The reproducible additional checks are:

```sh
cargo run --release --locked --example vocabulary_priority_probe -- gemma4:e4b
cargo run --release --locked --example vocabulary_priority_probe -- gemma4:e4b fresh
```

This investigation made 114 synthetic completion requests, including the rejected
candidate and paired controls. One Inez preservation request returned a local
transport error; the complete eight-case group was repeated successfully, with
Inez retained. The failed request remains in the evidence. These are unblinded,
single-context observations rather than an aggregate accuracy score. No user
history or application settings were loaded, and the installed app was unchanged.
The source's deterministic all-target suites and Clippy check passed.


### Separate system-message experiment

Two later synthetic candidates used Ollama’s documented `system` field for
resolved explicit preferences when personalization was active. The first kept
those preferences in the original completion prompt as well; the second moved
them out of the inline profile and final reminder. Both retained learned
vocabulary, influence strength, output validation and the single-request design.
The API supports this field independently of the completion prompt; that does
not guarantee model compliance ([Ollama API](https://docs.ollama.com/api/generate)).

The duplicated variant did not fix the four development spelling conflicts,
generic identifier or missed recurring sign-off. The separated variant recovered
the learned `colourScale` identifier, but failed the earlier preservation gate:
it changed the American-spelling cheque completion from `ck` to `que` and added
“is grey, and it” inside a sentence whose existing suffix said “is blue.” A fresh
unchanged-baseline run passed both cases. The limited identifier improvement
therefore did not justify adopting either candidate.

This check used Gemma4 E4B with Ollama 0.40.8 and produced 57 completed synthetic
responses with no returned errors. One additional in-flight candidate request
was cancelled after the regression was found; seven completed expanded cases
remain in the evidence. The remaining expanded cases and eight prewritten new
holdouts were not run. These are unblinded development comparisons, not an
accuracy estimate. No history, settings or installed application changed.
Explicit-preference priority and learned-sign-off quality remain unresolved.


### Additional local models and MLX compatibility

On 2026-10-26, the already-downloaded Qwen3.5 2B Q4_K_M model completed the
seven frozen-profile checks and twelve longer-profile cases through the unchanged
production pipeline. It returned `cheque` despite the American instruction with
influence both on and off, used the learned sign-off instead of the requested
one, and supplied a technical term where the compatible recipient was expected.
Six calls returned no usable insertion. All nineteen calls completed without a
transport error; the median interval was 1.11 seconds and the first was 5.51
seconds. This did not justify changing the selected model.

An isolated compatibility candidate then allowed the already-downloaded Gemma4
12B NVFP4 model through Ollama’s local MLX runner. The seven frozen checks retained
the expected spelling, recipients and sign-off. In the twelve longer-profile
cases it honored American spelling for neighbour, travelling and organise and
used the recurring sign-off verbatim, but returned no usable insertion for the
favourite case and invented `x` rather than the learned identifier. This is a
limited quality improvement, not a solution to all personalization failures.

Basic backend checks exposed empty plain-boundary results on the raw continuation
path. The MLX candidate now uses a templated, structured completion request for
unconfigured writing as well. The six-case rerun produced valid plain English,
Spanish, partial-word and mid-line insertions and respected app/site instructions.
A further live check returned five contextual synonyms and a valid profile from
three invented samples. The profile retained the specialist term and recurring
sign-off but omitted the recurring person's name; valid analysis is not evidence
of complete extraction. These were 31 MLX completion calls plus one synonyms
call and one profile call, with no returned transport errors.
GGUF routing remains unchanged. The same metadata rule is used for discovery,
download verification and the pre-generation check: MLX requires Safetensors, an
explicit MLX runner and completion capability, with no remote reference. Unknown
formats and malformed remote fields are rejected before writing is sent.

These checks used Ollama 0.40.8 on the existing 16 GB M1. MLX completion intervals
varied substantially, including several over ten seconds; concurrent desktop
work and caches were not controlled. Ollama reported 8,956,851,444 bytes of loaded
allocation at a 2,048-token context, which is not a total-memory or peak-memory
measurement. No default or user model selection changed, no new weights were
downloaded, and no personal writing or history was used. Native editor use with
MLX remains unverified.

Exact artifacts used:

- `qwen3.5:2b-q4_K_M`: `1e2d21a4f03ab9e4a9c1976a4b7e88f21de61d65f2c5485d8fde8942be73f1e7`
- `gemma4:12b`: `c7597fc90b86aff6965317f37dd5465402284e26382d9b4bf2dfe3ef1491f4b6`

Ollama documents its [MLX backend](https://ollama.com/blog/mlx-performance); that
backend support does not by itself establish completion quality or speed.
