//! Interfaccia a riga di comando di Trascrittore.
//!
//! L'avanzamento viene stampato su righe separate (non con il ritorno a capo continuo),
//! così è leggibile anche con uno screen reader nel terminale.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use trascrittore_core::device::{self, Device};
use trascrittore_core::document::{Document, DocumentInfo};
use trascrittore_core::export::{self, ExportFormat};
use trascrittore_core::pipeline::{self, Job, Progress, ProgressFn};
use trascrittore_core::transcriber::{self, TranscribeOptions, Transcriber};
use trascrittore_core::transcript::format_duration_words;
use trascrittore_core::{is_cancelled, languages, models};

#[derive(Parser)]
#[command(
    name = "trascrittore-cli",
    version,
    about = "Trascrive file audio in locale con Whisper ed esporta il testo in TXT, HTML, PDF o DOCX."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Trascrive uno o più file audio.
    #[command(visible_alias = "transcribe")]
    Trascrivi(TranscribeArgs),
    /// Elenca i modelli Whisper e indica quelli già scaricati.
    #[command(visible_alias = "models")]
    Modelli,
    /// Scarica un modello Whisper dal repository ufficiale.
    #[command(visible_alias = "download")]
    Scarica {
        /// Nome del modello (vedi il comando «modelli»).
        modello: String,
    },
    /// Elenca le lingue con il loro codice.
    #[command(visible_alias = "languages")]
    Lingue,
    /// Elenca i dispositivi di calcolo: processore e schede video utilizzabili.
    #[command(visible_alias = "devices")]
    Dispositivi,
}

#[derive(Args)]
struct TranscribeArgs {
    /// File audio da trascrivere.
    #[arg(required = true)]
    file: Vec<PathBuf>,

    /// Modello: nome del catalogo (es. small) oppure percorso di un file .bin.
    #[arg(short, long, alias = "model", default_value = models::DEFAULT_MODEL_ID)]
    modello: String,

    /// Dispositivo di calcolo: auto (scheda video se disponibile), cpu, gpu oppure gpu:N.
    #[arg(short, long, alias = "device", default_value = "auto")]
    dispositivo: Device,

    /// Lingua parlata: codice ISO (es. it, en) oppure «auto» per il rilevamento automatico.
    #[arg(short, long, alias = "language", default_value = "auto")]
    lingua: String,

    /// Traduce il parlato in inglese.
    #[arg(long, alias = "translate")]
    traduci: bool,

    /// Inserisce l'orario di inizio di ogni frase.
    #[arg(short, long, alias = "timestamps")]
    timestamp: bool,

    /// Formati di esportazione separati da virgola: txt, html, pdf, docx.
    #[arg(short, long, alias = "format", value_delimiter = ',', default_value = "txt")]
    formato: Vec<ExportFormat>,

    /// Cartella di destinazione (predefinita: quella del file audio).
    #[arg(short = 'o', long, alias = "output")]
    uscita: Option<PathBuf>,

    /// Numero di thread da usare (predefinito: automatico).
    #[arg(long, alias = "threads")]
    thread: Option<usize>,

    /// Testo iniziale che orienta il riconoscimento (nomi propri, sigle, stile).
    #[arg(long)]
    prompt: Option<String>,

    /// Scarica il modello se non è ancora presente.
    #[arg(long, alias = "download-model")]
    scarica_modello: bool,

    /// Non mostra l'avanzamento.
    #[arg(short, long, alias = "quiet")]
    silenzioso: bool,

