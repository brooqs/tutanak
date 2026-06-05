# **tutanak**

Linux için gizlilik odaklı, terminal kullanımı gerektirmeyen bir toplantı notu asistanı.  
Sistem sesinizi (ve kendi mikrofonunuzu) kaydeder, yazıya döker (transkripsiyon), isteğe bağlı olarak çevirir, özetler ve sonucu bir markdown notu olarak kaydeder.  
Hem masaüstü arayüzü (tutanak-ui) hem de komut satırı arayüzü (tutanak) aynı çekirdeği (core) paylaşır. Yazıya dökme ve özetleme altyapıları bir **sağlayıcı havuzundan (provider registry)** seçilir — Groq (bulut), **FastFlowLM** (AMD Ryzen AI NPU) veya Ollama — böylece tüm iş akışını bilgisayarınızdan hiçbir şey dışarı çıkmadan **%100 yerel olarak NPU üzerinde** çalıştırabilirsiniz.

## **Neden?**

Çoğu toplantı notu aracı yalnızca bulut tabanlıdır, gizlilik konusunda şeffaf değildir ve Linux'u her zaman arka plana iter. tutanak ise tam tersi bir felsefeyle geliştirilmiştir: öncelik yerel çalışmadır (local-first), sesinizin nereye gittiği konusunda şeffaftır ve bir terminale dokunmadan kolayca çalıştırılabilir. Aynı motor her dilde çalışır; sunulan birinci sınıf deneyim, kayıtlarının kendi donanımlarında kalmasını isteyen kullanıcıları hedefler.

## **Mimari**

`ses yakalama (parecord, mikrofon + sistem miksi) ─► 16kHz mono WAV`  
        `│`  
        `▼`  
`STT motoru  ── Groq bulut │ FastFlowLM NPU │ whisper.cpp (planlanan)`  
        `│  parçalı, devam ettirilebilir, 429 hata yönetimi, 413 yeniden parçalama`  
        `▼`  
`metin dökümü ──► çeviri (isteğe bağlı, kaynak dil == hedef dil ise atlanır)`  
        `│`  
        `▼`  
`özet (map-reduce, hedef dil) ──► markdown notu  (~/.local/share/tutanak/)`

* core/ — iş akışı kütüphanesi (tutanak-core): ses yakalama, ses parçalama, sağlayıcı motorları, metin birleştirme, not depolama.  
* cli/ — tutanak komut satırı aracı.  
* gui/ — Slint ile geliştirilmiş tutanak-ui masaüstü arayüzü.

İş akışı somut bir yapıya sahiptir ve bu API'yi destekleyen her bulut veya yerel sunucu için OpenAI uyumlu tek bir HTTP motoru kullanır. Yeni bir altyapı eklemek kod yazmayı gerektirmez — yapılandırma dosyasına bir sağlayıcı profili eklemeniz yeterlidir.

## **Özellikler**

* **Sağlayıcı Havuzu (Provider Registry):** STT (sesi yazıya dökme) ve özetleme altyapılarını birbirinden bağımsız olarak seçebilirsiniz: Groq bulut, AMD NPU üzerinde FastFlowLM, Ollama veya OpenAI uyumlu herhangi bir sunucu. Tamamen yerel bir iş akışı (yakala → NPU ile yazıya dök → NPU ile özetle) yalnızca bir ayar değişikliği uzaklığındadır.  
* **Mikrofon \+ Sistem Sesi Yakalama:** Hem diğer katılımcıları (sistem sesi) hem de kendi mikrofonunuzu, bir PulseAudio sanal çıkışı (null-sink) üzerinden anlık olarak karıştırarak kaydeder — böylece kendi sesiniz de notlara dahil olur. PulseAudio ve PipeWire üzerinde çalışır.  
* **Uzun Toplantılarda Güvenilirlik:** Ses, sağlayıcının boyut sınırına göre parçalara ayrılır, istek sınırı (rate-limit) yönetimiyle yüklenir ve her parçanın metin dökümü diske önbelleğe alınır; böylece hata durumunda süreç baştan başlamak yerine kaldığı yerden devam eder.  
* **Map-Reduce Özetleri:** LLM (büyük dil modeli) bağlam penceresini (context window) aşan uzun metin dökümleri, pencereler halinde özetlenir ve ardından birleştirilir; böylece saatler süren toplantılardan bile tek bir özet üretilir.  
* **Otomatik Dil Yönetimi:** Kaynak dil otomatik olarak algılanır; kaynak dil çıktı dilinizle zaten eşleşiyorsa çeviri adımı atlanır.  
* **Geçmiş:** Geçmiş notlar arayüzde listelenir ve başlangıçta yeniden yüklenir, böylece uygulama yeniden başlatıldığında asla boş görünmez.  
* **Uygulama İçi Ayarlar:** Sağlayıcı URL'lerini, modelleri ve parça sınırlarını arayüzden düzenleyebilirsiniz; gizli anahtarlar (secrets) asla yapılandırma dosyasına açıkça yazılmaz.  
* **Arayüz (GUI) ve Komut Satırı (CLI):** Hangisi kolayınıza gelirse onu kullanın; her ikisi de aynı çekirdeği ve yapılandırmayı paylaşır.

