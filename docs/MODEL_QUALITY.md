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
