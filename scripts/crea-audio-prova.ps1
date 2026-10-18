<#
.SYNOPSIS
    Crea un file WAV di prova in italiano usando la sintesi vocale di Windows.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\crea-audio-prova.ps1 -Output prova.wav
#>
param(
    [string]$Output = "prova.wav",
    [string]$Text = "Buongiorno a tutti. Questa è una prova del programma Trascrittore. Il testo viene riconosciuto in locale, senza inviare dati in rete. Alla fine la trascrizione può essere esportata in formato Word, PDF, HTML oppure testo semplice."
)

Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$italian = $synth.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -like "it*" } | Select-Object -First 1
if ($italian) {
    $synth.SelectVoice($italian.VoiceInfo.Name)
    Write-Output ("Voce: " + $italian.VoiceInfo.Name)
} else {
    Write-Warning "Nessuna voce italiana installata: uso la voce predefinita."
}
$fullPath = [System.IO.Path]::GetFullPath($Output)
$synth.SetOutputToWaveFile($fullPath)
$synth.Speak($Text)
$synth.Dispose()
Write-Output ("Creato: " + $fullPath)