## **Gereksinimler**

* Rust (1.95+) ve cargo  
* parecord (pulseaudio-utils / pipewire-pulse paketlerinden) — hem PulseAudio hem de PipeWire üzerinde çalışan sistem sesi yakalama aracı  
* ffmpeg — process komutu için (mevcut ses/video dosyalarını içe aktarırken kullanılır)  
* Bir altyapı sunucusu: Groq API anahtarı, çalışan bir FastFlowLM sunucusu ve/veya Ollama

## **Derleme**

`cargo build --release        # derlenen dosyalar: target/release/{tutanak, tutanak-ui}`  
`cargo test                   # birim + entegrasyon testleri (gerçek bir API gerekmez)`

## **Kullanım**

### **Masaüstü Arayüzü (GUI)**

`cargo run -p tutanak-ui      # veya doğrudan target/release/tutanak-ui dosyasını çalıştırın`

Yazıya dökme ve özetleme sağlayıcılarını seçin, dili ve başlığı belirleyin, kendi mikrofonunuzu dahil etmek isteyip istemediğinizi seçin ve ardından **Kaydet (Record)** butonuna basın. Toplantı bittiğinde **Durdur (Stop)** butonuna basın. Özet ve metin dökümü sekmelerde görünecek ve not \~/.local/share/tutanak/ dizinine kaydedilecektir. Eski notlara geçmiş açılır menüsünden ulaşabilirsiniz.

### **Komut Satırı (CLI)**

`export GROQ_API_KEY=...                      # Yalnızca Groq sağlayıcısı için gereklidir`

`# Canlı: Sistem sesini + mikrofonu kaydeder, ENTER ile durdurulur ve özet üretir`  
`tutanak record --title "Sprint Planlama"`

`# Sadece sistem sesi (mikrofonunuzu dahil etmez)`  
`tutanak record --system-only`

`# Mevcut bir ses/video dosyasını işleme (ffmpeg ile 16kHz mono formatına dönüştürülür)`  
`tutanak process toplanti.mp4 --title "Mimari Gözden Geçirme"`

`# Ek olarak metnin tam çevirisini de üretir (kaynak dil == hedef dil ise atlanır)`  
`tutanak record --translate`

`# Yapılandırma yardımcıları`  
`tutanak config init          # Açıklamalı varsayılan yapılandırma dosyasını oluşturur`  
`tutanak config show          # Geçerli yapılandırmayı gösterir (gizli anahtarlar gizlenir)`  
`tutanak config path          # Yapılandırma dosyasının yolunu yazdırır`

Notlar \~/.local/share/tutanak/\<zaman-damgasi\>-\<baslik\>.md şeklinde kaydedilir. Uzun bir işlem yarıda kalırsa, önbelleğe alınmış parçalardan devam etmek için başlangıçta yazdırılan \--job \<id\> parametresini kullanarak komutu yeniden çalıştırın.

## **Yapılandırma — Sağlayıcı Havuzu**

Ayarlar \~/.config/tutanak/config.toml dosyasından okunur. Öncelik sırası **varsayılanlar → yapılandırma dosyası → ortam değişkenleri** şeklindedir (ortam değişkenleri baskındır). Masaüstü arayüzü ayarları yapılandırma dosyasına yazar; ortam değişkenleri ise ileri düzey kullanıcılar ve CI (Sürekli Entegrasyon) sistemleri içindir.  
Sağlayıcılar, adlandırılmış profillerden oluşan bir havuzdur. stt.provider / summary.provider ayarları, bir profili adıyla seçer. Her profilin bir aktarım türü (kind) vardır:

