# Trascrittore

Trascrittore converte file audio in testo **sul tuo computer**, con [Whisper](https://github.com/ggerganov/whisper.cpp)
(implementazione whisper.cpp), ed esporta la trascrizione in **TXT, HTML, PDF e DOCX**.

È progettato per essere usato con gli screen reader (NVDA, JAWS, Assistente vocale) e solo da tastiera.
L'analisi completa del progetto è in [ANALISI.md](ANALISI.md).

## Caratteristiche

- Formati audio: MP3, WAV, FLAC, OGG/Vorbis, M4A/AAC, ALAC, AIFF, CAF, MKV/WebM (decodifica in puro Rust).
- Trascrizione locale: nessun dato inviato in rete. Solo i modelli vengono scaricati, su richiesta.
- Scelta del modello (da *tiny* a *large-v3*), della lingua o rilevamento automatico, traduzione in inglese.
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
3. **F5** avvia la trascrizione. Lo stato viene annunciato dallo screen reader; **F2** lo ripete; **Esc** annulla.
4. Al termine il cursore si sposta sul **testo trascritto**, che puoi leggere e correggere.
5. Scegli il **formato** (Alt+O) e premi **Ctrl+S** per esportare.

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
trascrittore-cli trascrivi lezione.m4a -m small -t -f txt,html -o trascrizioni --scarica-modello
```

Altri comandi: `trascrittore-cli modelli` (elenco e stato dei modelli), `trascrittore-cli scarica <modello>`,
`trascrittore-cli lingue`, `trascrittore-cli --help`. L'avanzamento è stampato su righe separate,
così è leggibile anche con uno screen reader nel terminale.

## Modelli

| Modello | Dimensione | Note |
|---|---|---|
| tiny | 78 MB | velocissimo, qualità bassa |
| base | 148 MB | veloce |
| small | 488 MB | buon equilibrio |
| medium | 1,5 GB | qualità alta, lento |
| **large-v3-turbo-q5_0** | 574 MB | **predefinito**: qualità ottima, buona velocità |
| large-v3-turbo | 1,6 GB | qualità ottima |
| large-v3 | 3,1 GB | qualità massima, molto lento su CPU |

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

libclang ha bisogno degli header di MSVC, quindi la compilazione va eseguita nell'ambiente di
Visual Studio. Lo script `scripts\build.ps1` lo prepara (vcvars64, CMake, LLVM) e poi esegue cargo
con gli argomenti indicati:

```bash
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 build --release
```

In alternativa si può usare `cargo build --release` da «Developer PowerShell for VS 2022».

I binari sono `target\release\trascrittore.exe` (GUI) e `target\release\trascrittore-cli.exe`.

Accelerazione GPU opzionale: `cargo build --release --features trascrittore-core/cuda`
(richiede CUDA Toolkit) oppure `--features trascrittore-core/vulkan` (richiede Vulkan SDK).

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
- La GUI è solo per Windows; nucleo e CLI funzionano anche su Linux e macOS.

## Struttura

```
crates/core   libreria: audio, modelli, trascrizione, documento, esportatori
crates/cli    trascrittore-cli
crates/gui    trascrittore (Win32 nativo, accessibile)
scripts/      verifica dell'accessibilità e audio di prova
```

## Carattere del PDF

Il PDF incorpora un carattere di sistema (Arial o Segoe UI su Windows, DejaVu/Liberation/Noto su Linux,
Arial su macOS). Per usarne un altro: variabili d'ambiente `TRASCRITTORE_PDF_FONT` e
`TRASCRITTORE_PDF_FONT_BOLD` con il percorso dei file `.ttf`.
