<#
Read-only post-install evidence. Run with -Expect <version> [-Before].
Do not claim "installed and verified" without pasting the complete after table.
No application is launched: sq.version supplies the installer's recorded version.
The embedded task checker runs in this PowerShell process, with repairs disabled;
normal completion/terminating error maps to exit code 0/1, just as powershell -File.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?$')]
    [string]$Expect,
    [switch]$Before
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'install_recovery_check.ps1')
$rows = [Collections.Generic.List[object]]::new()
function Add-Check([string]$Name, [bool]$Pass, [string]$Observed) {
    $rows.Add([pscustomobject]@{ Check = $Name; Result = $(if ($Pass) { 'PASS' } else { 'FAIL' }); Observed = $Observed })
}
function Safe-Version($Value) {
    if ($Value -is [string] -and $Value -cmatch '^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?$') { return $Value }
    return 'missing or invalid'
}
function Measure-IdleCpu([string]$Exe) {
    $minUptime = 300
    $sampleSeconds = 60
    $entries = @()
    try {
        foreach ($item in @(Get-CimInstance Win32_Process -Filter "Name = 'sidenote.exe'")) {
            if (-not $item.ExecutablePath -or $item.ExecutablePath -ine $Exe) { continue }
            $kind = if ($item.CommandLine -match '(?i)(?:^|\s)"?--listen"?(?=\s|$)') { 'recorder' }
                elseif ($item.CommandLine -match '(?i)\.exe"?\s*$') { 'window' } else { continue }
            $process = Get-Process -Id $item.ProcessId -ErrorAction Stop
            $entries += [pscustomobject]@{ Kind = $kind; Process = $process; Start = $process.StartTime }
        }
        $now = [DateTime]::Now
        $report = @{}
        $eligible = @()
        foreach ($entry in $entries) {
            $uptime = ($now - $entry.Start).TotalSeconds
            if ($uptime -lt $minUptime) {
                $report[$entry] = "idle: too soon after start ($([int]$uptime) s)"
                continue
            }
            $entry.Process.Refresh()
            $entry | Add-Member -NotePropertyName Uptime -NotePropertyValue $uptime
            $entry | Add-Member -NotePropertyName CpuStart -NotePropertyValue $entry.Process.TotalProcessorTime.TotalSeconds
            $eligible += $entry
        }
        $clock = [Diagnostics.Stopwatch]::StartNew()
        if ($eligible.Count) { Start-Sleep -Seconds $sampleSeconds }
        foreach ($entry in $eligible) {
            try {
                $entry.Process.Refresh()
                if ($entry.Process.HasExited -or $entry.Process.StartTime -ne $entry.Start) { $report[$entry] = 'unavailable'; continue }
                $cpuEnd = $entry.Process.TotalProcessorTime.TotalSeconds
                $overUptime = 100 * $cpuEnd / (($entry.Uptime) + $clock.Elapsed.TotalSeconds)
                $sampled = 100 * ($cpuEnd - $entry.CpuStart) / $clock.Elapsed.TotalSeconds
                $report[$entry] = 'since start {0:F2}% over {1} s uptime; {2} s sample {3:F2}%' -f $overUptime, [int]($entry.Uptime + $clock.Elapsed.TotalSeconds), $sampleSeconds, $sampled
            } catch { $report[$entry] = 'unavailable' }
        }
        $parts = foreach ($kind in @('window', 'recorder')) {
            $kindEntries = @($entries | Where-Object Kind -eq $kind)
            if ($kindEntries.Count) { "$kind=$((@($kindEntries | ForEach-Object { $report[$_] })) -join ',')" }
            else { "$kind=unavailable" }
        }
        $load = @(Get-CimInstance Win32_Processor | ForEach-Object { $_.LoadPercentage } | Where-Object { $null -ne $_ })
        $loadText = if ($load.Count) { 'system load {0:F0}%' -f ($load | Measure-Object -Average).Average } else { 'system load unavailable' }
        $measured = $eligible.Count -gt 0
        return "% of one core; $($parts -join '; '); $loadText; $(if ($measured) { 'measured' } else { 'not measured' }); activity not checked"
    } catch { return 'idle CPU unavailable, not measured (informational only)' }
}
function Invoke-ListenerCheck([string]$Exe) {
    $saved = @{}
    $flags = @{ SIDENOTE_TASK_NAME = 'Sidenote meeting listener'; SIDENOTE_TASK_EXE = $Exe
        SIDENOTE_TASK_ARGS = '--listen'; SIDENOTE_TASK_RUN = 'no'; SIDENOTE_TASK_REGISTER = 'no' }
    try {
        $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '../src/updates_windows.rs'))
        $match = [regex]::Match($source, '(?s)const LISTENER_TASK_SCRIPT: &str = r#"(.*?)"#;')
        if (-not $match.Success) { throw 'Embedded checker unavailable' }
        foreach ($key in $flags.Keys) {
            $saved[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
            [Environment]::SetEnvironmentVariable($key, $flags[$key], 'Process')
        }
        # Discard every stream: task errors may contain private paths or account names.
        & ([scriptblock]::Create($match.Groups[1].Value)) *> $null
        return 0
    } catch { return 1 }
    finally {
        foreach ($key in $saved.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key], 'Process') }
    }
}

