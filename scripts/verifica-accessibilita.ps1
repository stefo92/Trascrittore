<#
.SYNOPSIS
    Verifica l'accessibilità della finestra di Trascrittore così come la vedono gli screen reader.

.DESCRIPTION
    Avvia trascrittore.exe e interroga ogni controllo tramite MSAA (IAccessible), l'interfaccia
    usata da NVDA e JAWS per i controlli Win32 standard. Per ogni controllo stampa ruolo, nome,
    valore, tasto di scelta rapida, descrizione e stato. Esce con codice 1 se un controllo
    interattivo non ha nome.

    Con -AudioFile esegue anche una prova funzionale: apre il file, preme «Avvia trascrizione»,
    attende il testo e controlla che il focus sia passato al campo «Testo trascritto».

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\verifica-accessibilita.ps1 -Exe target\debug\trascrittore.exe
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\verifica-accessibilita.ps1 -AudioFile prova.wav
#>
param(
    [string]$Exe = "target\debug\trascrittore.exe",
    [string]$AudioFile = "",
    [int]$TimeoutSeconds = 300
)

Add-Type -ReferencedAssemblies Accessibility -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using Accessibility;

public class ControlInfo {
    public IntPtr Hwnd; public string Class; public int Role; public string Name;
    public string Value; public string Shortcut; public string Description; public int State;
}

public static class Msaa {
    [DllImport("oleacc.dll")]
    static extern int AccessibleObjectFromWindow(IntPtr hwnd, uint id, ref Guid iid,
        [MarshalAs(UnmanagedType.Interface)] out object obj);
    delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc proc, IntPtr lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern IntPtr PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);

    public static List<ControlInfo> Controls(IntPtr top) {
        var list = new List<ControlInfo>();
        EnumChildWindows(top, (h, l) => {
            var cls = new StringBuilder(64);
            GetClassName(h, cls, 64);
            var info = new ControlInfo { Hwnd = h, Class = cls.ToString() };
            Guid iid = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
            object obj;
            if (AccessibleObjectFromWindow(h, 0xFFFFFFFC, ref iid, out obj) == 0) {
                var a = obj as IAccessible;
                if (a != null) {
                    try { info.Name = a.get_accName(0); } catch {}
                    try { info.Value = a.get_accValue(0); } catch {}
                    try { info.Shortcut = a.get_accKeyboardShortcut(0); } catch {}
                    try { info.Description = a.get_accDescription(0); } catch {}
                    try { info.Role = Convert.ToInt32(a.get_accRole(0)); } catch {}
                    try { info.State = Convert.ToInt32(a.get_accState(0)); } catch {}
                }
            }
            list.Add(info);
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
"@

$roles = @{ 41 = "testo"; 42 = "modifica"; 43 = "pulsante"; 44 = "casella di controllo";
            46 = "casella combinata"; 48 = "barra di avanzamento" }
$interactiveRoles = @(42, 43, 44, 46, 48)

function Describe-State([int]$state) {
    $flags = @()
    if ($state -band 0x1) { $flags += "non disponibile" }
    if ($state -band 0x4) { $flags += "focus" }
    if ($state -band 0x10) { $flags += "selezionato" }
    if ($state -band 0x40) { $flags += "sola lettura" }
    if ($state -band 0x100000) { $flags += "focalizzabile" }
    return ($flags -join ", ")
}

function Wait-Window($process) {
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 300
        $process.Refresh()
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { return $process.MainWindowHandle }
    }
    return [IntPtr]::Zero
}

$arguments = @()
if ($AudioFile) { $arguments += ('"' + [System.IO.Path]::GetFullPath($AudioFile) + '"') }
$process = if ($arguments.Count) { Start-Process -FilePath $Exe -ArgumentList $arguments -PassThru } else { Start-Process -FilePath $Exe -PassThru }
$window = Wait-Window $process
if ($window -eq [IntPtr]::Zero) {
    Write-Error "Finestra di Trascrittore non trovata."
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    exit 2
}

$problems = 0
$controls = [Msaa]::Controls($window)
foreach ($c in $controls) {
    $role = if ($roles.ContainsKey($c.Role)) { $roles[$c.Role] } else { "ruolo " + $c.Role }
    $line = "{0,-22} '{1}'" -f $role, $c.Name
    if ($c.Value) { $line += "  valore: '" + $c.Value + "'" }
    if ($c.Shortcut) { $line += "  tasto: " + $c.Shortcut }
    if ($c.Description) { $line += "  descrizione: " + $c.Description }
    $state = Describe-State $c.State
    if ($state) { $line += "  [" + $state + "]" }
    if ($interactiveRoles -contains $c.Role -and [string]::IsNullOrWhiteSpace($c.Name)) {
        $line = "PROBLEMA, senza nome: " + $line
        $problems++
    }
    Write-Output $line
}
Write-Output ""

if ($AudioFile) {
    $start = $controls | Where-Object { $_.Role -eq 43 -and $_.Name -eq "Avvia trascrizione" } | Select-Object -First 1
    $text = $controls | Where-Object { $_.Role -eq 42 -and $_.Name -eq "Testo trascritto" } | Select-Object -First 1
    $BM_CLICK = 0x00F5
    [void][Msaa]::PostMessage($start.Hwnd, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
    Write-Output "Premuto «Avvia trascrizione», attendo il risultato…"
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $result = $null
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 1
        $current = [Msaa]::Controls($window)
        $field = $current | Where-Object { $_.Hwnd -eq $text.Hwnd }
        if ($field.Value) { $result = $field; break }
    }
    if ($result) {
        $status = ($current | Where-Object { $_.Role -eq 41 } | Where-Object { $_.Name -like "Trascrizione*" } | Select-Object -First 1).Name
        Write-Output ("Stato: " + $status)
        Write-Output ("Focus sul testo trascritto: " + [bool]($result.State -band 0x4))
        Write-Output ("Testo: " + $result.Value)
    } else {
        Write-Output "PROBLEMA: nessun testo trascritto entro $TimeoutSeconds secondi."
        $problems++
    }
}

Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
if ($problems -gt 0) {
    Write-Output "$problems problemi trovati."
    exit 1
}
Write-Output "Nessun problema: tutti i controlli interattivi hanno nome e ruolo."
