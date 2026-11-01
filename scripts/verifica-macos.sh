#!/bin/bash
# Verifica la versione portable per macOS (dist/Trascrittore-portable-macos):
# - gli eseguibili sono arm64, hanno una firma valida e sono collegati solo a librerie di sistema;
# - copiata in una cartella temporanea, tiene modelli e dati nella sua cartella «dati»;
# - la CLI trascrive con la GPU (Metal) e il Neural Engine (Core ML, encoder scaricato in
#   «dati/modelli») e con il solo processore;
# - l'interfaccia grafica (Trascrittore.app) espone a VoiceOver ruoli, nomi e ordine di
#   tabulazione corretti, trascrive dalla finestra e al termine porta il focus sul testo;
# - non scrive nulla nella Libreria dell'utente (a parte il piccolo indice della compilazione
#   per il Neural Engine che macOS tiene in ~/Library/Caches, segnalato a parte).
#
# Va eseguito su un Mac Apple Silicon dopo «bash scripts/build-macos.sh build --release».
# Scarica il modello tiny-q5_1 (32 MB), a meno che TRASCRITTORE_MODELLO_PROVA indichi un file
# .bin già presente. Senza GPU Metal (es. in una macchina virtuale) la prova sulla GPU viene
# saltata, a meno che VERIFICA_GPU_OBBLIGATORIA=1. La prova dell'interfaccia grafica richiede
# una sessione grafica attiva; VERIFICA_GUI=0 la salta.
#
# Esempio:
#   bash scripts/verifica-macos.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
portable="$root/dist/Trascrittore-portable-macos"
problems=0
fail() {
    echo "PROBLEMA: $*"
    problems=$((problems + 1))
}

if [ ! -x "$portable/trascrittore-cli" ]; then
    echo "Manca $portable/trascrittore-cli: esegui prima bash scripts/build-macos.sh build --release" >&2
    exit 2
fi

echo "=== Eseguibile"
exe="$portable/trascrittore-cli"
archs="$(lipo -archs "$exe")"
echo "Architetture: $archs"
[ "$archs" = "arm64" ] || fail "architettura inattesa: $archs"
if codesign --verify --strict "$exe"; then echo "Firma valida"; else fail "firma non valida"; fi
echo "macOS minimo: $(otool -l "$exe" | awk '/minos/ && !m { m = $2 } END { print m }')"
echo "Librerie collegate:"
otool -L "$exe" | tail -n +2 | awk '{ print "  " $1 }'
external="$(otool -L "$exe" | tail -n +2 | awk '{ print $1 }' | grep -v -E '^(/usr/lib/|/System/Library/)' || true)"
[ -z "$external" ] || fail "librerie esterne a macOS (non portabili): $external"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
copy="$work/Trascrittore"
cp -R "$portable" "$copy"
cli="$copy/trascrittore-cli"

model="tiny-q5_1"
if [ -n "${TRASCRITTORE_MODELLO_PROVA:-}" ]; then
    cp "$TRASCRITTORE_MODELLO_PROVA" "$copy/dati/modelli/"
    model="$copy/dati/modelli/$(basename "$TRASCRITTORE_MODELLO_PROVA")"
fi

audio="$work/prova.wav"
text="Buongiorno a tutti. Questa è una prova del programma Trascrittore."
if grep -q -E '^Alice +it' <<< "$(say -v '?')"; then
    say -v Alice --file-format=WAVE --data-format=LEI16@16000 -o "$audio" "$text"
else
    say --file-format=WAVE --data-format=LEI16@16000 -o "$audio" "$text"
fi

marker="$work/inizio"
touch "$marker"
sleep 1

echo "=== Modalità portable"
models_output="$("$cli" modelli)"
sed -n '1,2p' <<< "$models_output"
# Si confronta solo la fine del percorso: su macOS /var è un collegamento a /private/var.
grep -q "^Modalità portable: impostazioni e modelli in .*/Trascrittore/dati$" <<< "$models_output" \
    || fail "la copia non è in modalità portable"

echo "=== Dispositivi"
devices="$("$cli" dispositivi)"
echo "$devices"
coreml=0
if grep -q "^Neural Engine (Core ML): incluso" <<< "$devices"; then coreml=1; fi

transcribe() {
    local name="$1"
    shift
    local output
    if ! output="$("$cli" trascrivi "$audio" -m "$model" --scarica-modello -l it -f txt \
        -o "$work/$name" "$@" 2>&1)"; then
        echo "$output"
        fail "trascrizione con $name non riuscita"
        return
    fi
    grep -E "Completato|non si è attivata|Neural Engine|Core ML" <<< "$output" || true
    if grep -q "non si è attivata" <<< "$output"; then
        fail "la scheda video non si è attivata"
    fi
    if grep -q "Core ML non si è attivato" <<< "$output"; then
        fail "il Neural Engine (Core ML) non si è attivato"
    fi
    if [ "${expect_coreml:-0}" = "1" ] && ! grep -q "Encoder eseguito sul Neural Engine" <<< "$output"; then
        fail "l'encoder non è stato eseguito sul Neural Engine"
    fi
    if [ ! -s "$work/$name/prova.txt" ]; then
        fail "nessun testo trascritto con $name"
    else
        sed -n '/^Testo trascritto/,$p' "$work/$name/prova.txt" | tail -n +3
    fi
}

