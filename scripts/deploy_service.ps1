#Requires -Version 5.1
<#
.SYNOPSIS
    Installs the Rust build of The Entity In The Shed as a Windows service named
    entity_in_the_shed. It shares port 5005 with the Python service
    entity-in-the-shed, so only one of them can run at a time.

.DESCRIPTION
    The binary is a console server. It serves HTTP but never calls
    StartServiceCtrlDispatcher, so the Service Control Manager cannot host it
    directly: `sc create` would register it and then every start would fail with
    error 1053 ("did not respond in a timely fashion"). The same is true of the
    .bat wrapper the previous version of this script installed - the SCM calls
    CreateProcess on the ImagePath, which cannot execute a batch file at all.

    So the binary is wrapped by NSSM, the supervisor already running this
    machine's llama-server and ai-monitor services. NSSM supplies what the SCM
    does not: a working directory, captured stdout/stderr, and restart-on-exit.

    The two implementations follow plan decision D9 for state isolation: a
    distinct service name and a distinct STORAGE_ROOT. They share the HTTP port
    (5005) instead of running side by side, so stop the Python service
    entity-in-the-shed before starting this one. The installer refuses to
    proceed while another process owns the port, so a clash is reported rather
    than becoming a bind failure at boot.

.PARAMETER DryRun
    Validate paths, the port and the command lines, print what would be done,
    change nothing. Does not need elevation.

.EXAMPLE
    # see the plan without touching the machine
    .\deploy_service.ps1 -DryRun

.EXAMPLE
    # install, start and health-check (elevated)
    powershell -ExecutionPolicy Bypass -File .\scripts\deploy_service.ps1

.EXAMPLE
    # remove the service again
    .\scripts\deploy_service.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [string] $ServiceName = 'entity_in_the_shed',
    [string] $DisplayName = 'Entity In The Shed (Rust)',
    [string] $Description = 'The Entity In The Shed Rust HTTP server (console binary hosted by NSSM).',
    [string] $ExePath = 'D:\Entity_In_The_Shed\target\release\entity_in_the_shed.exe',
    [string] $AppDirectory = 'D:\Entity_In_The_Shed',
    [string] $NssmPath = 'D:\Prod\llama-service\bin\nssm.exe',
    [int]    $Port = 5005,
    [string] $StorageRoot = 'C:\Temp\entity_in_the_shed',
    # The Python app points this at the local llama-server router too
    # (D:\Entity-In-The-Shed\.env); the binary's own default is LM Studio on :1234,
    # which this machine does not run.
    [string] $EngineHost = 'http://127.0.0.1:8080',
    [int]    $StartupTimeoutSeconds = 60,
    [switch] $NoStart,
    [switch] $Uninstall,
    [switch] $DryRun
)

$ErrorActionPreference = 'Stop'

$logDir = Join-Path $StorageRoot 'Logs'
$installLog = Join-Path $logDir 'install.log'

function Resolve-Nssm {
    param([string] $Preferred)

    if ($Preferred -and (Test-Path -LiteralPath $Preferred)) { return (Resolve-Path -LiteralPath $Preferred).Path }

    $onPath = (Get-Command nssm.exe -ErrorAction SilentlyContinue).Source
    if ($onPath) { return $onPath }

    foreach ($candidate in @(
        'D:\Prod\llama-service\bin\nssm.exe',
        'C:\Program Files\nssm\win64\nssm.exe',
        'C:\tools\nssm\nssm.exe'
    )) {
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    return $null
}

function Assert-PortFree {
    param([int] $Number, [string] $OwnExe)

    $ownName = if ($OwnExe) { [System.IO.Path]::GetFileName($OwnExe) } else { $null }
    $listeners = Get-NetTCPConnection -State Listen -LocalPort $Number -ErrorAction SilentlyContinue
    foreach ($listener in $listeners) {
        $owner = Get-CimInstance Win32_Process -Filter "ProcessId=$($listener.OwningProcess)" -ErrorAction SilentlyContinue
        $where = if ($owner) { "$($owner.Name) (pid $($owner.ProcessId))" } else { "pid $($listener.OwningProcess)" }

        # A dry run against a service that is already installed and serving sees
        # the service's own process here; that is not a conflict.
        if ($ownName -and $owner -and $owner.Name -eq $ownName) {
            Write-Host "Port $Number is held by this service's own process ($where)."
            continue
        }
        throw "Port $Number is already in use by $where. Pick another -Port, or stop that process first."
    }
}

function Remove-ServiceIfPresent {
    param([string] $Name, [string] $Nssm)

    $existing = Get-Service -Name $Name -ErrorAction SilentlyContinue
    if (-not $existing) { return }

    Write-Host "Removing the existing '$Name' service before reinstalling."
    if ($existing.Status -ne 'Stopped') {
        try { Stop-Service -Name $Name -Force -ErrorAction Stop } catch { Write-Warning $_ }
    }

    & $Nssm remove $Name confirm | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "nssm remove $Name failed ($LASTEXITCODE)." }

    # A service that was running is deleted asynchronously; recreating it too soon
    # fails with 1072 "marked for deletion".
    foreach ($attempt in 1..20) {
        if (-not (Get-Service -Name $Name -ErrorAction SilentlyContinue)) { return }
        Start-Sleep -Milliseconds 500
    }
    throw "Service '$Name' is still registered after removal; it is probably marked for deletion. Close any open handle (services.msc) and retry."
}

