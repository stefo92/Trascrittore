<#
.SYNOPSIS
    Prepara l'ambiente di compilazione (Visual Studio, CMake, LLVM) ed esegue cargo.

.DESCRIPTION
    whisper.cpp richiede gli header di MSVC per generare i binding con libclang e CMake per
    la compilazione. Questo script carica vcvars64.bat dei Build Tools, aggiunge CMake e LLVM
    al PATH e poi esegue cargo con gli argomenti indicati.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1 build --release
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1 test -p trascrittore-core
#>

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

& cargo @args
exit $LASTEXITCODE
