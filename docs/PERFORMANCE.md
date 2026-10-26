# Performance measurements

## Automatic analysis yields to typing

The current source cancels an automatic history-profile analysis when keyboard
activity resumes. Previously the five-second idle check governed only when an
analysis started; a running analysis could continue alongside new completion
requests. Explicitly requested analysis remains active, and the automatic
scheduler retains its sixty-second retry interval to avoid repeated restarts.

The history worker invalidates the inference request before queuing cancellation
and restores its ready snapshot without rewriting the vault. Deterministic tests
hold the analyzer open, interrupt it, verify the previous encrypted snapshot is
unchanged, then release a delayed result and verify it cannot replace the saved
profile. A subsequent manual build still completes. Completed automatic jobs and
locking are covered as well. These tests use invented writing and a temporary
key, without accessing application history or Keychain.

The release-mode `profile_interrupt_probe` subsequently exercised the production
worker and local profile generator with invented samples in a temporary encrypted
vault. Interrupting three active calls after 150, 300 and 600 ms produced canceled
results 2.94, 14.67 and 21.67 ms after interruption. The encrypted snapshot remained
byte-for-byte unchanged. Each following production completion returned usable
text, in 1.69, 1.98 and 0.40 seconds respectively.

This used Ollama 0.40.11 and the locally installed `gemma4:e4b` alias, which now
resolved to an MLX/Safetensors model with digest
`eb2177adf7646d15faead94fdad6d3e170dc8015653418a3b7d5d52a6380be5b`.
That differs from the model behind the earlier measurements below. The probe
did not replace model weights or change application settings. Repeated prompts,
model loading and other desktop workloads were not controlled. These three
observations establish client-call cancellation and subsequent successful
inference, not server-side GPU release time or a comparative latency benefit.
The change is installed in build 64, whose Accessibility and keyboard listener
checks are active. Interruption of a live automatic analysis by native keyboard
input has not yet been exercised.

```sh
cargo run --release --locked --example profile_interrupt_probe -- gemma4:e4b
```

### Paired foreground-contention check

On 2026-10-26, `analysis_contention_probe` compared idle inference with canceled
and continuing profile analysis on the same MLX artifact above. It used the
production profile generator, completion pipeline and cancellation epoch, with
three invented analysis samples and three plain end-of-text completion contexts.
It accessed no saved writing, Keychain or application configuration and wrote no
profile to the installed app.

One analysis and all three completion contexts were warmed first. Six rounds
covered all permutations of the three conditions. Each context was used twice,
with the cancellation/continuation order reversed. Analysis remained in flight
after 300 ms in every contention trial. Foreground inference started immediately
after the cancellation decision, without first waiting for the canceled client
to exit. Each round used identical foreground text for its three conditions.

| Condition | Foreground median | Observed range | Trials |
| --- | ---: | ---: | ---: |
| No background analysis | 0.41 s | 0.39–0.66 s | 6 |
| Background analysis canceled | 0.53 s | 0.42–0.72 s | 6 |
| Background analysis allowed to continue | 4.26 s | 3.88–4.68 s | 6 |

Cancellation reduced the foreground request interval in all six paired trials,
by 3.36–4.04 seconds. Every foreground result was nonempty and identical across
the three conditions within its round. All six canceled analysis calls reported
cancellation, exiting 5.73–25.33 ms after the epoch change; all six continuing
calls produced valid profiles. The model digest and Ollama version were unchanged
before and after the study. The probe completed 21 foreground calls including
warmups, seven completed analyses and six canceled analyses, without unexpected
errors.

This establishes an observed request-latency benefit for the tested contention
workload. It does not directly measure GPU release time, CPU or battery use,
model cold starts, or native typing-to-draw latency. The native keyboard event,
Engine command routing and debounce are not part of this probe; those paths have
separate tests, and the live automatic-analysis keyboard check remains pending.
Foreground requests used no attached learned profile or optional context.
Repeated prompts could benefit from cache state; other desktop workloads and
server caches were not isolated. These six paired observations are not a general
latency guarantee for other models, longer documents or personalized completions.

```sh
cargo run --release --locked --example analysis_contention_probe -- gemma4:e4b
```

## Synthetic inference and process resources

On 2026-10-24, a release-mode production-pipeline probe ran four invented
contexts in three rotated rounds on the 16 GB Apple M1. The contexts covered
plain continuation, a partial word, mid-line insertion and explicit writing
instructions. All 12 requests succeeded. The selected Gemma4 E4B model was
already loaded before the measurement and remained loaded afterward; no model
was unloaded and no application settings were changed.

| Request kind | Observed request-time range |
| --- | --- |
| Plain continuation | 0.26–0.30 s |
| Partial word with following text | 1.89–2.17 s |
| Mid-line insertion | 0.77–1.65 s |
| Explicit writing instructions | 0.61–1.73 s |

The overall median was 1.15 seconds and the workload interval was 14.31 seconds.
Each interval covers one `model::generate` call, including metadata validation
and any fallback. It excludes typing debounce, native text capture, rendering
and acceptance. Repeated prompts can benefit from cache state. These are four
contexts, not twelve independent writing-quality examples, and the desktop's
other workloads were not isolated.

Process RSS was sampled approximately every 250 ms:

| Process | Sampled resident-memory range |
| --- | --- |
| Installed LibreType build 47 | 36–40 MiB |
| Ollama server | 16–42 MiB |
| Ollama model process | 3.78–3.97 GiB |
| Synthetic probe | Up to 8.5 MiB |