    /// Stampa la trascrizione anche sullo standard output.
    #[arg(long, alias = "stdout")]
    stampa: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Trascrivi(args) => transcribe(args),
        Command::Modelli => list_models(),
        Command::Scarica { modello } => {
            let info = find_model(&modello);
            info.and_then(|info| download_model(info, false).map(|_| ()))
        }
        Command::Lingue => {
            for lang in languages::LANGUAGES {
                println!("{}\t{}", lang.code, lang.name);
            }
            println!("auto\tRilevamento automatico");
            Ok(())
        }
        Command::Dispositivi => {
            list_devices();
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if is_cancelled(&e) => {
            eprintln!("Operazione annullata.");
            ExitCode::from(130)
        }
        Err(e) => {
            eprintln!("Errore: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn transcribe(args: TranscribeArgs) -> Result<()> {
    let language = match args.lingua.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => None,
        code if transcriber::is_supported_language(code) => Some(code.to_string()),
        code => bail!("Lingua «{code}» non riconosciuta. Usa il comando «lingue» per l'elenco."),
    };
    // Verifica subito il dispositivo, prima di eventuali download.
    args.dispositivo.resolve(device::list_gpus())?;
    let (model_path, model_name) = resolve_model(&args.modello, args.scarica_modello, args.silenzioso)?;
    if let Some(dir) = &args.uscita {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Impossibile creare la cartella «{}».", dir.display()))?;
    }

    let options = TranscribeOptions {
        language,
        translate: args.traduci,
        threads: args.thread,
        initial_prompt: args.prompt.clone(),
    };
    let mut cache: Option<Transcriber> = None;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut failures = 0usize;

    for (index, audio_path) in args.file.iter().enumerate() {
        if !args.silenzioso {
            eprintln!(
                "File {} di {}: {}",
                index + 1,
                args.file.len(),
                audio_path.display()
            );
        }
        let job = Job {
            audio_path: audio_path.clone(),
            model_path: model_path.clone(),
            device: args.dispositivo,
            options: options.clone(),
        };
        let started = Instant::now();
        let outcome = pipeline::run(
            &job,
            &mut cache,
            progress_printer(args.silenzioso),
            Arc::clone(&cancel),
        )
        .and_then(|outcome| {
            if !args.silenzioso {
                eprintln!(
                    "Completato in {} su {}.",
                    format_duration_words(started.elapsed().as_millis() as u64),
                    outcome.device.label()
                );
            }
            let transcript = outcome.transcript;
            let text = transcript.to_text(args.timestamp);
            if args.stampa {
                println!("{text}");
            }
            let doc = Document::from_text(
                &text,
                &DocumentInfo {
                    audio_path: Some(audio_path),
                    duration_ms: Some(transcript.audio_duration_ms),
                    language: transcript.language.as_deref(),
                    model_name: Some(&model_name),
                    translated: args.traduci,
                },
            );
            let out_dir = args
                .uscita
                .clone()
                .or_else(|| audio_path.parent().map(Path::to_path_buf))
                .unwrap_or_default();
            let stem = audio_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "trascrizione".to_string());
            for format in &args.formato {
                let path = out_dir.join(format!("{stem}.{}", format.extension()));
                export::export(&doc, *format, &path)?;
                if !args.silenzioso {
                    eprintln!("Salvato: {}", path.display());
                }
            }
            Ok(())
        });

        if let Err(e) = outcome {
            if is_cancelled(&e) {
                return Err(e);
            }
            failures += 1;
            eprintln!("Errore con «{}»: {e:#}", audio_path.display());
        }
    }

    match failures {
        0 => Ok(()),
        n => bail!("{n} file su {} non sono stati trascritti.", args.file.len()),
    }
}

/// Stampa l'avanzamento su righe separate, a passi del 10%.
fn progress_printer(quiet: bool) -> ProgressFn {
    #[derive(PartialEq)]
    enum Phase {
        None,
        Decoding,
        Loading,
        Transcribing(i32),
    }
    let phase = Mutex::new(Phase::None);
    Arc::new(move |progress| {
        if quiet {
            return;
        }
        let Ok(mut phase) = phase.lock() else {
            return;
        };
        match progress {
            Progress::Decoding(_) => {
                if *phase != Phase::Decoding {
                    eprintln!("Decodifica dell'audio…");
                    *phase = Phase::Decoding;
                }
            }
            Progress::LoadingModel(device) => {
                eprintln!("Caricamento del modello su {}…", device.label());
                *phase = Phase::Loading;
            }
            Progress::Transcribing(percent) => {
                let last = match *phase {
                    Phase::Transcribing(last) => last,
                    _ => {
                        eprintln!("Trascrizione in corso…");
                        0
                    }
                };
                let step = percent / 10 * 10;
                if step > last {
                    eprintln!("Trascrizione: {step}%");
                }
                *phase = Phase::Transcribing(step.max(last));
            }
        }
    })
}

fn find_model(id: &str) -> Result<&'static models::ModelInfo> {
    models::find(id).with_context(|| {
        format!("Modello «{id}» sconosciuto. Usa il comando «modelli» per l'elenco.")
    })
}

