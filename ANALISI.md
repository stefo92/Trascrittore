# Trascrittore — Analisi del software

Versione del documento: 0.2 — 28 ottobre 2026 (interfaccia grafica per macOS)

## 1. Obiettivo

Trascrittore è un'applicazione desktop scritta in Rust che:

1. riceve in input un file audio (MP3, WAV, FLAC, OGG/Vorbis, M4A/AAC, ALAC, MKV/WebM audio…);
2. lo trascrive **in locale** con Whisper (implementazione whisper.cpp), senza inviare dati in rete;
3. permette di rivedere e correggere il testo;
4. esporta la trascrizione in **TXT, HTML, PDF e DOCX**;
5. è pienamente utilizzabile da persone cieche con screen reader (NVDA, JAWS, Assistente vocale/Narrator su Windows, VoiceOver su macOS) e solo da tastiera.

## 2. Utenti e scenari d'uso

| Utente | Scenario tipico |
|---|---|
| Persona cieca che usa NVDA o JAWS | Apre una registrazione (lezione, riunione, intervista), avvia la trascrizione, ascolta lo stato di avanzamento, legge il testo con le frecce, lo corregge e lo esporta in DOCX per condividerlo. |
| Persona cieca che usa VoiceOver su Mac | Stesso flusso con i tasti rapidi del Mac (Comando+O, Comando+R, Comando+S); gli annunci di VoiceOver riportano fasi e avanzamento senza spostare il cursore. |
| Persona ipovedente | Usa ingrandimento di sistema e contrasto elevato: l'interfaccia deve rispettare DPI, dimensione caratteri e temi ad alto contrasto di Windows. |
| Utente vedente | Stesso flusso con mouse o tastiera. |
| Utente avanzato / automazioni | Usa la riga di comando per trascrivere molti file in batch. |

## 3. Requisiti funzionali

| ID | Requisito |
|---|---|
| RF-01 | Selezionare un file audio tramite finestra di dialogo di sistema o argomento da riga di comando. |
| RF-02 | Decodificare i formati audio comuni e convertirli in PCM mono 16 kHz (formato richiesto da Whisper). |
| RF-03 | Trascrivere con Whisper in locale, scegliendo il modello (tiny, base, small, medium, large-v3-turbo, large-v3 e le loro varianti compresse, la più leggera di 32 MB) o un file modello personalizzato. |
| RF-03b | Scegliere il dispositivo di calcolo: automatico, processore (CPU) o una specifica scheda video (AMD, NVIDIA, Intel). |
| RF-03c | Eseguire l'encoder con OpenVINO su acceleratori neurali (NPU) e schede video Intel o sul processore, con installazione del runtime e download degli encoder dal programma. |
| RF-03d | Sui Mac Apple Silicon eseguire l'encoder con Core ML sul Neural Engine, con download degli encoder dal programma e possibilità di disattivarlo. |
| RF-04 | Scegliere la lingua del parlato o lasciarla rilevare automaticamente. |
| RF-05 | Opzione "Traduci in inglese" (funzione nativa di Whisper). |
| RF-06 | Mostrare l'avanzamento (percentuale e fase: decodifica, caricamento modello, trascrizione). |
| RF-07 | Annullare una trascrizione in corso. |
| RF-08 | Visualizzare il testo in un campo modificabile, con o senza marcature temporali. |
| RF-09 | Esportare il testo (incluse le correzioni manuali) in TXT, HTML, PDF, DOCX. |
| RF-10 | Scaricare su richiesta i modelli Whisper ufficiali, mostrando dimensione e avanzamento. |
| RF-11 | Ricordare le preferenze (modello, lingua, opzioni, ultima cartella). |
| RF-12 | Copiare l'intera trascrizione negli appunti. |
| RF-13 | Interfaccia a riga di comando con le stesse funzioni (incluso export multiplo). |

## 4. Requisiti non funzionali

### 4.1 Accessibilità (requisito primario)

