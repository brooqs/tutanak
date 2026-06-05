# TODOS — tutanak

## Roadmap (next)

- [ ] **CI** — GitHub Actions: `cargo test` + `cargo clippy` on every push.
- [ ] **AppImage release** — tag-triggered workflow that packages `tutanak-ui` and
  attaches it to a GitHub Release.
- [ ] **In-process whisper.cpp STT engine** — the `whispercpp` provider profile
  already exists; implement the `Transport::WhisperCpp` arm in `engine::build_stt`
  so STT can run with no server (CPU, and Metal/CoreML on macOS).
- [ ] **Settings UI: add/remove providers** — currently the panel edits the two
  selected profiles; allow creating/deleting `[providers.*]` entries.
- [ ] Minor: summary shows raw markdown bullets; optional simple cleanup/render.

## Cross-platform (Windows / macOS)

Goal: ship beyond Linux. ~90% is already portable (core pipeline, OpenAI-compatible
engines, Slint GUI, config, storage via the `dirs` crate). The only platform-bound
piece is audio capture.

- [ ] **Refactor `core/src/capture.rs` into a trait + per-OS implementations.**
  - Keep the existing public API (`Source`, `Session`, `start`, `stop`) so the CLI
    and GUI don't change. Split impls behind `#[cfg(target_os = "...")]`:
    - `capture/linux.rs` — current PulseAudio null-sink + loopback mix (parecord/pactl).
    - `capture/windows.rs` — WASAPI loopback (render endpoint) + mic mix; `cpal` or
      the `wasapi` crate.
    - `capture/macos.rs` — ScreenCaptureKit (macOS 13+) or a virtual audio device
      (e.g. BlackHole) for system audio; CoreAudio/`cpal` for mic.
  - Mirrors the provider-registry pattern (decoupled, swappable). Do this early so
    the boundary is clean before adding platforms.
  - **Depends on:** nothing; can be done now with Linux as the single impl.

- [ ] **Windows port** (recommended as the second platform).
  - WASAPI loopback is native and well-supported.
  - FastFlowLM (AMD NPU) is Windows-first → the NPU local-pipeline story already works.
  - Distribution: `.msi`/`.exe` (cargo-wix) or a portable zip.

- [ ] **macOS port** (hardest, do last).
  - System-audio capture is restricted: needs ScreenCaptureKit audio or a virtual
    device; mic alone is easy.
  - No AMD NPU → FastFlowLM N/A; but whisper.cpp on Apple Silicon (Metal/CoreML) is
    excellent → pair with the in-process whisper.cpp engine above for a strong local
    STT story.
  - Distribution: `.dmg`/`.app` with codesigning + notarization (the real chore).

## Deferred (v1.1+)

- [ ] **Flatpak portal vs monitor-capture investigation spike**
  - **What:** Build a trivial Flatpak that does nothing but capture a system-audio
    monitor source (via `parec`/PulseAudio API) inside the sandbox and write a WAV.
  - **Why:** Flatpak's `xdg-desktop-portal` sandboxing may block monitor-source
    capture — the app's reason for existing. Latent v1.1 blocker: you could build the
    whole Flathub package and then find the sandbox blocks capture.
  - **Pros:** Learn the blocker cheaply before investing in Flathub packaging.
  - **Cons:** A few hours of portal/permission spelunking.
  - **Context:** Surfaced in plan-eng-review (outside voice). AppImage is v1 (no
    sandbox, capture works freely). Flathub is the newcomer-correct channel but its
    privacy sandbox conflicts with system-audio capture. Confirm capture works under
    portals before committing to Flathub.
  - **Depends on / blocked by:** AppImage v1 must work first. Do this only when
    pursuing Flathub distribution (v1.1).
