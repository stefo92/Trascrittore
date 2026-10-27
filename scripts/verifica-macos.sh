#!/bin/bash
# Verifica la versione portable per macOS (dist/Trascrittore-portable-macos):
# - l'eseguibile è arm64, ha una firma valida ed è collegato solo a librerie di sistema;
# - copiato in una cartella temporanea, tiene modelli e dati nella sua cartella «dati»;
# - trascrive con la GPU (Metal) e con il processore;
# - non scrive nulla nella Libreria dell'utente.
#
# Va eseguito su un Mac Apple Silicon dopo «bash scripts/build-macos.sh build --release».
# Scarica il modello tiny-q5_1 (32 MB), a meno che TRASCRITTORE_MODELLO_PROVA indichi un file
# .bin già presente. Senza GPU Metal (es. in una macchina virtuale) la prova sulla GPU viene
# saltata, a meno che VERIFICA_GPU_OBBLIGATORIA=1.
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
    grep -E "Completato|non si è attivata" <<< "$output" || true
    if grep -q "non si è attivata" <<< "$output"; then
        fail "la scheda video non si è attivata"
    fi
    if [ ! -s "$work/$name/prova.txt" ]; then
        fail "nessun testo trascritto con $name"
    else
        sed -n '/^Testo trascritto/,$p' "$work/$name/prova.txt" | tail -n +3
    fi
}

if grep -q '^gpu:' <<< "$devices"; then
    echo "=== Trascrizione sulla GPU (Metal)"
    transcribe gpu --dispositivo gpu
elif [ "${VERIFICA_GPU_OBBLIGATORIA:-0}" = "1" ]; then
    fail "nessuna GPU Metal utilizzabile"
else
    echo "AVVISO: nessuna GPU Metal utilizzabile (normale in una macchina virtuale): salto la prova sulla GPU."
fi
echo "=== Trascrizione sul processore"
transcribe cpu --dispositivo cpu

if [ -z "${TRASCRITTORE_MODELLO_PROVA:-}" ] && [ ! -f "$copy/dati/modelli/ggml-$model.bin" ]; then
    fail "il modello non è stato scaricato in dati/modelli"
fi

echo "=== Scritture nella Libreria dell'utente"
written="$(find "$HOME/Library/Application Support" "$HOME/Library/Preferences" \
    "$HOME/Library/Caches" -newer "$marker" -iname '*trascrittore*' 2>/dev/null || true)"
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
