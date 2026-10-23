# Model licenses

LibreType is GPL-3.0-only. Models and the separately installed Ollama service have
their own licenses; neither weights nor Ollama are included in the app/source
release. The Local models window shows terms before download and links to them.
Existing selected models are preserved. The starting recommendation is Qwen3
1.7B, including on smaller-memory machines; actual memory needs and quality vary.
See [exploratory completion results](MODEL_QUALITY.md) for observed speed and
quality trade-offs. The starting recommendation is a lightweight option, not a
claim that it produces the best writing.

| Catalog family | Tags | Upstream license |
| --- | --- | --- |
| Qwen3 | 1.7b, 4b, 8b, 30b | Apache-2.0 |
| Gemma4 | e2b-it-q4_K_M, e4b-it-q4_K_M, 26b-a4b-it-q4_K_M | Apache-2.0 |
| Gemma3 (optional) | 1b, 4b | Gemma Terms of Use, including use restrictions |

Gemma3 is an optional model with restrictions, not unrestricted FLOSS weights.
Its restrictions do not apply to the application code. Downloading it requires an
explicit choice after the model notice; review the linked terms first.

Verified upstream family information on 2026-10-23:

- Qwen3: https://huggingface.co/Qwen/Qwen3-1.7B/blob/main/LICENSE
- Gemma4: https://ai.google.dev/gemma/docs/core/model_card_4
- Gemma3: https://ai.google.dev/gemma/terms/

These family declarations do not certify every quantized artifact or tokenizer.
Before mirroring or bundling weights, record the exact artifact digest, review
that variant's complete license/notice files, and satisfy its redistribution
conditions. Additional models installed outside the catalog need their own review.
