<#
.SYNOPSIS
    Prepara l'ambiente di compilazione (Visual Studio, CMake, LLVM, Vulkan) ed esegue cargo.

.DESCRIPTION
    whisper.cpp richiede gli header di MSVC per generare i binding con libclang e CMake per
    la compilazione. Questo script carica vcvars64.bat dei Build Tools, aggiunge CMake, LLVM e
    il Vulkan SDK al PATH e poi esegue cargo con gli argomenti indicati.

    Se il Vulkan SDK è installato, il supporto per le schede video (AMD, NVIDIA, Intel) viene
    incluso automaticamente; con --solo-cpu si compila la versione che usa solo il processore.
    Dopo una build release i programmi pronti vengono copiati nella cartella dist\ del progetto,
    insieme alla versione portable (dist\Trascrittore-portable e dist\Trascrittore-portable.zip).

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1 build --release
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1 test --workspace
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1 build --release --solo-cpu
#>

$cargoArgs = [System.Collections.Generic.List[string]]::new()
$cpuOnly = $false
foreach ($a in $args) {
    if ($a -eq "--solo-cpu") { $cpuOnly = $true } else { $cargoArgs.Add([string]$a) }
}

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere)) {
    Write-Error "Visual Studio Installer non trovato. Installa i Build Tools di Visual Studio 2022."
    exit 1
}
$vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vsPath) {
    Write-Error "Strumenti C++ di Visual Studio non trovati (carico di lavoro 'Sviluppo di applicazioni desktop C++')."
    exit 1
}

# Importa le variabili d'ambiente di vcvars64 (INCLUDE, LIB, PATH...).
$vcvars = Join-Path $vsPath "VC\Auxiliary\Build\vcvars64.bat"
cmd /c "`"$vcvars`" >nul && set" | ForEach-Object {
    if ($_ -match "^([^=]+)=(.*)$") {
        Set-Item -Path "env:$($matches[1])" -Value $matches[2]
    }
}

$cmakeDir = Join-Path $vsPath "Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"
if (-not (Get-Command cmake -ErrorAction SilentlyContinue) -and (Test-Path $cmakeDir)) {
    $env:PATH = "$cmakeDir;$env:PATH"
}

$llvmDir = "C:\Program Files\LLVM\bin"
if (-not $env:LIBCLANG_PATH -and (Test-Path (Join-Path $llvmDir "libclang.dll"))) {
    $env:LIBCLANG_PATH = $llvmDir
}
if (-not $env:LIBCLANG_PATH) {
    Write-Warning "libclang non trovata: installa LLVM con 'winget install LLVM.LLVM'."
}

# Vulkan SDK: la variabile di sistema può mancare nelle sessioni aperte prima dell'installazione.
if (-not $env:VULKAN_SDK) {
    $machine = [Environment]::GetEnvironmentVariable("VULKAN_SDK", "Machine")
    if ($machine) {
        $env:VULKAN_SDK = $machine
    } elseif (Test-Path "C:\VulkanSDK") {
        $latest = Get-ChildItem "C:\VulkanSDK" -Directory | Sort-Object Name | Select-Object -Last 1
        if ($latest) { $env:VULKAN_SDK = $latest.FullName }
    }
}
if ($env:VULKAN_SDK) {
    $env:PATH = (Join-Path $env:VULKAN_SDK "Bin") + ";$env:PATH"
}

# Supporto GPU incluso automaticamente quando possibile.
$command = if ($cargoArgs.Count -gt 0) { $cargoArgs[0] } else { "" }
$gpu = [bool]($cargoArgs -match "vulkan|cuda")
if (-not $gpu -and -not $cpuOnly -and @("build", "run", "test", "check") -contains $command) {
    if ($env:VULKAN_SDK) {
        $cargoArgs.Add("--features")
        $cargoArgs.Add("trascrittore-core/vulkan")
        $gpu = $true
        Write-Output "Supporto per le schede video (Vulkan) incluso. Usa --solo-cpu per escluderlo."
    } else {
        Write-Warning "Vulkan SDK non trovato: compilo la versione che usa solo il processore. Per le schede video: winget install KhronosGroup.VulkanSDK"
    }
}

# Con il supporto GPU i sotto-progetti CMake di ggml creano percorsi oltre i 260 caratteri
# di Windows se la cartella di build è profonda: si usa una cartella corta.
# Si può sceglierne un'altra con CARGO_TARGET_DIR.
if ($gpu -and -not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = Join-Path $env:SystemDrive "tr-build"
}
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot "..\target" }

& cargo @cargoArgs
$code = $LASTEXITCODE

# Programmi pronti in un'unica cartella, per non confondere versioni diverse.
if ($code -eq 0 -and $command -eq "build" -and ($cargoArgs -contains "--release")) {
    $dist = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\dist"))
    New-Item -ItemType Directory -Force $dist | Out-Null
    foreach ($exe in "trascrittore.exe", "trascrittore-cli.exe") {
        $source = Join-Path $targetDir "release\$exe"
        if (Test-Path $source) { Copy-Item $source $dist -Force }
    }
    $kind = if ($gpu) { "con supporto per le schede video" } else { "solo processore" }
    Write-Output "Programmi pronti ($kind) in: $dist"

    # Versione portable: cartella pronta da copiare su una chiavetta USB, più il suo zip.
    # La cartella «dati» non viene svuotata (può contenere modelli scaricati) e non entra nello zip.
    $exes = "trascrittore.exe", "trascrittore-cli.exe" | ForEach-Object { Join-Path $dist $_ }
    if (($exes | Where-Object { -not (Test-Path $_) }).Count -eq 0) {
        $portable = Join-Path $dist "Trascrittore-portable"
        New-Item -ItemType Directory -Force (Join-Path $portable "dati\modelli") | Out-Null
        foreach ($exe in $exes) { Copy-Item $exe $portable -Force }
        # Testi in UTF-8 con BOM e a capo CRLF, leggibili anche con il Blocco note di Windows meno recenti.
        $utf8Bom = New-Object System.Text.UTF8Encoding $true
        foreach ($name in "LEGGIMI.txt", "portable.txt") {
            $text = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot "..\portable\$name"))
            $text = $text -replace "`r`n", "`n" -replace "`n", "`r`n"
            [System.IO.File]::WriteAllText((Join-Path $portable $name), $text, $utf8Bom)
        }

        $zipPath = Join-Path $dist "Trascrittore-portable.zip"
        if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
        Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
        $zip = [System.IO.Compression.ZipFile]::Open($zipPath, [System.IO.Compression.ZipArchiveMode]::Create)
        try {
            foreach ($name in "trascrittore.exe", "trascrittore-cli.exe", "LEGGIMI.txt", "portable.txt") {
                [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                    $zip, (Join-Path $portable $name), "Trascrittore-portable/$name",
                    [System.IO.Compression.CompressionLevel]::Optimal)
            }
            [void]$zip.CreateEntry("Trascrittore-portable/dati/modelli/")
        } finally {
            $zip.Dispose()
        }
        Write-Output "Versione portable pronta in: $portable"
        Write-Output "Archivio della versione portable: $zipPath"
    }
}
exit $code