# Only these two folder locations are consulted; private settings are never loaded.
$app = $null
$scripts = $null
$local = [Environment]::GetFolderPath('LocalApplicationData')
$profile = [Environment]::GetFolderPath('UserProfile')
if ($local) { $app = Join-Path $local 'Sidenote/current' }
if ($profile) { $scripts = Join-Path $profile '.config/hex/scripts' }
$exe = if ($app) { Join-Path $app 'sidenote.exe' } else { '' }
$listenerExit = Invoke-ListenerCheck $exe
if ($Before) {
    Add-Check 'Listener task check' ($listenerExit -eq 0) "exit=$listenerExit; run=no; register=no"
    $rows | Format-Table -AutoSize | Out-String -Width 240 | Write-Output
    exit $listenerExit
}

$installTime = $null
try {
    if (-not $app) { throw 'App unavailable' }
    $appFiles = @(Get-ChildItem -LiteralPath $app -File -Recurse -Force)
    if (-not $appFiles.Count) { throw 'Install time unavailable' }
    $installTime = ($appFiles | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1).LastWriteTimeUtc
} catch { $installTime = $null }

try {
    if (-not $app -or -not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'App unavailable' }
    # Disable XML entity resolution even though this is local installer metadata.
    $xml = [xml]::new()
    $xml.XmlResolver = $null
    $xml.Load((Join-Path $app 'sq.version'))
    $node = $xml.SelectSingleNode('//*[local-name()="metadata"]/*[local-name()="version"]')
    $version = Safe-Version $node.InnerText
    Add-Check 'Installed version' ($version -ceq $Expect) "$version (installer sq.version)"
} catch { Add-Check 'Installed version' $false 'unavailable or invalid installer metadata' }

try {
    $service = Get-CimInstance Win32_Service -Filter "Name = 'Schedule'"
    $parent = Get-CimInstance Win32_Process -Filter "ProcessId = $($service.ProcessId)"
    $recorders = @(Get-CimInstance Win32_Process -Filter "Name = 'sidenote.exe'" | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath -ieq $exe -and
        $_.CommandLine -match '(?i)(?:^|\s)"?--listen"?(?=\s|$)'
    })
    # Creation order rules out a recycled parent PID.
    $owned = @($recorders | Where-Object {
        $service.State -eq 'Running' -and $service.ProcessId -gt 0 -and
        $_.ParentProcessId -eq $service.ProcessId -and $parent.CreationDate -and
        $_.CreationDate -and $parent.CreationDate -le $_.CreationDate
    })
    Add-Check 'Recorder parent' ($recorders.Count -eq 1 -and $owned.Count -eq 1) "listeners=$($recorders.Count); scheduler-parent=$($owned.Count)"
} catch { Add-Check 'Recorder parent' $false 'process or scheduler query unavailable' }

try {
    $task = Get-ScheduledTask -TaskPath '\' -TaskName 'Sidenote meeting listener'
    $state = if ([string]$task.State -in @('Running', 'Ready', 'Disabled', 'Queued', 'Unknown')) { [string]$task.State } else { 'unknown' }
    Add-Check 'Listener task state' ($state -eq 'Running') "exists; $state"
} catch { Add-Check 'Listener task state' $false 'missing or unreadable' }
Add-Check 'Listener task check' ($listenerExit -eq 0) "exit=$listenerExit; run=no; register=no"

$generation = 'invalid'
$checks = [ordered]@{ id_match = $false; folder_exists = $false; manifest_match = $false
    created_after_install = $false; previous_present = $false }
