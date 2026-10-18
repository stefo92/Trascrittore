<#
.SYNOPSIS
    Prepara l'ambiente di compilazione (Visual Studio, CMake, LLVM, Vulkan) ed esegue cargo.

.DESCRIPTION
    whisper.cpp richiede gli header di MSVC per generare i binding con libclang e CMake per
    la compilazione. Questo script carica vcvars64.bat dei Build Tools, aggiunge CMake, LLVM e
    il Vulkan SDK al PATH e poi esegue cargo con gli argomenti indicati.

    Se il Vulkan SDK è installato, il supporto per le schede video (AMD, NVIDIA, Intel) viene
    incluso automaticamente; con --solo-cpu si compila la versione che usa solo il processore.
    Dopo una build release i programmi pronti vengono copiati nella cartella dist\ del progetto.

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
}
exit $code