* openai — OpenAI uyumlu herhangi bir HTTP sunucusu: **Groq** (bulut), **FastFlowLM** (AMD Ryzen AI NPU, :52625), **Ollama** (:11434), llama.cpp sunucusu vb.  
* whisper-cpp — Uygulama içi (in-process) CPU/GPU üzerinde çalışan whisper.cpp (planlanıyor).

Gizli anahtarlar dosyada saklanmaz; bir profil api\_key\_env aracılığıyla bir ortam değişkenini işaret eder (örneğin GROQ\_API\_KEY). Yerel NPU/CPU sunucuları bir anahtara ihtiyaç duymaz.

### **Örnek: AMD NPU üzerinde yazıya dökme, bulutta özetleme**

`[stt]`  
`provider = "fastflowlm"      # NPU üzerinde whisper-v3-turbo, gizli ve hızlı`

`[summary]`  
`provider = "groq"            # Bulut üzerinde özetleme`

FastFlowLM sunucusunun çalışıyor olması gerekir. Tek bir komut, hem ASR (ses tanıma) hem de LLM uç noktalarını tek bir port üzerinden dışa açar:  
`flm serve gemma4-it:e2b --asr 1`

### **Örnek: Tamamen yerel — bilgisayardan hiçbir veri çıkmaz**

`[stt]`  
`provider = "fastflowlm"      # NPU üzerinde yazıya dökme`

`[summary]`  
`provider = "fastflowlm"      # NPU üzerinde özetleme (aynı sunucu)`

### **Ortam Değişkenleri Geçersiz Kılmaları (Overrides)**

| Değişken | Açıklama   |
| :---- | :---- |
| GROQ\_API\_KEY | Groq profili için API anahtarı |
| TUTANAK\_STT\_PROVIDER / TUTANAK\_SUMMARY\_PROVIDER | Bir sağlayıcı profilini seçer |
| TUTANAK\_STT\_MODEL\` / \`TUTANAK\_LLM\_MODEL | Model seçimini geçersiz kılar |
| TUTANAK\_OUTPUT\_LANG | Çıktı dili (varsayılan tr) |
| TUTANAK\_CONFIG | Yapılandırma dosyası yolu (testler için kullanışlıdır) |
| TUTANAK\_CHUNK\_BYTES | Parça boyutu sınırı (Groq ücretsiz \= 25MB, geliştirici \= 100MB) |

## **Gizlilik**

Notlar yerel olarak \~/.local/share/tutanak/ dizininde saklanır. Herhangi bir telemetri (veri takibi) bulunmamaktadır. Bulut (Groq) modunda, sağlayıcıya yalnızca ses parçaları gönderilir. Tamamen yerel modda (FastFlowLM / Ollama) ise bilgisayarınızdan hiçbir şey dışarı çıkmaz. Masaüstü arayüzü, verilerinizin nereye gittiğini bilmeniz için hangi sağlayıcının aktif olduğunu her zaman gösterir.

## **Test Etme**

`cargo test                                   # Tüm testleri çalıştırır, gerçek API gerekmez`  
`GROQ_API_KEY=... TUTANAK_REAL_SMOKE=1 \`  
  `cargo test -- --ignored real_smoke         # Gerçek API kullanan duman (smoke) testini çalıştırır`

## **Durum ve Yol Haritası**

Mevcut çalışan özellikler: CLI ve GUI, sağlayıcı havuzu (Groq / FastFlowLM / Ollama), mikrofon \+ sistem sesi yakalama, devam ettirilebilir parçalı yazıya dökme, map-reduce özetleri, not geçmişi ve uygulama içi ayarlar.  
Planlanan özellikler: Sunucu gerektirmeyen, uygulama içi çalışan bir whisper.cpp motoru, AppImage/Flatpak dağıtımları, ayarlar arayüzünden sağlayıcı ekleme/çıkarma, canlı akış (streaming) halinde yazıya dökme ve aranabilir bir not arşivi.

## **Lisans**

[GNU GPL v3 veya üzeri](http://docs.google.com/LICENSE). Çatallamalar (fork) açık kaynak kalmak zorundadır.  
Masaüstü arayüzü, burada GPLv3 seçeneği altında kullanılan [Slint](https://slint.dev) ile oluşturulmuştur.