# ---------------------------------------------------------------------------
# Validate everything before changing anything.
# ---------------------------------------------------------------------------

$nssm = Resolve-Nssm -Preferred $NssmPath
if (-not $nssm) { throw 'nssm.exe not found. Pass -NssmPath, or put NSSM on PATH.' }

if (-not (Test-Path -LiteralPath $LogDir)) { New-Item -ItemType Directory -Path $LogDir -Force | Out-Null }
if (-not (Test-Path -LiteralPath $StorageRoot)) { New-Item -ItemType Directory -Path $StorageRoot -Force | Out-Null }

$stdoutLog = Join-Path $logDir 'service.stdout.log'
$stderrLog = Join-Path $logDir 'service.stderr.log'
$healthUrl = "http://127.0.0.1:$Port/health"
$pageUrl = "http://127.0.0.1:$Port/"

# The binary resolves templates/, styles/, static/, characters/ and the .env
# against its own folder unless --base-dir names one. This build lives in
# target\release (or target\debug for a dev run), while those files live in the
# repository, so the service must hand it the repository path. Without this the
# service answers /health but every page request is a 500 -- the UI simply does
# not display.
$serviceParameters = "--base-dir `"$AppDirectory`""

# AppEnvironmentExtra replaces the service's whole extra environment block, so
# every name the service needs belongs in this one list: re-running the
# installer with a narrower list silently unsets the others.
$serviceEnv = @("APP_PORT=$Port", "STORAGE_ROOT=$StorageRoot")
if ($EngineHost) { $serviceEnv += "LM_ENGINE_HOST=$EngineHost" }

$plan = @(
    "$nssm install $ServiceName `"$ExePath`""
    "$nssm set $ServiceName AppDirectory `"$AppDirectory`""
    "$nssm set $ServiceName AppParameters $serviceParameters"
    "$nssm set $ServiceName AppStdout `"$stdoutLog`""
    "$nssm set $ServiceName AppStderr `"$stderrLog`""
    "$nssm set $ServiceName AppEnvironmentExtra $($serviceEnv -join ' ')"
    "$nssm set $ServiceName Start SERVICE_AUTO_START"
    "$nssm set $ServiceName AppExit Default Restart"
    "Start-Service $ServiceName"
    "GET $healthUrl"
    "GET $pageUrl (the UI)"
)

if ($DryRun) {
    "service name   : $ServiceName"
    "binary         : $ExePath  (exists: $(Test-Path -LiteralPath $ExePath))"
    "working dir    : $AppDirectory"
    "supervisor     : $nssm"
    "port           : $Port"
    "storage root   : $StorageRoot"
    "engine host    : $EngineHost"
    ""
    "plan:"
    $plan | ForEach-Object { "  $_" }
    if (-not (Test-Path -LiteralPath $ExePath)) { throw "Executable not found: $ExePath. Build it with: cargo build --release" }
    Assert-PortFree -Number $Port -OwnExe $ExePath
    ""
    "Port $Port checked; dry run only, nothing was changed."
    return
}

# ---------------------------------------------------------------------------
# Elevation is required for every write below: the SCM database, and the
# service's own registry key. Fail loudly rather than half-installing.
# ---------------------------------------------------------------------------

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Administrator rights are required to install a service. Right-click PowerShell 'Run as administrator', then: powershell -ExecutionPolicy Bypass -File `"$PSCommandPath`""
}

Start-Transcript -Path $installLog -Append | Out-Null