try {
    if (-not $scripts) { throw 'Scripts unavailable' }
    $pointer = Get-Content -LiteralPath (Join-Path $scripts '.pipeline-current.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $checks.previous_present = $pointer.previous.generation -is [string] -and
        -not [string]::IsNullOrWhiteSpace($pointer.previous.generation)
    $checks.id_match = $pointer.current.generation -is [string] -and
        $pointer.current.generation -cmatch '\Ageneration-[0-9a-f]{32}\z'
    if ($checks.id_match) {
        $generation = $pointer.current.generation
        $folder = Get-Item -LiteralPath (Join-Path (Join-Path $scripts '.pipeline-generations') $generation) -Force
        $checks.folder_exists = $folder.PSIsContainer
        if ($checks.folder_exists) {
            $checks.created_after_install = $null -ne $installTime -and $folder.CreationTimeUtc -ge $installTime
            if ($pointer.current.manifest_sha256 -is [string] -and
                $pointer.current.manifest_sha256 -cmatch '\A[0-9a-f]{64}\z') {
                $hash = (Get-FileHash -LiteralPath (Join-Path $folder.FullName '.generation.json') -Algorithm SHA256).Hash
                $checks.manifest_match = $hash -ieq $pointer.current.manifest_sha256
            }
        }
    }
} catch { } # Report only fixed flags, never filesystem or JSON error text.
$flags = @($checks.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join '; '
Add-Check 'Pipeline generation' (-not ($checks.Values -contains $false)) "$generation; $flags"

try {
    if (-not $local) { throw 'Local app data unavailable' }
    $evidence = Get-InstallRecoveryEvidence (Join-Path $local 'Sidenote/updates') $Expect
    Add-Check 'Install recovery' $true $evidence
} catch { Add-Check 'Install recovery' $false 'incomplete or unreadable' }

try {
    if (-not $installTime -or -not $scripts) { throw 'Folders or install time unavailable' }
    $newest = Get-ChildItem -LiteralPath $scripts -File -Recurse -Force | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    $observed = 'scripts=missing'
    if ($newest) { $observed = 'scripts=' + $newest.LastWriteTimeUtc.ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ') }
    # Symbolic folder identifies the source without printing any environment value.
    $observed += '; install=' + $installTime.ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ') + ' (newest file in %LOCALAPPDATA%/Sidenote/current)'
    Add-Check 'Scripts newer than install' ($null -ne $newest -and $newest.LastWriteTimeUtc -gt $installTime) $observed
} catch { Add-Check 'Scripts newer than install' $false 'folder or timestamp unavailable; install folder=%LOCALAPPDATA%/Sidenote/current' }

try {
    if (-not $installTime -or -not $local) { throw 'Install time unavailable' }
    $log = Join-Path $local 'hex-meeting-watch.log'
    $count = 0
    $undated = 0
    $kinds = [Collections.Generic.HashSet[string]]::new()
    # Continuations inherit the preceding entry's time; leading undated text is old.
    $stamp = [datetimeoffset]::MinValue
    # Streaming scan of this diagnostic log only; no meeting files are opened.
    foreach ($line in [IO.File]::ReadLines($log)) {
        $dated = $false
        $parsed = [datetimeoffset]::MinValue
        if ($line -match '^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)(?=\s|$)' -and
            [datetimeoffset]::TryParse($Matches[1], [ref]$parsed)) {
            $stamp = $parsed
            $dated = $true
        }
        if ($line -notmatch '(?i)\b(error|failed|failure|exception|timeout|timed out)\b|\bkind\s*=|(?-i:\b[A-Z][A-Z0-9_]*_[A-Z0-9_]+\s*\([A-Za-z][A-Za-z0-9]*(?:,\s*os\s+-?\d+)?\))') { continue }
        if ($stamp.UtcDateTime -lt $installTime) { continue }
        $count++
        if (-not $dated) { $undated++ }
        # Only fixed categories can leave this process, never raw log tokens.
        $kind = 'error'
        if ($line -match '\bpipeline-failure\b') {
            $kind = 'pipeline-failure'
            if ($line -match '\bcategory=(timeout|exit|missing)(?=\s|$)') { $kind += '/' + $Matches[1].ToLowerInvariant() }
        } elseif ($line -match '(?i)\b(timeout|timed out)\b') { $kind = 'timeout' }
        elseif ($line -match '(?i)\bexception\b') { $kind = 'exception' }
        elseif ($line -match '(?i)\b(failed|failure)\b') { $kind = 'failure' }
        [void]$kinds.Add($kind)
    }
    $names = if ($kinds.Count) { ($kinds | Sort-Object) -join ', ' } else { 'none' }
    Add-Check 'Errors since install' ($count -eq 0) "count=$count; kinds=$names; undated-error-lines=$undated"
} catch { Add-Check 'Errors since install' $false 'count=unknown; kinds=unavailable' }

$rows.Add([pscustomobject]@{ Check = 'Idle CPU (informational)'; Result = 'INFO'; Observed = (Measure-IdleCpu $exe) })
$rows | Format-Table -AutoSize | Out-String -Width 320 | Write-Output
if (@($rows | Where-Object Result -eq 'FAIL').Count) { exit 1 }
exit 0
