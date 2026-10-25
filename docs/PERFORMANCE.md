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