try {
    if (-not (Test-Path -LiteralPath $ExePath)) {
        throw "Executable not found: $ExePath. Build it with: cargo build --release"
    }

    Remove-ServiceIfPresent -Name $ServiceName -Nssm $nssm

    if ($Uninstall) {
        Write-Host "Service '$ServiceName' removed."
        return
    }

    Assert-PortFree -Number $Port -OwnExe $ExePath

    Write-Host "Installing service '$ServiceName' -> $ExePath"
    & $nssm install $ServiceName "$ExePath"
    if ($LASTEXITCODE -ne 0) { throw "nssm install failed ($LASTEXITCODE)." }

    & $nssm set $ServiceName DisplayName "$DisplayName"
    & $nssm set $ServiceName Description "$Description"
    & $nssm set $ServiceName AppDirectory "$AppDirectory"
    & $nssm set $ServiceName AppParameters $serviceParameters
    & $nssm set $ServiceName AppStdout "$stdoutLog"
    & $nssm set $ServiceName AppStderr "$stderrLog"
    & $nssm set $ServiceName AppRotateFiles 1
    & $nssm set $ServiceName AppRotateOnline 1
    & $nssm set $ServiceName AppRotateBytes 10485760
    & $nssm set $ServiceName AppEnvironmentExtra @serviceEnv
    & $nssm set $ServiceName Start SERVICE_AUTO_START
    & $nssm set $ServiceName AppExit Default Restart
    if ($LASTEXITCODE -ne 0) { throw "nssm configuration failed ($LASTEXITCODE)." }

    # A registered service is not an installed one: prove the SCM knows it.
    & sc.exe query $ServiceName | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "The SCM does not know '$ServiceName' after install (sc query exit $LASTEXITCODE)." }

    $service = Get-Service -Name $ServiceName
    Write-Host "Registered: $($service.Name) [$($service.Status)]"

    if ($NoStart) {
        Write-Host "Installed but not started (-NoStart). Start it with: Start-Service $ServiceName"
        return
    }

    Write-Host "Starting '$ServiceName'..."
    Start-Service -Name $ServiceName

    Write-Host "Waiting for $healthUrl ..."
    $healthy = $false
    foreach ($attempt in 1..($StartupTimeoutSeconds * 2)) {
        Start-Sleep -Milliseconds 500
        try {
            Invoke-WebRequest -Uri $healthUrl -TimeoutSec 2 -UseBasicParsing | Out-Null
            $healthy = $true
            break
        }
        catch { }
    }

    $service = Get-Service -Name $ServiceName
    Write-Host "Waiting for $pageUrl ..."
    $uiHealthy = $false
    foreach ($attempt in 1..($StartupTimeoutSeconds * 2)) {
        try {
            $page = Invoke-WebRequest -Uri $pageUrl -TimeoutSec 5 -UseBasicParsing
            if ($page.StatusCode -eq 200 -and $page.Content -match 'The Entity in the Shed') {
                $uiHealthy = $true
                break
            }
        }
        catch { }
        Start-Sleep -Milliseconds 500
    }

    $service = Get-Service -Name $ServiceName
    if ($healthy -and $uiHealthy) {
        Write-Host "Service '$ServiceName' is $($service.Status) and answering $healthUrl and $pageUrl"
    }
    elseif ($healthy) {
        # Health answers without the page's files: the binary is serving, but
        # / is a 500 and the browser shows "internal server error". This is the
        # failure a missing --base-dir produces, so name it rather than wait.
        Write-Warning "Service '$ServiceName' answers $healthUrl but the UI at $pageUrl did not become ready after $StartupTimeoutSeconds s."
        Write-Warning "Its AppParameters must pass --base-dir `"$AppDirectory`" so the page, styles, helpers and characters resolve."
        Write-Warning "Logs: $stdoutLog / $stderrLog"
        & $nssm dump $ServiceName
        throw "Install aborted: the service does not serve the UI at $pageUrl."
    }
    else {
        # A stopped service plus a log path is a diagnosis; a success message is not.
        Write-Warning "Service '$ServiceName' is $($service.Status) but $healthUrl has not answered after $StartupTimeoutSeconds s."
        Write-Warning "Logs: $stdoutLog / $stderrLog"
        & $nssm dump $ServiceName
        throw "Install aborted: the service does not serve $healthUrl."
    }

    Write-Host ""
    Write-Host "Installed '$ServiceName' (port $Port, storage $StorageRoot)."
    Write-Host "  sc.exe query $ServiceName"
    Write-Host "  net start $ServiceName / net stop $ServiceName"
    Write-Host "  $installLog"
}
finally {
    Stop-Transcript | Out-Null
}
