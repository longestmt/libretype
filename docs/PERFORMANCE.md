# Performance measurements

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
Native verification of this instrumentation is pending.
