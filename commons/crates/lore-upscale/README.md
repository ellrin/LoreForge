# lore-upscale

Shared image upscaler (`docs/img2img.md`, phase 1): the Real-ESRGAN **ncnn-vulkan** CLI
(BSD-3-Clause; ncnn wrapper MIT) as a child process.

```rust
let mut o = Options::new(Model::AnimeX4, 4);          // or AnimeVideo (x2/x3/x4), GeneralX4
o.install = Some(Install::in_home(&model_home));      // None: LOCAL_LLM_HOME / engine.toml [local] home
o.runtime = SharedRuntime::process("sc");             // take the runtime's slot; None: do not
upscale_with(in, out, &o, &mut |percent| { /* 0..=100 */ })?;   // o.cancel.cancel() from elsewhere
```

- **Not in the repo.** `<home>/runtimes/realesrgan/` holds the binary, `models/`,
  `LICENSE-Real-ESRGAN.txt` (BSD-3), `LICENSE-Real-ESRGAN-ncnn-vulkan.txt` (MIT) and `install.json`.
  `download(&Install, &Source::official(), &cancel, on_progress).await` installs it: pinned
  official release URL + SHA-256 (checked before unpacking), only the binary and `models/*`,
  quarantine attribute removed, chmod 755, atomic swap (a failure keeps the old install). A UI
  calls it only when the author presses 下載.
- Missing binary/models: `UpscaleError::NotInstalled { dir, missing }`. Bad `scale`/`tile`/extension: `Options`.
- Output is written to a hidden part file and renamed on success: cancel or failure never leaves a half image or replaces an existing one.
- Slot: `lore_llm::shared_runtime::begin_tool_blocking` (`docs/local-models.md` 7a): the local model's requests wait while it runs.
- `cargo run -p lore-upscale --example upscale -- in.png out.png [--model anime|video|general] [--scale N] [--tile N] [--dir D] [--download]`
- Tests: fake CLI + local fake download server. Real binary: `REALESRGAN_DIR=<folder> cargo test -p lore-upscale --test upscale -- --ignored`.
