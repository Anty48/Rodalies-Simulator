# Recogida AUTOMÁTICA de datos reales (GTFS-RT de Renfe) en el Programador de tareas de Windows.
#
#   install    crea la tarea «Rod4lia - Recolector GTFS-RT Renfe» (al iniciar sesión + cada día a las
#              04:30 por si se cerró) y la arranca ya. Sin consola (pythonw), sin permisos de admin.
#   uninstall  borra la tarea (no para un recolector que ya esté en marcha: eso es /api/gtfsrt/stop).
#   status     JSON con si existe la tarea y su estado.
#
# Uso: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\gtfsrt_autostart.ps1 -Action install
param([ValidateSet("install", "uninstall", "status")][string]$Action = "status")

$ErrorActionPreference = "Stop"
$TaskName = "Rod4lia - Recolector GTFS-RT Renfe"
$Root = Split-Path -Parent $PSScriptRoot
$Script = Join-Path $Root "scripts\collect_gtfsrt.py"

function Out-Json($o) { $o | ConvertTo-Json -Compress }

function Find-Pythonw {
    foreach ($name in @("python", "py", "python3")) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) {
            # Ruta real del intérprete (py.exe es un lanzador): se pregunta al propio Python.
            $exe = & $cmd.Source -c "import sys; print(sys.executable)" 2>$null
            if ($exe) {
                $w = Join-Path (Split-Path -Parent $exe.Trim()) "pythonw.exe"
                if (Test-Path $w) { return $w }
            }
        }
    }
    return $null
}

try {
    switch ($Action) {
        "install" {
            $pyw = Find-Pythonw
            if (-not $pyw) { Out-Json @{ ok = $false; error = "No se encuentra pythonw.exe (instala Python 3 desde python.org)." }; exit 0 }
            $act = New-ScheduledTaskAction -Execute $pyw -Argument "`"$Script`" --every 20" -WorkingDirectory $Root
            $trg = @(
                (New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME),
                (New-ScheduledTaskTrigger -Daily -At "04:30")
            )
            # Sin límite de duración (por defecto Windows corta a las 72 h), reintentos si se cae,
            # también con batería, y nunca dos a la vez (además el propio recolector es instancia única).
            $set = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 `
                -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable -AllowStartIfOnBatteries `
                -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew
            Register-ScheduledTask -TaskName $TaskName -Action $act -Trigger $trg -Settings $set `
                -Description "Graba posiciones, retrasos, cancelaciones y avisos de Rodalies (GTFS-RT de Renfe) en raw\gtfsrt para comparar el simulador con la realidad." -Force | Out-Null
            Start-ScheduledTask -TaskName $TaskName
            Out-Json @{ ok = $true; task = $TaskName; python = $pyw }
        }
        "uninstall" {
            if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
                Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
            }
            Out-Json @{ ok = $true }
        }
        "status" {
            $t = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
            if ($t) { Out-Json @{ installed = $true; state = "$($t.State)"; task = $TaskName } }
            else { Out-Json @{ installed = $false; task = $TaskName } }
        }
    }
} catch {
    Out-Json @{ ok = $false; error = $_.Exception.Message }
}