if grep -q '^gpu:' <<< "$devices"; then
    echo "=== Trascrizione sulla GPU (Metal) con il Neural Engine"
    expect_coreml=$coreml transcribe gpu --dispositivo gpu
elif [ "${VERIFICA_GPU_OBBLIGATORIA:-0}" = "1" ]; then
    fail "nessuna GPU Metal utilizzabile"
else
    echo "AVVISO: nessuna GPU Metal utilizzabile (normale in una macchina virtuale): salto la prova sulla GPU."
fi
echo "=== Trascrizione sul solo processore"
transcribe cpu --dispositivo cpu --senza-neural-engine
if [ $coreml -eq 1 ] && ! find "$copy/dati/modelli" -maxdepth 1 -name '*-encoder.mlmodelc' | grep -q .; then
    fail "l'encoder per il Neural Engine non è stato scaricato in dati/modelli"
fi

if [ -z "${TRASCRITTORE_MODELLO_PROVA:-}" ] && [ ! -f "$copy/dati/modelli/ggml-$model.bin" ]; then
    fail "il modello non è stato scaricato in dati/modelli"
fi

# Esegue un comando e lo interrompe se dura più di $1 secondi.
with_timeout() {
    local seconds="$1"
    shift
    "$@" &
    local pid=$!
    ( sleep "$seconds"; kill "$pid" 2>/dev/null ) &
    local watcher=$!
    local code=0
    wait "$pid" || code=$?
    kill "$watcher" 2>/dev/null || true
    wait "$watcher" 2>/dev/null || true
    return $code
}

echo "=== Interfaccia grafica (Trascrittore.app)"
app="$copy/Trascrittore.app"
gui="$app/Contents/MacOS/trascrittore"
if [ ! -x "$gui" ]; then
    fail "manca Trascrittore.app nella versione portable"
else
    gui_archs="$(lipo -archs "$gui")"
    [ "$gui_archs" = "arm64" ] || fail "architettura inattesa dell'interfaccia grafica: $gui_archs"
    if codesign --verify --deep --strict "$app"; then
        echo "Firma di Trascrittore.app valida"
    else
        fail "firma di Trascrittore.app non valida"
    fi
    plutil -lint -s "$app/Contents/Info.plist" || fail "Info.plist non valido"
    gui_external="$(otool -L "$gui" | tail -n +2 | awk '{ print $1 }' \
        | grep -v -E '^(/usr/lib/|/System/Library/)' || true)"
    [ -z "$gui_external" ] || fail "librerie esterne a macOS nell'interfaccia grafica: $gui_external"
    if [ "${VERIFICA_GUI:-1}" = "0" ]; then
        echo "AVVISO: VERIFICA_GUI=0, salto la prova della finestra."
    elif with_timeout 300 "$gui" --verifica-accessibilita --modello "$model" "$audio" \
        > "$work/gui.txt" 2>&1; then
        # L'albero completo è lungo: si mostrano l'ordine di tabulazione e l'esito.
        sed -n '/^=== Ordine di tabulazione/,$p' "$work/gui.txt"
    else
        cat "$work/gui.txt"
        fail "verifica dell'interfaccia grafica non riuscita"
    fi
fi

echo "=== Scritture nella Libreria dell'utente"
found="$(find "$HOME/Library/Application Support" "$HOME/Library/Preferences" \
    "$HOME/Library/Caches" -newer "$marker" -iname '*trascrittore*' 2>/dev/null || true)"
written=""
while IFS= read -r path; do
    [ -n "$path" ] || continue
    # Core ML tiene in ~/Library/Caches/<programma> l'indice dei modelli compilati per il
    # Neural Engine (com.apple.e5rt...): lo crea macOS, come la cache degli shader Metal.
    if [ "${path#"$HOME/Library/Caches/"}" != "$path" ] && [ -d "$path" ] \
        && [ -z "$(find "$path" -mindepth 1 -maxdepth 1 ! -name 'com.apple.e5rt.*')" ]; then
        echo "Cache di Core ML creata da macOS (normale): $path"
        continue
    fi
    written="$written$path "
done <<< "$found"
if [ -n "$written" ]; then
    fail "file di Trascrittore nella Libreria dell'utente: $written"
else
    echo "Nessun file di Trascrittore nella Libreria dell'utente."
fi

echo
if [ $problems -gt 0 ]; then
    echo "$problems problemi trovati."
    exit 1
fi
echo "Nessun problema: la versione portable per macOS funziona."