/// Restituisce percorso e nome del modello, scaricandolo se richiesto.
fn resolve_model(spec: &str, download: bool, quiet: bool) -> Result<(PathBuf, String)> {
    let as_path = Path::new(spec);
    if as_path.is_file() {
        let name = as_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| spec.to_string());
        return Ok((as_path.to_path_buf(), name));
    }
    let info = find_model(spec)?;
    let path = info.path()?;
    if path.is_file() {
        return Ok((path, info.id.to_string()));
    }
    if !download {
        bail!(
            "Il modello «{}» non è ancora stato scaricato ({}). Scaricalo con «trascrittore-cli scarica {}» oppure aggiungi l'opzione --scarica-modello.",
            info.id,
            models::format_size(info.size_bytes),
            info.id
        );
    }
    let path = download_model(info, quiet)?;
    Ok((path, info.id.to_string()))
}

fn download_model(info: &models::ModelInfo, quiet: bool) -> Result<PathBuf> {
    if info.is_downloaded() {
        let path = info.path()?;
        eprintln!("Il modello «{}» è già presente: {}", info.id, path.display());
        return Ok(path);
    }
    if !quiet {
        eprintln!(
            "Download del modello «{}» (circa {}) da {}",
            info.id,
            models::format_size(info.size_bytes),
            info.url()
        );
    }
    let cancel = AtomicBool::new(false);
    let mut last_step = 0u64;
    let path = models::download(
        info,
        |done, total| {
            if quiet {
                return;
            }
            if let Some(total) = total.filter(|t| *t > 0) {
                let step = done * 100 / total / 10 * 10;
                if step > last_step {
                    last_step = step;
                    eprintln!("Download: {step}%");
                }
            }
        },
        &cancel,
    )?;
    eprintln!("Modello salvato in {}", path.display());
    Ok(path)
}

fn list_devices() {
    let backends = device::compiled_gpu_backends();
    println!("cpu\tProcessore (CPU)");
    for gpu in device::list_gpus() {
        println!("gpu:{}\t{}", gpu.index, gpu.label());
    }
    if backends.is_empty() {
        println!(
            "Questa versione non include il supporto per le schede video: ricompila con --features vulkan (AMD, NVIDIA, Intel) o --features cuda (NVIDIA)."
        );
    } else if device::list_gpus().is_empty() {
        println!(
            "Supporto GPU incluso ({}), ma nessuna scheda video utilizzabile: controlla i driver.",
            backends.join(", ")
        );
    }
    if let Ok(resolved) = Device::Auto.resolve(device::list_gpus()) {
        println!("Con «auto» verrà usato: {}.", resolved.label());
    }
}

fn list_models() -> Result<()> {
    println!("Cartella dei modelli: {}", models::models_dir()?.display());
    for info in models::MODELS {
        let mut line = format!(
            "{:<22} {:>7}  {}",
            info.id,
            models::format_size(info.size_bytes),
            info.description
        );
        if info.is_downloaded() {
            line.push_str(" [scaricato]");
        }
        if info.id == models::DEFAULT_MODEL_ID {
            line.push_str(" [predefinito]");
        }
        println!("{line}");
    }
    Ok(())
}