The app's observed CPU-time increase was 0.15 seconds, the Ollama server's
0.21 seconds and the model process's 6.57 seconds. The installed app was running
alongside the probe; the probe supplied the measured requests. This is not an
active-typing benchmark of the installed app.

RSS can include shared or unified GPU memory, so the rows must not be added
and presented as unique physical memory. Sampling can miss peaks, and process
CPU time does not measure GPU energy. Ollama reported a 3,226,332,036-byte model
allocation with a 2,048-token context; that allocation is a different metric
from RSS. Battery impact, system-wide footprint, cold loading and full native
typing latency remain unmeasured by this check.

Exact evaluated model digest:
`c6eb396dbd5992bbe3f5cdb947e8bbc0ee413d7c17e2beaae69f5d569cf982eb`.

## Reproduce

Build the synthetic probe, identify the running LibreType executable and Ollama
server process IDs, then run the sampler. Replace the two example PIDs below
with the actual process IDs. The script checks executable names, uses only
numeric loopback for model metadata, and refuses to overwrite an existing report.

```sh
cargo build --locked --release --example completion_probe
python3 scripts/measure_inference.py --model gemma4:e4b \
  --app-pid 12345 --server-pid 23456 --output /tmp/libretype-resources.json
```

The script records the initial/final loaded-model metadata, individual synthetic
outputs and timings, process samples and limitations. It reads no writing,
history, clipboard, screen content or app settings. Generated reports belong
outside the published source tree. The probe has a 120-second workload budget;
on failure it terminates only its own child process, not the app or Ollama.

## Native typing-to-draw instrumentation

Development builds with `last_typing_latency` in the health snapshot retain one
numeric sample for an ordinary keyboard event followed by a matched native
capture, a successful model completion and a visible ghost-view draw. No key
values, writing, wall-clock timestamps or foreground-app identities are stored
in this sample. In-flight marks are discarded on missing capture, changed
context, cancellation or a different foreground process. Manual activation,
preview, emoji/spelling replacement and retained-word redraws are not trials.

The phases are event receipt → first valid capture → request preparation starts
→ model worker starts → worker finishes → main-thread reply receipt → first
visible AppKit draw. They sum to `event_to_draw_ms`. The capture interval includes
polling; the next includes debounce; request-to-worker includes optional context
collection and queueing. The worker interval includes model metadata checks and
fallbacks. This does not measure hardware-to-photon latency: OS delivery before
the event tap and final compositor/display scanout are outside the measurement.

Use a disposable editor, type a known phrase yourself, then leave it focused
until ghost text appears. Wait several seconds before starting another trial.
Browser-simulated keys can bypass the system listener and are unsuitable for
this measurement. Run the bounded sampler with the installed process ID:

```sh
python3 scripts/measure_typing.py --pid 12345 --samples 3 --seconds 120 \
  --output /tmp/libretype-typing.json
```

The sampler ignores the pre-existing sample, validates numeric fields and phase
sums, refuses another process or an existing output path, and exports only the
new timing samples. Health snapshots refresh about once per second, so rapid
samples may be missed. Missing/canceled/failed completions are not included; a
small controlled sample is not a population latency or battery benchmark.

### Native Chrome check, build 58

Three successful typing-to-draw samples were captured in disposable Chrome
textareas after Accessibility and the keyboard listener were active. The user
typed the three displayed phrases and confirmed completion of the check.

| Sample | Event to capture | Capture to request | Model worker | Reply to draw | Event to draw |
| --- | --- | --- | --- | --- | --- |
| 1 | 60 ms | 352 ms | 6,978 ms | 43 ms | 7,496 ms |
| 2 | 73 ms | 357 ms | 1,616 ms | 17 ms | 2,083 ms |
| 3 | 85 ms | 314 ms | 1,650 ms | 19 ms | 2,097 ms |

Request-to-worker was below 0.03 ms; worker-to-reply was 20–63 ms. Generation,
including its metadata checks and possible fallback, dominated all three
samples. The larger first interval was not isolated to model loading: loading,
cache state and other desktop workloads were not controlled, so its cause is
undetermined. These are samples of successful draws, not per-phrase latency
guarantees or a complete success-rate measurement. The sampler stores no text
with which to identify the particular keystroke that began each sample.

The selected model was Gemma4 E4B with the digest listed above. Settings were
unchanged: 250 ms configured debounce, 1,200 context characters, eight maximum
words, and existing explicit writing instructions. History, clipboard and
screen context remained off. The native tracker and numeric-only sampler are
now exercised together; broader latency, battery and controlled resource
comparisons remain outstanding.

### Generation-phase follow-up

A separate release-mode probe on the same model ran four invented contexts
(plain continuation, explicit instructions, a partial word and mid-line
insertion) in three rotated rounds. All 12 requests completed successfully;
each case returned the same text across its three rounds. Temporary source
instrumentation measured metadata validation at 9–16 ms. Time to first visible
content on successful generation paths ranged from 137 to 1,290 ms. Partial-word
requests first rejected the raw continuation and used the existing structured
fallback; the first-content range excludes those rejected attempts.

First-round request times were 1.21–1.83 seconds, versus 0.26–1.34 seconds in
later rounds. Repeated prompts can benefit from cache state; model loading and
other workloads were not isolated. This does not explain the earlier 7.50-second
native sample or establish a cold-start penalty. Metadata validation was a small
part of these requests, so no caching or removal of the local-model check was
adopted. The installed application and settings were unchanged; temporary tracing
is not included in application builds.