- **Controlli nativi del sistema operativo**: ogni elemento è un vero controllo Win32 (pulsante, casella combinata, casella di modifica, barra di avanzamento), quindi espone automaticamente nome, ruolo, stato e valore tramite MSAA/UI Automation. Nessun controllo "disegnato a mano".
- **Etichette associate**: ogni campo è preceduto, nell'ordine di tabulazione, da un'etichetta statica; Windows la usa come nome accessibile del campo.
- **Uso completo da tastiera**: ordine di tabulazione logico (dall'alto in basso), tasti di scelta rapida (Alt+lettera) per ogni campo, acceleratori globali (Ctrl+O, F5, Ctrl+S…), menu standard raggiungibile con Alt. Il campo di testo multiriga non "intrappola" il Tab.
- **Annunci automatici**: i cambi di stato (inizio, fasi, completamento, errori) vengono notificati allo screen reader tramite *UI Automation Notification Events*, senza spostare il focus. Al termine il focus passa al testo trascritto; gli errori usano finestre di messaggio standard, sempre lette.
- **Stato su richiesta**: F2 ripete lo stato corrente (fase e percentuale).
- **macOS (VoiceOver)**: stessi principi con i controlli nativi di AppKit (`NSButton`, `NSPopUpButton`, `NSTextField`, `NSTextView`, `NSProgressIndicator`). Il nome accessibile è impostato sul controllo e sulla sua cella (l'elemento che VoiceOver legge per campi e menu a comparsa) e le etichette visibili sono escluse dall'albero, per non essere lette due volte; il percorso del file è esposto come campo di testo (nome e valore). Gli annunci usano `NSAccessibilityAnnouncementRequestedNotification` con priorità alta; Comando+L (o F2) ripete lo stato. Tasti rapidi secondo le convenzioni del Mac, voci nascoste con F1, F2, F5 come su Windows, menu *Vai* (Comando+1…5) per raggiungere i campi anche senza la navigazione da tastiera completa; nel testo Tab passa al controllo successivo, Esc interrompe il lavoro.
- **Ipovisione**: consapevolezza DPI per monitor (Per-Monitor v2), carattere di sistema, rispetto dei temi a contrasto elevato (i controlli nativi li ereditano), testo ingrandibile nel campo trascrizione (Ctrl++ / Ctrl+-).
- **Documenti prodotti accessibili**:
  - HTML5 semantico con `lang`, titolo, intestazioni, landmark `<main>`;
  - PDF **taggato** conforme a PDF/UA-1 (struttura, lingua, titolo, testo selezionabile);
  - DOCX con stili *Titolo/Titolo 1* reali (navigabili per intestazioni in Word) e lingua impostata;
  - TXT in UTF-8.

### 4.2 Privacy e sicurezza

- Elaborazione al 100% locale. L'unico accesso alla rete è il download dei modelli, solo su richiesta esplicita dell'utente, da Hugging Face (repository ufficiale whisper.cpp) via HTTPS.
- Download in file temporaneo `.part`, rinominato solo a completamento e con verifica della dimensione.

### 4.3 Prestazioni

- whisper.cpp compilato con ottimizzazioni anche in debug; uso di tutti i core disponibili.
- Accelerazione GPU: feature `vulkan` (AMD, NVIDIA e Intel con un'unica build) e `cuda` (solo NVIDIA). Le schede vengono elencate dal registro dei dispositivi di ggml, con lo stesso ordine che usa whisper.cpp; «Automatico» preferisce una scheda dedicata, poi una integrata, poi il processore.
- Modelli compressi (quantizzati q5): circa un terzo della dimensione, caricamento e calcolo più rapidi.
- Opzioni attivabili dall'utente: scheda video sì/no e quale scheda, modalità rapida (decodifica greedy invece della beam search a 5), Flash Attention. Valori predefiniti scelti con misure reali (AMD Radeon RX 6800 XT, 2 minuti di audio): GPU 5 s, modalità rapida 3 s, Flash Attention 7 s (più lenta, quindi disattivata); CPU 1 min 27 s.
- Verifica dell'uso effettivo della GPU: dopo il caricamento del modello si controlla dalla memoria video libera che i pesi siano sulla scheda; altrimenti l'utente viene avvisato e lo stato riporta il processore.
- Lo script di build include il supporto Vulkan quando il Vulkan SDK è presente e copia i programmi in `dist\`, così non si avvia per errore una versione solo CPU.
- Mac Apple Silicon: encoder su Core ML (Neural Engine), attivo per impostazione predefinita. Misure su Apple M5, 3 minuti di audio con large-v3-turbo-q5_0: scheda video 14,3 s → 10,3 s, modalità rapida 11,3 s → 6,8 s, solo processore 30,4 s → 12,3 s; con base-q5_1 la scheda video resta leggermente più veloce (4,2 s contro 4,7 s). La prima esecuzione compila il modello per il Neural Engine (circa 15 s con large-v3-turbo), durante il caricamento.
- Il modello caricato viene riutilizzato tra trascrizioni successive.
- Tutte le operazioni lunghe girano su un thread di lavoro: l'interfaccia resta reattiva e lo screen reader non si blocca.

### 4.4 Portabilità

- Nucleo (`trascrittore-core`) e CLI: Windows, Linux, macOS.
- GUI: Windows (Win32) e macOS (AppKit, pacchetto `Trascrittore.app`), vedi §6.1. Entrambe riusano tutto il nucleo; una GUI per Linux si potrà aggiungere allo stesso modo.

## 5. Architettura

Workspace Cargo con tre crate:

```
trascrittore/
├── crates/core   → libreria: audio, modelli, trascrizione, documento, esportazione
├── crates/cli    → binario `trascrittore-cli` (clap)
└── crates/gui    → binario `trascrittore` (Win32 nativo su Windows, AppKit nativo su macOS)
```

```
 File audio ──► audio::decode ──► PCM f32 mono 16 kHz
                (symphonia + rubato)        │
                                            ▼
 Modello .bin ──► Transcriber (whisper-rs) ──► Transcript { segmenti, lingua }
                                            │
                                            ▼
                         Testo (con/senza timestamp) ──► campo modificabile (GUI)
                                            │
                                            ▼
                         Document { titolo, metadati, paragrafi, lingua }
                                            │
                     ┌──────────┬───────────┼───────────┐
                     ▼          ▼           ▼           ▼
                    TXT       HTML      PDF/UA        DOCX
```

### 5.1 Moduli del nucleo

| Modulo | Responsabilità |
|---|---|
| `audio` | Apertura del contenitore, decodifica di tutte le tracce in PCM, mixaggio in mono, ricampionamento a 16 kHz. |
| `models` | Catalogo dei modelli ufficiali (nome, file, dimensione), cartella modelli, download con avanzamento e annullamento. |
| `device` | Elenco delle schede video utilizzabili, scelta «Automatico / CPU / GPU n» e sua risoluzione. |
| `coreml` | Encoder Core ML (Neural Engine): catalogo degli encoder ufficiali con impronta SHA-256, percorso cercato da whisper.cpp, download ed estrazione. |
| `openvino` | Caricamento dinamico dell'API C di OpenVINO, dispositivi Intel, encoder chiamato da whisper.cpp, installazione del runtime e degli encoder. |
| `transcriber` | Caricamento del contesto Whisper, parametri (lingua, traduzione, thread), callback di avanzamento e annullamento, raccolta dei segmenti. |
| `transcript` | Strutture `Segment`/`Transcript`, formattazione dei timestamp, conversione in testo. |
| `document` | Modello neutro del documento da esportare. |
| `export::{txt,html,pdf,docx}` | Un esportatore per formato, tutti dietro `export(doc, formato, percorso)`. |
| `settings` | Preferenze utente in JSON nella cartella di configurazione dell'utente. |
| `languages` | Elenco delle lingue supportate con nome in italiano e codice ISO. |

## 6. Scelte tecnologiche

### 6.1 Interfaccia grafica: Win32 nativo (crate `windows`)

È la decisione più importante, perché determina la qualità dell'accessibilità.

| Opzione | Accessibilità con screen reader | Note |
|---|---|---|
| **Win32 nativo via `windows-rs`** ✅ | Eccellente: controlli standard supportati da NVDA, JAWS e Narrator da decenni, nessuno strato intermedio. | 100% Rust, binario piccolo, nessun runtime. Solo Windows. |
| egui / Slint + AccessKit | Buona e in crescita, ma i controlli sono disegnati dal toolkit: caselle combinate, menu e testo multiriga lungo restano meno robusti. | Multipiattaforma, 100% Rust. |
| Tauri (WebView) | Molto buona (ARIA), ma l'esperienza è quella di una pagina web (modalità navigazione/focus). | UI in HTML/JS, non in Rust. |
| GTK 4 | Buona su Linux, scarsa su Windows. | — |

Motivazione: la grande maggioranza degli utenti ciechi su desktop usa Windows con NVDA o JAWS, e con i controlli nativi l'interfaccia "parla" in modo prevedibile e familiare. L'interfaccia è isolata nel crate `gui`, che riusa l'intero nucleo.

### 6.1b Interfaccia grafica per macOS: AppKit nativo (crate `objc2`)

Stesso criterio: su Mac l'accessibilità migliore è quella dei controlli di AppKit, che VoiceOver supporta da sempre. La GUI è scritta in Rust con `objc2`, `objc2-foundation` e `objc2-app-kit` (binding generati dagli header di Apple), nel modulo `crates/gui/src/mac`, senza Swift né progetti Xcode: si compila con lo stesso `cargo` del resto del programma. Alternative scartate: SwiftUI/AppKit in Swift con il nucleo come libreria C (due linguaggi e un livello FFI), egui/Slint con AccessKit (controlli disegnati dal toolkit), una finestra web (esperienza da pagina web).

### 6.2 Altre librerie

| Ambito | Crate | Motivo |
|---|---|---|
| Motore di trascrizione | `whisper-rs` 0.16 (whisper.cpp) | Whisper locale, veloce su CPU, GPU opzionale, modelli GGML; su Mac feature `metal` e `coreml` (Neural Engine, con ripiego sull'encoder normale). |
| Decodifica audio | `symphonia` 0.6 | Puro Rust, nessuna dipendenza da FFmpeg, formati comuni. |
| Ricampionamento | `rubato` 5 | Ricampionamento di qualità (sinc/FFT) a 16 kHz. |
| PDF | `krilla` 0.8 | Unico crate Rust maturo con **PDF taggati e validazione PDF/UA-1**; incorpora i caratteri. |
| Misura testo PDF | `ttf-parser` | Larghezze dei glifi per l'andata a capo. |
| DOCX | `zip` + XML Office Open scritto a mano | Controllo totale su stili, intestazioni, lingua e metadati. |
| HTML / TXT | libreria standard | Nessuna dipendenza necessaria. |
| Download modelli | `ureq` 3 (rustls) | Client HTTP semplice e sincrono. |
| OpenVINO | API C caricata con `libloading`, `sha2` per verificare il download | Nessuna dipendenza in compilazione; runtime C++ separati (Trascrittore usa /MT); compatibile con versioni diverse di OpenVINO perché non usa gli enum dei tipi. |
| Percorsi utente | `directories` | Cartelle dati/configurazione standard per sistema. |
| CLI | `clap` 4 | Parsing argomenti con aiuto generato. |

Licenza del progetto: **GNU GPL versione 3** (`GPL-3.0-only`). Tutte le dipendenze hanno licenze
compatibili (MIT, Apache 2.0, MPL 2.0, BSD, ISC, Zlib, Unlicense, Unicode); OpenVINO e il caricatore
Vulkan non vengono distribuiti con il programma.

## 7. Progettazione dell'interfaccia

### 7.1 Disposizione e ordine di tabulazione

```
Menu: File | Trascrizione | Aiuto
────────────────────────────────────────────────────────
File a̲udio:            [percorso (sola lettura)     ] [Sfog̲lia…]
M̲odello:               [large-v3-turbo-q5_0 (scaricato) ▼]
L̲ingua:                [Rilevamento automatico ▼]
Disp̲ositivo:           [Automatico (ora: AMD Radeon RX 6800 XT) ▼]
[ ] Traduci in i̲nglese     [x] Includi times̲tamp
[Av̲via trascrizione]   [An̲nulla]
Avanzamento:           [██████████░░░░░░░░ 54%]
Stato:                 Trascrizione in corso: 54%
Testo tr̲ascritto:
[                                                        ]
[            campo multiriga modificabile                ]
Fo̲rmato di esportazione: [DOCX ▼]   [E̲sporta…]
```

### 7.2 Tasti rapidi

| Tasto | Azione |
|---|---|
| Ctrl+O | Apri file audio |
| F5 | Avvia trascrizione |
| Esc | Annulla l'operazione in corso |
| Ctrl+S | Esporta |
| Ctrl+Maiusc+C | Copia tutta la trascrizione |
| F2 | Annuncia lo stato corrente |
| Ctrl++ / Ctrl+- | Ingrandisci / riduci il testo trascritto |
| F1 | Guida rapida |
| Alt+lettera sottolineata | Vai al campo corrispondente |

### 7.3 Regole di comportamento

1. I pulsanti non disponibili sono disabilitati (lo screen reader lo annuncia) invece di essere nascosti.
2. Il focus non viene mai spostato "a sorpresa", tranne al termine della trascrizione (sul testo).
3. Ogni annuncio è una frase completa in italiano, senza simboli.
4. Gli errori indicano cosa è successo e cosa fare ("Il modello non è presente. Vuoi scaricarlo ora? Dimensione: 574 MB").
5. Le modifiche al testo non vengono perse senza conferma (cambio timestamp, nuova trascrizione, chiusura).

### 7.4 macOS

Stessa disposizione di §7.1 (senza OpenVINO, che serve solo per gli acceleratori Intel), con i tasti del Mac:

| Tasto | Azione |
|---|---|
| Comando+O | Apri file audio (anche trascinamento sull'icona e «Apri con») |
| Comando+R / F5 | Avvia trascrizione |
| Esc / Comando+punto | Interrompi l'operazione in corso |
| Comando+S | Esporta (con la scelta del formato anche nel pannello Esporta) |
| Comando+Maiusc+C | Copia tutta la trascrizione |
| Comando+L / F2 | Annuncia lo stato corrente |
| Comando++ / Comando+- / Comando+0 | Dimensione del testo trascritto |
| Comando+1…5 | Vai a file audio, modello, lingua, testo, formato |
| Comando+? / F1 | Guida rapida |

Nel gruppo delle prestazioni, dopo «Scheda video», c'è la casella «Usa il Neural Engine (Core ML)». Le regole di §7.3 valgono anche qui; il testo non esportato è segnalato dal pallino nel pulsante di chiusura della finestra (`documentEdited`).

## 8. Formati di esportazione

| Formato | Contenuto | Accessibilità |
|---|---|---|
| TXT | Titolo, metadati, testo | UTF-8, leggibile ovunque. |
| HTML | `<h1>` titolo, `<dl>` metadati, `<h2>` + paragrafi | `lang`, `<title>`, `<main>`, contrasto, CSS di stampa. |
| PDF | Titolo, metadati, testo con andata a capo e impaginazione A4 | Taggato (H1, H2, P), lingua e titolo nei metadati, PDF/UA-1. |
| DOCX | Stili *Title*, *Heading 1*, *Normal* | Struttura navigabile in Word, lingua e titolo nelle proprietà. |

Il documento esportato contiene ciò che si vede nel campo di testo, incluse le correzioni manuali.

## 9. Gestione dei modelli

- Cartella predefinita: `%LOCALAPPDATA%\Trascrittore\data\models` (equivalenti su Linux/macOS), modificabile con `TRASCRITTORE_MODELS_DIR`.
  In modalità portable (cartella `dati` o file `portable.txt` accanto all'eseguibile) i modelli stanno in
  `dati\modelli` e le impostazioni in `dati\impostazioni.json`.
- Catalogo: tiny-q5_1 (32 MB, il più leggero), tiny (78 MB), base-q5_1 (60 MB), base (148 MB), small-q5_1 (190 MB), small (488 MB), medium-q5_0 (539 MB), medium (1,5 GB), large-v3-turbo-q5_0 (574 MB, **predefinito**: ottimo compromesso qualità/velocità), large-v3-turbo (1,6 GB), large-v3 (3,1 GB).
- Sorgente: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-<nome>.bin`.
- È sempre possibile scegliere un file `.bin` personalizzato.
- Mac: encoder Core ML per famiglia (`ggml-<famiglia>-encoder.mlmodelc`, dallo stesso repository, in zip con impronta SHA-256): tiny 15 MB, base 38 MB, small 163 MB, medium 568 MB, large-v3 e large-v3-turbo circa 1,2 GB. Valgono anche per le varianti compresse e vengono estratti accanto al modello.

## 10. Gestione degli errori

- Il nucleo usa `anyhow` con messaggi in italiano e contesto (es. "Impossibile decodificare il file audio: formato non supportato").
- La GUI mostra gli errori in finestre di messaggio standard e li annuncia; la CLI li stampa su stderr con codice di uscita ≠ 0.
- L'annullamento è un esito distinto dall'errore (nessun messaggio di errore).

## 11. Verifica e test

1. **Test unitari** del nucleo: formattazione timestamp, esportatori (file validi e contenuto atteso), ricampionamento.
2. **Test di integrazione**: trascrizione di un file di prova generato con la sintesi vocale di Windows.
3. **Test di accessibilità automatizzati**: `scripts/verifica-accessibilita.ps1` interroga ogni controllo via MSAA (l'interfaccia usata da NVDA e JAWS per i controlli Win32) e verifica ruolo, nome, tasto di scelta rapida e stato; con un file audio esegue anche una trascrizione dalla GUI e controlla lo spostamento del focus.
3b. **Verifica automatica su macOS**: `trascrittore --verifica-accessibilita` legge l'albero di accessibilità della propria finestra (gli stessi ruoli, nomi e valori che riceve VoiceOver), controlla nomi, etichette e ordine di tabulazione e, con un file audio, trascrive dalla finestra e controlla che il focus passi al testo; `scripts/verifica-macos.sh` la esegue sulla versione portable.
4. **Test manuali** con NVDA, JAWS e Narrator (lista di controllo in `docs/TEST-ACCESSIBILITA.md`, fase successiva).
5. Validazione dei documenti: PDF/UA (validatore integrato in krilla; verifica esterna con PAC), DOCX aperto in Word/LibreOffice, HTML con validatore W3C.

## 12. Prerequisiti di compilazione (Windows)

- Rust stabile (toolchain MSVC).
- Visual Studio Build Tools 2022 con "Sviluppo di applicazioni desktop C++" (compilatore e CMake inclusi).
- CMake nel `PATH` (va bene quello incluso nei Build Tools).
- LLVM (`winget install LLVM.LLVM`): whisper-rs genera i binding C con libclang; quelli inclusi nel crate sono generati per Linux e non sono validi su Windows.
- Facoltativi: Vulkan SDK (`--features trascrittore-core/vulkan`, schede AMD/NVIDIA/Intel) o CUDA Toolkit (`--features trascrittore-core/cuda`, solo NVIDIA). Per le build con GPU lo script usa la cartella `C:\tr-build`: i sotto-progetti CMake di ggml (generatore di shader) creano percorsi oltre il limite di 260 caratteri di Windows se la cartella di build è profonda.

## 13. Piano di sviluppo

| Fase | Contenuto | Stato |
|---|---|---|
| 1 | Analisi, workspace, nucleo (audio, trascrizione, modelli) | completata |
| 2 | Esportatori TXT, HTML, PDF/UA, DOCX | completata |
| 3 | CLI | completata |
| 4 | GUI Win32 accessibile | completata (verificata via MSAA, non ancora con screen reader reali) |
| 4b | GUI macOS (AppKit) accessibile con VoiceOver | completata (verificata sull'albero di accessibilità, non ancora con VoiceOver da un utente) |
| 5 | Test di accessibilità con NVDA, JAWS e Narrator, rifiniture | da fare |
| 6 | Installer (MSI/Inno Setup), firma del codice | da fare |
| Futuro | Sottotitoli SRT/VTT, elaborazione in batch nella GUI, riconoscimento parlanti, GUI Linux, interfaccia multilingua | — |

## 14. Rischi

| Rischio | Mitigazione |
|---|---|
| Build di whisper.cpp su Windows (CMake/MSVC/libclang) | Prerequisiti documentati; CMake dei Build Tools; feature `whisper` disattivabile per compilare e testare il resto del nucleo. |
| Annunci non letti da alcuni screen reader | Doppio canale: notifiche UIA + spostamento del focus a fine lavoro + finestre di messaggio per gli errori + F2. |
| Modelli grandi lenti su CPU | Predefinito large-v3-turbo quantizzato; feature GPU. |
| Caratteri per il PDF | Uso dei caratteri di sistema (Arial/Segoe UI su Windows, DejaVu/Liberation su Linux) con percorso configurabile. |
| OpenVINO non provato su NPU e GPU Intel | Integrazione verificata con il dispositivo CPU di OpenVINO (testo identico, encoder 28% più veloce); in caso di errore si torna all'encoder normale con un avviso, e un errore durante il lavoro ferma la trascrizione invece di produrre testo errato. |
| Encoder OpenVINO non disponibili per tiny e large-v3-turbo | Pacchetti ufficiali Intel per base, small, medium e large-v3; per gli altri modelli la GUI propone di trascrivere senza OpenVINO e il README spiega la conversione. |
| Annullamento lento | whisper.cpp controlla l'interruzione solo tra un blocco di codifica e l'altro: la GUI spiega l'attesa invece di sembrare bloccata. |
| Bug di `set_abort_callback_safe` in whisper-rs 0.16.0 | Callback di interruzione impostata con l'API di basso livello e un trampolino proprio. |
| Uscita dalla GUI per Mac con il modello caricato su Metal | I distruttori globali di ggml-metal si interrompono se trovano buffer ancora allocati: il modello viene liberato prima dell'uscita (o, se una trascrizione sta finendo, si esce senza distruttori). |
| Compilazione per il Neural Engine | La prima esecuzione con Core ML (e la prima dopo un aggiornamento) richiede alcuni secondi in più: avviene durante il caricamento del modello e viene annunciata. macOS tiene un piccolo indice della cache in `~/Library/Caches/<programma>`, anche in modalità portable (documentato). |
| File audio molto lunghi (memoria) | ~230 MB per un'ora di audio: accettabile; elaborazione a blocchi in fase futura. |
