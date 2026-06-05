# TODOS — tutanak

## Deferred (v1.1+)

- [ ] **Flatpak portal vs monitor-capture investigation spike**
  - **What:** Build a trivial Flatpak that does nothing but capture a system-audio
    monitor source (via `parec`/PulseAudio API) inside the sandbox and write a WAV.
  - **Why:** Flatpak's `xdg-desktop-portal` sandboxing may block monitor-source
    capture — the app's reason for existing. Latent v1.1 blocker: you could build
    the whole Flathub package and then find the sandbox blocks capture.
  - **Pros:** Learn the blocker cheaply before investing in Flathub packaging.
  - **Cons:** A few hours of portal/permission spelunking.
  - **Context:** Surfaced in plan-eng-review (outside voice). AppImage is v1 (no
    sandbox, capture works freely). Flathub is the newcomer-correct channel but its
    privacy sandbox conflicts with system-audio capture. Confirm capture works
    under portals before committing to Flathub.
  - **Depends on / blocked by:** AppImage v1 must work first. Do this only when
    pursuing Flathub distribution (v1.1).
