# Third-party software and models in Notsy

## llama.cpp

- **What:** runs the writing model inside `notsy-ai.exe`. It is linked statically, CPU only.
- **Source:** https://github.com/ggml-org/llama.cpp, tag `b11284`.
- **License:** MIT. Copyright (c) 2023-2026 The ggml authors. The full text is in `LICENSE-llama.cpp.txt`.

## LFM2.5-1.2B-Instruct (Liquid AI)

- **What:** the writing model, `models/notsy-writer-1.2b-Q4_K_M.gguf`. It is built on LFM2.5-1.2B-Instruct. A build for smaller PCs ships `models/notsy-writer-350m-Q4_K_M.gguf` instead, built the same way on LFM2.5-350M (https://huggingface.co/LiquidAI/LFM2.5-350M).
- **Modified:** yes. Notsy fine-tuned it (LoRA, merged into the weights) to correct typos, spelling and grammar in notes, on text from Tatoeba (CC BY 2.0 FR) and Wikipedia (CC BY-SA 4.0) with mistakes put in. It was then quantized to Q4_K_M. The training code is in `tools/train`. Liquid AI has not reviewed or endorsed these changes.
- **Source:** https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct
- **License:** LFM Open License v1.0. The full text is in `models/LICENSE-LFM.txt`.
- **Commercial use:** free below the license's revenue threshold of $10,000,000 a year. Above it, a separate license from Liquid AI is required.
- **Where it runs:** on this computer, only on the note that is open. Nothing it reads or writes leaves the machine.

## Simple Icons

- **What:** the YouTube mark in the social preview (`crates/notsy-ui/src/vector.rs`).
- **Source:** https://simpleicons.org
- **License:** CC0 1.0 (public domain). The mark itself is YouTube's trademark, shown only to say which platform a preview is for.

## Carbon Icons

- **What:** the clock beside Recent in the side menu (`recently-viewed`, in `crates/notsy-ui/src/vector.rs`; the SVG is `assets/icons/carbon/recently-viewed.svg`), found through https://allsvgicons.com.
- **Source:** https://github.com/carbon-design-system/carbon (IBM).
- **License:** Apache 2.0. Copyright IBM Corp.

## Lucide and Qlementine Icons

- **What:** the shapes of the app's icons on the Mac, iOS and Linux, drawn from `assets/fonts/NotsyIcons.ttf`, a font that `tools/iconfont` builds from the open (outlined) SVGs in `assets/icons/`, found through https://allsvgicons.com. Windows draws the same icons from Segoe Fluent Icons, which ships with Windows.
- **Lucide** (most icons): https://github.com/lucide-icons/lucide. License: ISC. Copyright (c) 2026 Lucide Icons and Contributors. The full text is in `assets/icons/lucide/LICENSE`.
- **Qlementine Icons** (`new-24`, the new-note icon): https://github.com/oclero/qlementine-icons. License: MIT. Copyright (c) 2023 Olivier Cléro. The full text is in `assets/icons/qlementine-icons/LICENSE`.
