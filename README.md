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
  NVIDIA anche tramite CUDA), con scelta automatica.
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
     su alcune schede video accelera, su altre e sul processore rallenta.

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

`--dispositivo` accetta `auto` (predefinito), `cpu`, `gpu` (prima scheda video) o `gpu:N`;
`--rapida` attiva la modalità rapida; `--flash-attention` attiva Flash Attention.
Altri comandi: `trascrittore-cli dispositivi` (processore e schede video utilizzabili),
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
variabile d'ambiente `TRASCRITTORE_MODELS_DIR`). Si può usare anche un file `.bin` personalizzato.

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

La build con Vulkan richiede `vulkan-1.dll`, installata da tutti i driver video recenti; su computer
che non ce l'hanno va distribuita insieme al programma. Se non viene trovata nessuna scheda video,
Trascrittore usa il processore.

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
  esplicitamente `CMAKE_C_FLAGS_RELEASE` e `CMAKE_CXX_FLAGS_RELEASE`. Non rimuoverlo.
- La GUI è solo per Windows; nucleo e CLI funzionano anche su Linux e macOS.

## Struttura

```
crates/core   libreria: audio, modelli, dispositivi, trascrizione, documento, esportatori
crates/cli    trascrittore-cli
crates/gui    trascrittore (Win32 nativo, accessibile)
scripts/      verifica dell'accessibilità e audio di prova
```

## Carattere del PDF

Il PDF incorpora un carattere di sistema (Arial o Segoe UI su Windows, DejaVu/Liberation/Noto su Linux,
Arial su macOS). Per usarne un altro: variabili d'ambiente `TRASCRITTORE_PDF_FONT` e
`TRASCRITTORE_PDF_FONT_BOLD` con il percorso dei file `.ttf`.
