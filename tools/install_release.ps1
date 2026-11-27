<# Install only while Hex is idle, then require complete post-install evidence. #>
[CmdletBinding(DefaultParameterSetName = 'Release')]
param(
    [Parameter(Mandatory)][string]$Feed,
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?$')]
    [string]$Version,
    [Parameter(Mandatory)][string]$Log,
    [Parameter(ParameterSetName = 'Release')][switch]$WhatIf,
    [Parameter(Mandatory, ParameterSetName = 'Proof')][string]$ProofInstallDirectory
)
$ErrorActionPreference = 'Stop'
$stage = 'preflight'
function Quote-Argument([string]$Value) {
    return '"' + ([regex]::Replace($Value, '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
}
function Run-Hidden([string]$Exe, [string[]]$Arguments, [scriptblock]$Started = {}, [int]$Timeout = -1) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Exe
    $info.Arguments = (@($Arguments | ForEach-Object { Quote-Argument $_ }) -join ' ')
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $child = [Diagnostics.Process]::Start($info)
    try {
        & $Started
        # Drain both streams without exposing paths, environment values or hook errors.
        $stdout = $child.StandardOutput.ReadToEndAsync()
        $stderr = $child.StandardError.ReadToEndAsync()
        if (-not $child.WaitForExit($Timeout)) {
            $child.Kill($true)
            $child.WaitForExit()
            throw 'Setup timed out'
        }
        [void]$stdout.GetAwaiter().GetResult()
        [void]$stderr.GetAwaiter().GetResult()
        if ($child.ExitCode -ne 0) { throw 'Child failed' }
    } finally { $child.Dispose() }
}
function Get-InstallPaths {
    $local = [Environment]::GetFolderPath('LocalApplicationData')
    $data = [Environment]::GetFolderPath('ApplicationData')
    $state = $null
    foreach ($name in @('SIDENOTE_DATA_DIR','SIDENOTE_STATE_DIR')) {
        $state = [Environment]::GetEnvironmentVariable($name)
        if (-not $state) { $state = [Environment]::GetEnvironmentVariable($name.Replace('SIDENOTE_', 'HEX_MEETINGS_')) }
        if ($state) { break }
    }
    if (-not $state) {
        $state = Join-Path $data 'sidenote'
        if (-not (Test-Path -LiteralPath $state)) { $state = Join-Path $data 'hex-meetings' }
    }
    return @{ State = $state; Updates = (Join-Path $local 'Sidenote/updates'); Exe = (Join-Path $local 'sidenote/current/sidenote.exe') }
}
function Assert-HexIdle {
    $raw = & hex.com status --json 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Status failed' }
    $status = ($raw -join "`n") | ConvertFrom-Json
    if ($status.meeting.state -cne 'idle' -or
        ($status.capture.pending_jobs -isnot [int] -and $status.capture.pending_jobs -isnot [long]) -or
        $status.capture.pending_jobs -ne 0) { throw 'Hex is busy or status is incomplete' }
}
function Assert-SidenoteIdle($Paths) {
    $router = Join-Path $Paths.State 'local/router-queue'
    if ((Test-Path -LiteralPath $router) -and @(Get-ChildItem -LiteralPath $router -File -Filter '*.json').Count) { throw 'Routing is pending' }
    $notes = Join-Path $Paths.State 'local/notes-jobs'
    if (-not (Test-Path -LiteralPath $notes)) { return }
    $terminal = @('done','no_helper','needs_click','helper_missing','helper_unavailable_on_linux',
        'HELPER_LIMIT','HELPER_AUTH','timed_out','bad_output','too_long','context_too_long','empty','backend_error')
    foreach ($file in @(Get-ChildItem -LiteralPath $notes -File -Filter '*.json')) {
        if ($file.Length -gt 2MB) { throw 'Notes job is unreadable' }
        $job = [IO.File]::ReadAllText($file.FullName) | ConvertFrom-Json
        if ($job.state -cnotin $terminal -or $job.sorting_pending -eq $true -or
            ($job.state -ceq 'backend_error' -and $null -ne $job.next_try_ms)) { throw 'Notes are active or pending' }
    }
}
function Open-InstallLock([string]$Path) {
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path)) | Out-Null
    return [IO.File]::Open($Path, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
}
function Release-InstallLocks($Locks) {
    foreach ($lock in $Locks) { $lock.Dispose() }
    $Locks.Clear()
}
function Write-NewMarker([string]$Path, [string]$Text) {
    $file = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
        $file.Write($bytes, 0, $bytes.Length)
        $file.Flush($true)
    } catch {
        $file.Dispose()
        Remove-Item -LiteralPath $Path -Force
        throw
    } finally { $file.Dispose() }
}
function Wait-ListenerStopped([string]$Path) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        try { $probe = Open-InstallLock $Path; $probe.Dispose(); return }
        catch { if ($watch.Elapsed.TotalSeconds -ge 8) { throw 'Listener did not stop' } }
        Start-Sleep -Milliseconds 100
    }
}
function Test-ListenerRunning([string]$Exe) {
    return @(Get-CimInstance Win32_Process -Filter "Name = 'sidenote.exe'" | Where-Object {
        $_.ExecutablePath -ieq $Exe -and $_.CommandLine -match '(?i)(?:^|\s)"?--listen"?(?=\s|$)'
    }).Count -eq 1
}
function Invoke-InstallCheck([string]$Expected) {
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'install_check.ps1') -Expect $Expected
    if ($LASTEXITCODE -ne 0) { throw 'Install check failed' }
}
function Get-SetupPath {
    $channel = if ($Version -match '\.main\.\d+$') { 'latest' } elseif ($Version -match '-dev\.') { 'dev' } else { 'stable' }
    $setup = Join-Path $Feed "sidenote-$channel-Setup.exe"
    if (-not $ProofInstallDirectory -and -not (Test-Path -LiteralPath $setup -PathType Leaf)) {
        return Join-Path $Feed 'sidenote-win-Setup.exe'
    }
    return $setup
}
function Invoke-Setup([string]$Path, [string]$LogPath, [scriptblock]$Started = {}) {
    $arguments = @('--silent', '--log', $LogPath)
    $timeout = -1
    if ($ProofInstallDirectory) {
        $arguments += @('--installto', $ProofInstallDirectory)
        $timeout = 120000
    }
    Run-Hidden $Path $arguments $Started $timeout
}
function Invoke-ReleaseInstall {
    $setup = Get-SetupPath
    if ($WhatIf) {
        Write-Output '1. Require Hex idle with no capture jobs; reserve the existing update activity, window and worker locks; refuse pending notes/routing.'
        Write-Output '2. Request listener stop with its existing stop/resume markers; wait up to 8 seconds. Re-check Hex and Sidenote immediately before Setup.'
        Write-Output "3. Run $(Split-Path -Leaf $setup) --silent --log <Log>; release reservations only after Setup starts; wait and require exit 0."
        Write-Output "4. Run installed sidenote.exe --veloapp-updated $Version; require success, then verify the listener within 30 seconds."
        Write-Output "5. Run tools/install_check.ps1 -Expect $Version; require every row to pass. Hex settings stay as they are; startup folder discovery catches recordings from the install window."
        return
    }
    $locks = [Collections.Generic.List[IDisposable]]::new()
    $stopOwned = $false
    $resumeCreated = $false
    $handoff = @{ Started = $false }
    try {
        $script:stage = 'Hex idle check: require readable idle status and no capture jobs'
        Assert-HexIdle
        $paths = Get-InstallPaths
        $script:stage = 'Sidenote idle check: close its window and finish notes, routing, Ask and update work'
        foreach ($path in @((Join-Path $paths.Updates 'update-activity.lock'),
            (Join-Path $paths.Updates 'update-window.lock'), (Join-Path $paths.Updates 'update-check.lock'),
            (Join-Path $paths.State 'local/notes-jobs/worker.lock'), (Join-Path $paths.State 'local/meeting-router.lock'),
            (Join-Path $paths.State 'local/notes-script.lock'))) { $locks.Add((Open-InstallLock $path)) }
        Assert-SidenoteIdle $paths
        $script:stage = 'listener handoff: another install must not own its stop marker; listener must stop within 8 seconds'
        $stop = Join-Path $paths.Updates 'update-listener.stop'
        $resume = Join-Path $paths.Updates 'update-listener.resume'
        Write-NewMarker $stop (@{pid=$PID; created=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()} | ConvertTo-Json -Compress)
        $stopOwned = $true
        if (-not (Test-Path -LiteralPath $resume)) { Write-NewMarker $resume 'resume'; $resumeCreated = $true }
        Wait-ListenerStopped (Join-Path $paths.Updates 'update-listener.lock')
        $logPath = [IO.Path]::GetFullPath($Log)
        $script:stage = 'silent Setup: require the feed installer'
        if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) { throw 'Installer missing' }
        $script:stage = 'final Hex idle check immediately before Setup'
        Assert-HexIdle
        $script:stage = 'final Sidenote idle check immediately before Setup'
        Assert-SidenoteIdle $paths
        $script:stage = 'silent Setup: require exit 0; inspect the requested log on failure'
        Invoke-Setup $setup $logPath {
            $handoff.Started = $true
            Release-InstallLocks $locks
        }
        $script:stage = 'after-update hook: require the installed app and exit 0'
        if (-not (Test-Path -LiteralPath $paths.Exe -PathType Leaf)) { throw 'Installed app missing' }
        Run-Hidden $paths.Exe @('--veloapp-updated', $Version)
        $script:stage = 'listener start: require the installed sidenote.exe --listen within 30 seconds'
        Start-ScheduledTask -TaskPath '\' -TaskName 'Sidenote meeting listener' *> $null
        $watch = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-ListenerRunning $paths.Exe)) {
            if ($watch.Elapsed.TotalSeconds -ge 30) { throw 'Listener did not start' }
            Start-Sleep -Milliseconds 250
        }
        $script:stage = 'install_check: every row must pass'
        Invoke-InstallCheck $Version
        Write-Output 'Install checks passed.'
    } finally {
        # Reverse a pre-Setup refusal: release only our marker/locks and resume the old listener.
        $restore = $false
        if ($stopOwned -and (Test-Path -LiteralPath $stop)) {
            try {
                $marker = [IO.File]::ReadAllText($stop) | ConvertFrom-Json
                if ($marker.pid -eq $PID) { Remove-Item -LiteralPath $stop; $restore = $true }
            } catch { }
        }
        Release-InstallLocks $locks
        if ($restore) {
            if ($resumeCreated -and -not $handoff.Started) { Remove-Item -LiteralPath $resume -ErrorAction SilentlyContinue }
            try { Start-ScheduledTask -TaskPath '\' -TaskName 'Sidenote meeting listener' *> $null } catch { }
        }
    }
}
if ($MyInvocation.InvocationName -eq '.') { return }
try {
    if ($ProofInstallDirectory) {
        $stage = 'isolated hosted proof preflight'
        if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
            -not [IO.Path]::IsPathFullyQualified($ProofInstallDirectory) -or
            (Test-Path -LiteralPath $ProofInstallDirectory)) {
            throw 'Proof Setup requires a fresh absolute install directory on a GitHub-hosted runner.'
        }
        $stage = 'isolated silent Setup'
        Invoke-Setup (Get-SetupPath) ([IO.Path]::GetFullPath($Log))
    } else { Invoke-ReleaseInstall }
    exit 0
} catch { [Console]::Error.WriteLine("install_release: failed at $stage."); exit 1 }
