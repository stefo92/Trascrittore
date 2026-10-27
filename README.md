# Trascrittore

Trascrittore converte file audio in testo **sul tuo computer**, con [Whisper](https://github.com/ggerganov/whisper.cpp)
(implementazione whisper.cpp), ed esporta la trascrizione in **TXT, HTML, PDF e DOCX**.

È progettato per essere usato con gli screen reader (NVDA, JAWS, Assistente vocale) e solo da tastiera.
L'analisi completa del progetto è in [ANALISI.md](ANALISI.md).

## Caratteristiche

- Formati audio: MP3, WAV, FLAC, OGG/Vorbis, M4A/AAC, ALAC, AIFF, CAF, MKV/WebM (decodifica in puro Rust).
- Trascrizione locale: nessun dato inviato in rete. Solo i modelli vengono scaricati, su richiesta.
- Scelta del modello (da *tiny compresso*, 32 MB, a *large-v3*), della lingua o rilevamento automatico,
  traduzione in inglese.
- Calcolo sul processore (CPU) o sulla scheda video (GPU AMD, NVIDIA o Intel tramite Vulkan;
  NVIDIA anche tramite CUDA; Mac Apple Silicon tramite Metal), con scelta automatica.
- Encoder su **OpenVINO** (Intel): acceleratori neurali (NPU), schede video Intel o processore;
  runtime scaricabile dal programma, nessuna dipendenza per compilare.
- Testo modificabile prima dell'esportazione, con o senza marcature temporali.
- Documenti esportati accessibili:
  - **PDF taggato conforme a PDF/UA-1** (struttura, lingua, titolo, segnalibri);
  - **DOCX** con stili *Titolo 1/Titolo 2* navigabili e lingua impostata;
  - **HTML5** semantico con attributo `lang`;
  - **TXT** UTF-8.
- Interfaccia grafica Win32 nativa e interfaccia a riga di comando.

## Uso dell'interfaccia grafica

Avvia `trascrittore.exe` (puoi anche passare un file audio come argomento o usare «Apri con»).

1. **Ctrl+O** (o il pulsante *Sfoglia*) per scegliere il file audio.
2. Scegli **Modello** (Alt+M) e **Lingua** (Alt+L).
3. Prestazioni:
   - **Usa la scheda video (GPU)** (Alt+C) attiva o disattiva la GPU: è molto più veloce del processore;
   - **Scheda video** (Alt+H) sceglie quale scheda usare, se ce n'è più d'una;
   - **Modalità rapida** (Alt+P) usa una decodifica più semplice: sulla scheda video è fino a due volte
     più veloce, con qualche errore in più; sul processore il guadagno è minimo;
   - **Accelerazione Flash Attention** (menu Trascrizione) è disattivata per impostazione predefinita:
     su alcune schede video accelera, su altre e sul processore rallenta;
   - **OpenVINO (Intel)** (con Tab dopo «Scheda video») esegue l'encoder su un acceleratore neurale,
     una scheda video Intel o il processore: vedi [OpenVINO](#openvino-intel).

   Tempi misurati per 2 minuti di audio con large-v3-turbo-q5_0 su AMD Radeon RX 6800 XT:

   | Configurazione | Scheda video | Processore |
   |---|---|---|
   | Normale | 5 s | 1 min 27 s |
   | Modalità rapida | 3 s | 1 min 37 s |
   | Normale con Flash Attention | 7 s | 2 min 10 s |

   Lo stato iniziale (F2) dice se la trascrizione userà la scheda video o il processore. Se la scheda
   video richiesta non si attiva davvero (Trascrittore lo verifica dalla memoria video), viene mostrato
   un avviso.
4. **F5** avvia la trascrizione. Lo stato viene annunciato dallo screen reader; **F2** lo ripete; **Esc** annulla.
5. Al termine il cursore si sposta sul **testo trascritto**, che puoi leggere e correggere. Lo stato riporta
   anche il tempo impiegato e il dispositivo usato.
6. Scegli il **formato** (Alt+O) e premi **Ctrl+S** per esportare.

| Tasto | Azione |
|---|---|
| Ctrl+O | Apri file audio |
| F5 | Avvia la trascrizione |
| Esc | Annulla l'operazione in corso |
| Ctrl+S | Esporta |
| Ctrl+Maiusc+C | Copia tutto il testo |
| F2 | Leggi lo stato corrente |
| Ctrl+più / Ctrl+meno / Ctrl+0 | Dimensione del testo trascritto |
| F1 | Guida rapida |
| Alt+lettera sottolineata | Vai al campo corrispondente |

Se il modello scelto non è ancora sul computer, Trascrittore chiede se scaricarlo e mostra la dimensione.

## Uso da riga di comando

```bash
trascrittore-cli trascrivi intervista.mp3 --lingua it --formato docx,pdf
```

```bash
trascrittore-cli trascrivi lezione.m4a -m small-q5_1 -t -f txt,html -o trascrizioni --scarica-modello
```

```bash
trascrittore-cli trascrivi riunione.mp3 --dispositivo gpu:0
```

```bash
trascrittore-cli trascrivi lezione.mp3 -m small-q5_1 --openvino npu --scarica-modello
```

`--dispositivo` accetta `auto` (predefinito), `cpu`, `gpu` (prima scheda video) o `gpu:N`;
`--rapida` attiva la modalità rapida; `--flash-attention` attiva Flash Attention;
`--openvino npu|gpu|gpu.1|cpu` esegue l'encoder con OpenVINO (con `--scarica-modello` scarica anche
l'encoder se manca). Altri comandi: `trascrittore-cli openvino` (stato e dispositivi OpenVINO),
`trascrittore-cli installa-openvino`, `trascrittore-cli dispositivi` (processore e schede video utilizzabili),
`trascrittore-cli modelli` (elenco e stato dei modelli), `trascrittore-cli scarica <modello>`,
`trascrittore-cli lingue`, `trascrittore-cli --help`. L'avanzamento è stampato su righe separate,
così è leggibile anche con uno screen reader nel terminale.

## Modelli

| Modello | Dimensione | Note |
|---|---|---|
| **tiny-q5_1** | 32 MB | il più leggero e veloce, qualità bassa |
| tiny | 78 MB | velocissimo, qualità bassa |
| base-q5_1 | 60 MB | molto veloce, qualità discreta |
| base | 148 MB | veloce |
| small-q5_1 | 190 MB | veloce, buona qualità |
| small | 488 MB | buon equilibrio |
| medium-q5_0 | 539 MB | qualità alta |
| medium | 1,5 GB | qualità alta, lento |
| **large-v3-turbo-q5_0** | 574 MB | **predefinito**: qualità ottima, buona velocità |
| large-v3-turbo | 1,6 GB | qualità ottima |
| large-v3 | 3,1 GB | qualità massima, molto lento su CPU |

Le versioni «compresse» (quantizzate, suffisso `q5`) occupano circa un terzo dello spazio e sono
più veloci, con una perdita di qualità minima.

I modelli vengono salvati in `%LOCALAPPDATA%\Trascrittore\data\models` (cartella modificabile con la
variabile d'ambiente `TRASCRITTORE_MODELS_DIR`), oppure in `dati\modelli` nella versione portable.
Si può usare anche un file `.bin` personalizzato.

## OpenVINO (Intel)

OpenVINO esegue l'**encoder** di Whisper, la parte più pesante del calcolo, su un acceleratore neurale
Intel (NPU, per esempio «Intel AI Boost»), su una scheda video Intel o sul processore. Il decoder resta
su ggml, quindi sul processore o sulla scheda video scelta con «Usa la scheda video».

1. **Installa OpenVINO**: pulsante *Installa OpenVINO…*, subito dopo la casella *OpenVINO (Intel)*
   (visibile finché OpenVINO non è installato), oppure menu *Trascrizione > Installa OpenVINO (Intel)*,
   oppure `trascrittore-cli installa-openvino`. Il programma scarica l'archivio ufficiale di Intel
   (OpenVINO 2026.4.1, versione con runtime C++ statico, circa 237 MB), ne verifica l'impronta SHA-256
   e installa le librerie necessarie (circa 210 MB, con le licenze) in
   `%LOCALAPPDATA%\Trascrittore\data\openvino`, oppure in `dati\openvino` nella versione portable.
   In alternativa si può indicare un'installazione esistente con la variabile `TRASCRITTORE_OPENVINO_DIR`.
2. **Scegli il dispositivo** nella casella *OpenVINO (Intel)*. Le schede video AMD e NVIDIA non sono
   elencate: il plugin GPU di OpenVINO supporta solo le schede Intel. Se il modello scelto non ha un
   encoder disponibile (per esempio il predefinito large-v3-turbo), lo stato lo dice subito.
3. **Encoder del modello**: serve un encoder convertito per OpenVINO. Intel ne pubblica di già pronti
   per **base, small, medium e large-v3**, validi anche per le versioni compresse
   ([Intel/whisper.cpp-openvino-models](https://huggingface.co/Intel/whisper.cpp-openvino-models)):
   la prima volta il programma propone di scaricare il pacchetto (171 MB, 610 MB, 2,1 GB, 4,3 GB).
   Per tiny e large-v3-turbo non esistono encoder pronti: si possono convertire con lo script
   `models/convert-whisper-to-openvino.py` di [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
   e mettere il file `ggml-<modello>-encoder-openvino.xml` (con il `.bin`) accanto al modello.

Al termine lo stato dice se l'encoder è stato eseguito con OpenVINO; se OpenVINO non si attiva, il
programma usa l'encoder normale e lo segnala.

Misure su AMD Ryzen 7 3800X e Radeon RX 6800 XT, 2 minuti di audio con il modello base (testo identico
con e senza OpenVINO):

| Configurazione | Tempo |
|---|---|
| Processore | 12,4 s |
| Processore, encoder OpenVINO sul processore | 8,9 s |
| Scheda video (Vulkan) | 4,5 s |
| Scheda video, encoder OpenVINO sul processore | 4,5 s |

Non è stato possibile provare NPU e schede video Intel, che questo computer non ha.

Come funziona: whisper.cpp è compilato con `WHISPER_USE_OPENVINO` (in `.cargo/config.toml`) e chiama
tre funzioni (`whisper_openvino_init/encode/free`) che Trascrittore implementa in
`crates/core/src/openvino.rs` con l'API C di OpenVINO, caricata solo se presente. Non si usa l'enum
dei tipi di OpenVINO, che cambia tra le versioni: i dati vengono copiati nei tensori della richiesta
di inferenza dopo averne controllato la dimensione. Se l'encoder fallisce durante il lavoro (whisper.cpp
ne ignorerebbe l'esito) la trascrizione si ferma con un errore invece di produrre testo senza senso.

## Versione portable

`scripts\build.ps1 build --release` crea anche **`dist\Trascrittore-portable`** (e lo stesso contenuto in
`dist\Trascrittore-portable.zip`): una cartella da copiare su una chiavetta USB o in qualsiasi cartella e
avviare senza installare nulla. Contiene i due programmi, `LEGGIMI.txt` per l'utente (sorgente in
`portable\LEGGIMI.txt`), `portable.txt` e la cartella vuota `dati\modelli`.

- **Modalità portable**: se accanto all'eseguibile c'è la cartella `dati` o il file `portable.txt`,
  le impostazioni vanno in `dati\impostazioni.json` e i modelli in `dati\modelli`, invece che in
  `%APPDATA%\Trascrittore` e `%LOCALAPPDATA%\Trascrittore`. I file aperti ed esportati non vengono
  aggiunti agli elementi recenti di Windows. La variabile `TRASCRITTORE_MODELS_DIR`, se impostata,
  ha comunque la precedenza.
- **Nessun Visual C++ Redistributable**: Rust è compilato con `+crt-static` e whisper.cpp con `/MT`
  (vedi `.cargo/config.toml`), quindi gli eseguibili importano solo DLL di Windows.
- **Vulkan facoltativo**: `vulkan-1.dll` è a caricamento ritardato (`/DELAYLOAD`, vedi sotto).

Lo script non svuota `dist\Trascrittore-portable\dati` (può contenere modelli scaricati) e non la mette
nello zip.

## Versione portable per macOS (Apple Silicon)

Su Mac c'è solo l'interfaccia a riga di comando (la GUI è Win32), utilizzabile con VoiceOver nel
Terminale. Va compilata su un Mac Apple Silicon con Xcode Command Line Tools, CMake e Rust:

```bash
xcode-select --install
```

```bash
brew install cmake
```

```bash
rustup target add aarch64-apple-darwin
```

```bash
bash scripts/build-macos.sh build --release
```

Lo script compila per `aarch64-apple-darwin` con la GPU del Mac (**Metal**, feature
`trascrittore-core/metal`; `--solo-cpu` per escluderla) e crea **`dist/Trascrittore-portable-macos`**
e `dist/Trascrittore-portable-macos.zip` con `trascrittore-cli`, `LEGGIMI.txt` (sorgente in
`portable/LEGGIMI-macOS.txt`), `portable.txt` e la cartella vuota `dati/modelli`. Accetta gli stessi
comandi di cargo, ad esempio `bash scripts/build-macos.sh test --workspace`.

- **Nessuna dipendenza esterna**: Metal, Accelerate e la libreria C++ fanno parte di macOS; la libreria
  Metal di ggml è incorporata nell'eseguibile. Versione minima: macOS 13 (modificabile con
  `MACOSX_DEPLOYMENT_TARGET`).
- **Firma**: l'eseguibile riceve una firma «ad hoc» (`codesign --sign -`), obbligatoria su Apple
  Silicon. Non è autenticato da Apple (serve un account sviluppatore), quindi chi scarica lo zip deve
  togliere la quarantena con `xattr -dr com.apple.quarantine .`, come spiegato nel LEGGIMI.
- **Modalità portable**: identica a Windows (`dati/impostazioni.json`, `dati/modelli`), al posto di
  `~/Library/Application Support/Trascrittore`.
- **Flag di compilazione**: `.cargo/config.toml` imposta `CMAKE_C_FLAGS_RELEASE` e
  `CMAKE_CXX_FLAGS_RELEASE` con i flag di Visual Studio; lo script li sostituisce con quelli di clang
  (le variabili d'ambiente hanno la precedenza su `[env]`). Un `cargo build` diretto su macOS fallirebbe.

`bash scripts/verifica-macos.sh` controlla la versione portable su un Mac: eseguibile arm64 firmato e
collegato solo a librerie di sistema, trascrizione da una copia temporanea con la GPU e con il
processore (scarica il modello tiny-q5_1, 32 MB), dati nella cartella `dati` e nessun file nella
Libreria dell'utente. Lo stesso controllo è in `.github/workflows/macos.yml` per GitHub Actions
(runner `macos-14`, Apple Silicon).

## Compilazione

Prerequisiti su Windows:

- [Rust](https://rustup.rs) stabile (toolchain MSVC);
- Visual Studio Build Tools 2022 con il carico di lavoro «Sviluppo di applicazioni desktop C++»;
- CMake nel `PATH` (va bene quello incluso nei Build Tools:
  `...\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin`);
- LLVM (serve `libclang` per generare i binding di whisper.cpp):

```bash
winget install LLVM.LLVM
```

- Vulkan SDK, per il supporto alle schede video AMD, NVIDIA e Intel:

```bash
winget install KhronosGroup.VulkanSDK
```

libclang ha bisogno degli header di MSVC, quindi la compilazione va eseguita nell'ambiente di
Visual Studio. Lo script `scripts\build.ps1` lo prepara (vcvars64, CMake, LLVM, Vulkan) e poi esegue
cargo con gli argomenti indicati:

```bash
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 build --release
```

**I programmi pronti vengono copiati in `dist\`** (`dist\trascrittore.exe` e `dist\trascrittore-cli.exe`):
avvia sempre quelli. Lo script dice anche se la build include il supporto per le schede video.

### Supporto per le schede video

- Se il Vulkan SDK è installato, lo script include **automaticamente** il supporto Vulkan (schede AMD,
  NVIDIA e Intel con un'unica build); per l'esecuzione basta il driver della scheda. Con `--solo-cpu`
  si compila la versione che usa solo il processore.
- **CUDA** (solo NVIDIA, di solito più veloce di Vulkan sulle stesse schede) richiede il CUDA Toolkit per
  compilare e le sue librerie per l'esecuzione: aggiungi `--features trascrittore-core/cuda`.

Con il supporto GPU lo script compila in `C:\tr-build`, perché i sotto-progetti CMake di ggml superano il
limite di 260 caratteri dei percorsi di Windows se la cartella di build è profonda (si può scegliere
un'altra cartella corta con `CARGO_TARGET_DIR`). Un semplice `cargo build` produce invece la versione
solo processore: la GUI lo segnala nello stato iniziale e quando si attiva «Usa la scheda video».

La build con Vulkan usa `vulkan-1.dll` (il Vulkan Loader di Khronos), installata in `System32` da tutti
i driver video recenti. La DLL **non viene distribuita** con il programma ed è collegata a
**caricamento ritardato** (`/DELAYLOAD:vulkan-1.dll`, negli script `build.rs` di GUI e CLI), quindi i
programmi partono anche dove manca:

- ggml ricava quasi tutte le funzioni Vulkan da `vkGetInstanceProcAddr`; ne importa direttamente solo
  altre due, usate dopo aver creato l'istanza Vulkan;
- prima che ggml usi Vulkan, l'hook del caricamento ritardato (`crates/core/src/vulkan.rs`) carica la
  DLL **solo dalla cartella di sistema**, mai da quella del programma (su una chiavetta USB chiunque
  potrebbe affiancarle una DLL con lo stesso nome);
- se la DLL manca, l'hook fornisce un `vkGetInstanceProcAddr` sostitutivo le cui funzioni falliscono con
  `VK_ERROR_INITIALIZATION_FAILED`: ggml considera Vulkan non disponibile e Trascrittore usa il processore.

Scelta e licenza: il Vulkan SDK 1.4 non contiene una `vulkan-1.dll` ridistribuibile come file singolo
(solo l'installatore `VulkanRT.exe`), e dove non ci sono driver Vulkan una copia del loader non
servirebbe comunque, perché non troverebbe nessuna scheda video da usare. Il loader ha licenza
Apache 2.0, ma non essendo distribuito non servono avvisi di licenza.

La variabile d'ambiente `TRASCRITTORE_NO_VULKAN=1` disattiva Vulkan del tutto (utile con driver
difettosi). Se non viene trovata nessuna scheda video, Trascrittore usa il processore.

## Test

```bash
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 test --workspace
```

Senza whisper.cpp (utile dove mancano CMake o LLVM):

```bash
cargo test -p trascrittore-core --no-default-features
```

Verifica automatica della GUI così come la vedono gli screen reader (ruoli, nomi, tasti di scelta
rapida e stati MSAA di ogni controllo); con `-AudioFile` esegue anche una trascrizione dalla GUI e
controlla che al termine il focus passi al testo:

```bash
powershell -ExecutionPolicy Bypass -File scripts\verifica-accessibilita.ps1 -AudioFile prova.wav
```

`scripts\crea-audio-prova.ps1` genera un file WAV di prova con la sintesi vocale di Windows.

## Limiti noti

- L'annullamento non è istantaneo: whisper.cpp si ferma al termine del blocco di audio
  (fino a 30 secondi di parlato) che sta elaborando, quindi possono servire alcuni secondi.
- In whisper-rs 0.16.0 `FullParams::set_abort_callback_safe` passa a whisper.cpp un puntatore
  interpretato in modo errato (la trascrizione fallisce con «failed to encode»): Trascrittore usa
  al suo posto l'API di basso livello con un proprio trampolino.
- Con MSVC il crate `cmake` toglie i flag di ottimizzazione dalle build release di whisper.cpp
  (che risulterebbe circa 12 volte più lento sulla CPU): `.cargo/config.toml` imposta
  esplicitamente `CMAKE_C_FLAGS_RELEASE` e `CMAKE_CXX_FLAGS_RELEASE`, con `/MT` (runtime statico) in
  accordo con `+crt-static` di Rust. Non rimuoverlo. Dopo una modifica di questi flag serve
  `cargo clean --release -p whisper-rs-sys` (con `CARGO_TARGET_DIR=C:\tr-build` per la build con GPU).
- La GUI è solo per Windows; nucleo e CLI funzionano anche su Linux e macOS. Su macOS si compila con
  `scripts/build-macos.sh`; su Linux bisogna impostare `CMAKE_C_FLAGS_RELEASE` e
  `CMAKE_CXX_FLAGS_RELEASE` (ad esempio `-O3 -DNDEBUG`), perché quelli di `.cargo/config.toml` sono per
  Visual Studio.

## Struttura

```
crates/core   libreria: audio, modelli, dispositivi, trascrizione, documento, esportatori
crates/cli    trascrittore-cli
crates/gui    trascrittore (Win32 nativo, accessibile)
portable/     LEGGIMI.txt, LEGGIMI-macOS.txt e portable.txt delle versioni portable
scripts/      compilazione (Windows e macOS), verifiche e audio di prova
```

## Carattere del PDF

Il PDF incorpora un carattere di sistema (Arial o Segoe UI su Windows, DejaVu/Liberation/Noto su Linux,
Arial su macOS). Per usarne un altro: variabili d'ambiente `TRASCRITTORE_PDF_FONT` e
`TRASCRITTORE_PDF_FONT_BOLD` con il percorso dei file `.ttf`.
