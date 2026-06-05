# tutanak

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

Linux için gizliliği net, terminal-suz hedefli bir toplantı-notu asistanı.
Sistem sesini yakalar → transkript çıkarır → (gerekiyorsa) çevirir → özetler → markdown not yazar.

CLI (`tutanak`) ve masaüstü GUI (`tutanak-ui`, slint) aynı çekirdeği paylaşır. Transkript ve
özet için **provider registry**: Groq (bulut), **FastFlowLM** (AMD Ryzen AI NPU), Ollama —
config'den seçilir; tamamen yerel (NPU) çalıştırılabilir.

## Mimari (v0)

```
capture (parecord) ─► 16kHz mono WAV ─► Groq transcribe (chunked, resumable) ─► transcript
                                                                                   │
                        özet (çıktı dili, map-reduce) ◄────────────────────────────┤
                        çeviri (opsiyonel, kaynak≠hedef ise) ◄──────────────────────┘
                                          │
                                       markdown not  (~/.local/share/tutanak/)
```

- `core/` — pipeline kütüphanesi (capture, audio chunking, Groq engine, stitch, storage).
- `cli/` — `tutanak` ikili (clap).

v0 bilinçli olarak somut (trait yok) ve her yer `anyhow`. Soyutlamalar (engine/capturer trait,
`thiserror`) v1'de ikinci motor + GUI gelince çıkarılacak.

## Gereksinimler

- Rust (1.95+), `cargo`
- `parecord` (pulseaudio-utils / pipewire-pulse) — sistem-ses yakalama (PulseAudio + PipeWire)
- `ffmpeg` — `process` (dosya içe aktarma) yolu için
- Bir Groq API anahtarı

## Kurulum

```bash
cargo build --release   # ikili: target/release/tutanak
cargo test              # 22 unit + 4 entegrasyon (httpmock)
```

## Kullanım

```bash
export GROQ_API_KEY=gsk_...

# Canlı: sistem sesini kaydet (ENTER ile durdur), özet çıkar
tutanak record --title "Sprint Planlama"

# Var olan dosyayı işle (ffmpeg ile 16kHz mono'ya çevrilir)
tutanak process toplanti.mp4 --title "Mimari Görüşmesi"

# Tam transkript çevirisi de iste (kaynak=hedef ise otomatik atlanır)
tutanak record --translate
```

Notlar `~/.local/share/tutanak/<zaman>-<başlık>.md` altına yazılır.
Uzun bir kayıt yarıda hata alırsa, **aynı `--title`/`--job` ile** tekrar çalıştır:
tamamlanan chunk'lar `~/.cache/tutanak/<job>/` içinden resume edilir.

## Yapılandırma — provider registry

Ayarlar `~/.config/tutanak/config.toml` dosyasından okunur (UI bunu yazacak).
Katmanlama: **varsayılan → config dosyası → ortam değişkenleri** (env üstün).

```bash
tutanak config init     # yorumlu varsayılan config'i oluştur
tutanak config show     # etkin ayarları + provider'ları göster (sır göstermez)
tutanak config path     # dosya yolu
```

**Provider'lar** bir registry'dir. `stt.provider` / `summary.provider` bir profili seçer.
Her profilin bir `kind`'i vardır:
- `openai` — OpenAI-uyumlu herhangi bir HTTP sunucu: **Groq** (bulut), **FastFlowLM**
  (AMD Ryzen AI NPU, `:52625`), **Ollama** (`:11434`), llama.cpp server, LocalAI...
- `whisper-cpp` — in-process whisper.cpp (CPU/GPU) — **v1'de gelecek**.

Yeni bir OpenAI-uyumlu backend eklemek **sıfır kod**: config'e bir `[providers.x]` ekle.

### Örnek: transkripti AMD NPU'da (FastFlowLM), özeti bulutta (Groq)
```toml
[stt]
provider = "fastflowlm"      # NPU'da whisper-v3-turbo, gizli + hızlı
[summary]
provider = "groq"            # özet bulutta
```
(FastFlowLM sunucusu çalışıyor olmalı: `flm serve`.)

### Örnek: tamamen yerel (NPU + Ollama) — hiçbir şey buluta gitmez
```toml
[stt]
provider = "fastflowlm"
[summary]
provider = "ollama"
model = "llama3.1"
```

Sırlar config'de değildir: her provider `api_key_env` ile bir env değişkeni adı verir
(Groq için `GROQ_API_KEY`). Yerel NPU/CPU sunucuları anahtar istemez.

### Env override'ları (güç-kullanıcı / CI)
| Değişken | Açıklama |
|---|---|
| `GROQ_API_KEY` | Groq profilinin anahtarı |
| `TUTANAK_STT_PROVIDER` / `TUTANAK_SUMMARY_PROVIDER` | provider profilini seç |
| `TUTANAK_STT_MODEL` / `TUTANAK_LLM_MODEL` | model override |
| `TUTANAK_OUTPUT_LANG` | çıktı dili (varsayılan `tr`) |
| `TUTANAK_CONFIG` | config dosyası yolu (test için) |
| `TUTANAK_CHUNK_BYTES` | chunk eşiği (Groq free=25MB, dev=100MB) |

## Gizlilik

Yerel saklama `~/.local/share/tutanak/`. Telemetri yok. Bulut (Groq) modunda yalnızca
ses chunk'ları Groq'a gider. (Yerel motor v1'de gelecek; o modda hiçbir şey cihazdan çıkmaz.)

## Test

```bash
cargo test                                   # tümü (gerçek API gerektirmez)
GROQ_API_KEY=... TUTANAK_REAL_SMOKE=1 \
  cargo test -- --ignored real_smoke         # opt-in gerçek API smoke
```

## Lisans

[GNU GPL v3 (veya üzeri)](LICENSE). Fork'lar açık kalmak zorundadır.

GUI [slint](https://slint.dev) ile yazıldı; slint bu projede GPLv3 seçeneği altında kullanılır.
