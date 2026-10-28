#!/bin/bash
# Compila Trascrittore per macOS su Apple Silicon ed esegue cargo con gli argomenti indicati.
#
# La GPU del Mac (Metal) viene inclusa automaticamente; con --solo-cpu si compila la versione che
# usa solo il processore. Dopo una build release viene preparata la versione portable in
# dist/Trascrittore-portable-macos (interfaccia grafica Trascrittore.app e trascrittore-cli),
# insieme al suo zip.
#
# Requisiti: Xcode Command Line Tools (xcode-select --install), CMake (brew install cmake) e Rust
# con il target aarch64-apple-darwin (rustup target add aarch64-apple-darwin).
#
# Esempi:
#   bash scripts/build-macos.sh build --release
#   bash scripts/build-macos.sh test --workspace
#   bash scripts/build-macos.sh build --release --solo-cpu
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

if [ "$(uname -s)" != "Darwin" ]; then
    echo "Questo script va eseguito su macOS (su Windows usa scripts\\build.ps1)." >&2
    exit 1
fi

target="aarch64-apple-darwin"
cpu_only=0
release=0
args=()
for a in "$@"; do
    case "$a" in
        --solo-cpu) cpu_only=1 ;;
        *)
            args+=("$a")
            if [ "$a" = "--release" ]; then release=1; fi
            ;;
    esac
done
if [ ${#args[@]} -eq 0 ]; then
    echo "Uso: bash scripts/build-macos.sh <comando cargo> [argomenti], es. build --release" >&2
    exit 1
fi
cmd="${args[0]}"

# Versione minima di macOS degli eseguibili: Rust, CMake e clang la leggono da qui.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
# .cargo/config.toml imposta questi flag per il compilatore di Visual Studio (Windows). Le
# variabili d'ambiente hanno la precedenza su [env], quindi qui si usano i flag di clang.
export CMAKE_C_FLAGS_RELEASE="-O3 -DNDEBUG"
export CMAKE_CXX_FLAGS_RELEASE="-O3 -DNDEBUG"

# Le opzioni aggiunte vanno subito dopo il comando, prima di un eventuale «--».
extra=()
case "$cmd" in
    build|run|test|check)
        if command -v rustup >/dev/null; then
            installed="$(rustup target list --installed)"
            if ! grep -qx "$target" <<< "$installed"; then
                echo "Manca il target Rust $target: rustup target add $target" >&2
                exit 1
            fi
        fi
        extra+=(--target "$target")
        if [ "$cpu_only" -eq 0 ]; then
            extra+=(--features trascrittore-core/metal)
            echo "Supporto per la scheda video (Metal) incluso. Usa --solo-cpu per escluderlo."
        fi
        ;;
esac

rest=("${args[@]:1}")
set +e
cargo "$cmd" ${extra[@]+"${extra[@]}"} ${rest[@]+"${rest[@]}"}
code=$?
set -e
if [ $code -ne 0 ] || [ "$cmd" != "build" ] || [ $release -eq 0 ]; then
    exit $code
fi

# Versione portable: cartella pronta da copiare su una chiavetta USB, più il suo zip.
# La cartella «dati» non viene svuotata (può contenere modelli scaricati) e non entra nello zip.
out="${CARGO_TARGET_DIR:-$root/target}/$target/release"
if [ ! -f "$out/trascrittore-cli" ]; then
    exit 0
fi
dist="$root/dist"
name="Trascrittore-portable-macos"
portable="$dist/$name"
mkdir -p "$portable/dati/modelli"
cp "$out/trascrittore-cli" "$portable/"
# Su Apple Silicon ogni eseguibile deve avere una firma valida, anche solo «ad hoc»: la si rinnova
# perché la rimozione dei simboli (strip) può invalidare quella apposta dal linker.
codesign --force --sign - "$portable/trascrittore-cli"
cp "$root/portable/LEGGIMI-macOS.txt" "$portable/LEGGIMI.txt"
cp "$root/portable/portable.txt" "$portable/portable.txt"
# La GPL chiede di accompagnare l'eseguibile con il testo della licenza.
cp "$root/LICENSE" "$portable/LICENZA.txt"

# Interfaccia grafica: pacchetto Trascrittore.app accanto a trascrittore-cli. Il programma
# riconosce il pacchetto e usa la cartella «dati» che lo contiene (vedi crates/core/src/paths.rs).
app="$portable/Trascrittore.app"
if [ -f "$out/trascrittore" ]; then
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$out/trascrittore" "$app/Contents/MacOS/trascrittore"
    version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)"
    sed -e "s/@VERSIONE@/$version/g" -e "s/@MACOS_MINIMO@/$MACOSX_DEPLOYMENT_TARGET/g" \
        "$root/portable/Info-macOS.plist" > "$app/Contents/Info.plist"
    plutil -lint -s "$app/Contents/Info.plist"
    printf 'APPL????' > "$app/Contents/PkgInfo"
    # L'icona viene disegnata una sola volta e conservata nella cartella di build.
    icon="$out/Trascrittore.icns"
    if [ ! -f "$icon" ] || [ "$root/scripts/crea-icona-macos.swift" -nt "$icon" ]; then
        swift "$root/scripts/crea-icona-macos.swift" "$icon" || rm -f "$icon"
    fi
    if [ -f "$icon" ]; then
        cp "$icon" "$app/Contents/Resources/Trascrittore.icns"
    fi
    codesign --force --sign - "$app"
else
    rm -rf "$app"
fi

staging="$(mktemp -d)"
mkdir -p "$staging/$name/dati/modelli"
cp "$portable/trascrittore-cli" "$portable/LEGGIMI.txt" "$portable/portable.txt" \
    "$portable/LICENZA.txt" "$staging/$name/"
if [ -d "$app" ]; then
    ditto "$app" "$staging/$name/Trascrittore.app"
fi
rm -f "$dist/$name.zip"
# ditto conserva permessi di esecuzione e firma, come l'Utility Archivio di macOS; --norsrc
# esclude gli attributi estesi, che altri programmi estrarrebbero come file «._…» dentro il
# pacchetto .app, invalidandone la firma.
ditto -c -k --norsrc --keepParent "$staging/$name" "$dist/$name.zip"
rm -rf "$staging"

kind="con supporto per la scheda video Metal"
if [ "$cpu_only" -eq 1 ]; then kind="solo processore"; fi
echo "Versione portable per macOS ($kind) pronta in: $portable"
echo "Archivio della versione portable: $dist/$name.zip"
