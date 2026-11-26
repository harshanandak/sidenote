#Requires -Version 7.4
[CmdletBinding()]
param([switch]$SelfTest)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$checksumError = 'The update checksum does not match. Try again.'

function Read-Json([string]$Path) {
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -AsHashtable -NoEnumerate
}

function Set-SourceVersion([string]$Source, [string]$Version) {
    foreach ($entry in @(@('Cargo.toml', '[package]'), @('Cargo.lock', '[[package]]'))) {
        $path = Join-Path $Source $entry[0]
        $text = [IO.File]::ReadAllText($path)
        $header = [regex]::Escape($entry[1])
        $fields = [regex]::Matches($text, "(?m)^$header\r?\nname = ""sidenote""\r?\n(?:default-run = ""sidenote""\r?\n)?version = ""([^""\r\n]+)""\r?$")
        if ($fields.Count -ne 1) { throw "Expected one Sidenote version in $($entry[0])." }
        $field = $fields[0].Groups[1]
        [IO.File]::WriteAllText($path, $text.Remove($field.Index, $field.Length).Insert($field.Index, $Version))
    }
}

function New-ProofFixture([string]$Root, [string]$Source) {
    $proofProfile = Join-Path $Root 'profile'
    $install = Join-Path $Root 'install'
    $local = Join-Path $proofProfile 'local'
    $scripts = Join-Path $proofProfile '.config/hex/scripts'
    $state = Join-Path $Root 'state'
    $meetings = Join-Path $Root 'meetings'
    $peers = Join-Path $Root 'peers'
    $feed = Join-Path $Root 'feed'
    $environment = @{
        USERPROFILE = $proofProfile
        APPDATA = Join-Path $proofProfile 'roaming'
        LOCALAPPDATA = $local
        SIDENOTE_DATA_DIR = $state
        SIDENOTE_HEX_DIR = $meetings
        SIDENOTE_PEERS_DIR = Join-Path $Root 'hook-peers'
        SIDENOTE_UPDATE_FEED = $feed
        SIDENOTE_LISTENER_TASK = "Sidenote updates proof $([guid]::NewGuid().ToString('N'))"
    }
    foreach ($setting in @('HEX_MEETING_PROJECT', 'HEX_MEETING_PROJECT_PATTERN', 'HEX_MEETING_PROJECT_CONTEXT_PATTERN',
        'HEX_MEETING_KB_REPO', 'HEX_MEETING_OWNER_NAME', 'HEX_MEETING_COMPANY_NAME',
        'HEX_MEETING_PRODUCT_NAME', 'HEX_TE_ASR_MODEL')) {
        $environment[$setting] = 'fixture'
    }
    $sentinels = @{}
    foreach ($directory in @($state, $meetings, $peers)) {
        [void][IO.Directory]::CreateDirectory($directory)
        $path = Join-Path $directory 'fixture.txt'
        Set-Content -LiteralPath $path -Value 'Synthetic release update fixture.' -NoNewline
        $sentinels[$path] = (Get-FileHash -LiteralPath $path).Hash
    }
    foreach ($directory in @($scripts, $local, $environment.APPDATA, $environment.SIDENOTE_PEERS_DIR)) {
        [void][IO.Directory]::CreateDirectory($directory)
    }
    Get-ChildItem -LiteralPath (Join-Path $Source 'scripts/meeting-pipeline') -File |
        Where-Object Name -NotLike 'test_*' | Copy-Item -Destination $scripts
    return @{
        Install = $install; Exe = Join-Path $install 'current/sidenote.exe'; Feed = $feed
        Env = $environment; Task = $environment.SIDENOTE_LISTENER_TASK; Peers = $peers
        Updates = Join-Path $local 'Sidenote/updates'; Scripts = $scripts; Sentinels = $sentinels
    }
}

function Invoke-ProofProcess($Fixture, [string]$Exe, [string[]]$Arguments, [switch]$Capture) {
    $start = [Diagnostics.ProcessStartInfo]::new($Exe)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WorkingDirectory = Split-Path -Parent $Exe
    $start.RedirectStandardOutput = $Capture.IsPresent
    $start.RedirectStandardError = $Capture.IsPresent
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    foreach ($entry in $Fixture.Env.GetEnumerator()) { $start.Environment[$entry.Key] = $entry.Value }
    $process = [Diagnostics.Process]::Start($start)
    try {
        if ($Capture) {
            $stdout = $process.StandardOutput.ReadToEndAsync()
            $stderr = $process.StandardError.ReadToEndAsync()
        }
        if (-not $process.WaitForExit(120000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "Timed out running $([IO.Path]::GetFileName($Exe))."
        }
        if ($Capture -and $process.ExitCode -ne 0) {
            Write-Host "$($Arguments[0]) exited with code $($process.ExitCode)."
            Write-Host ($stdout.GetAwaiter().GetResult())
            Write-Host ($stderr.GetAwaiter().GetResult())
        }
        $process.ExitCode
    } finally { $process.Dispose() }
}

function Invoke-UpdateCommand($Fixture, [string]$Command) {
    $capture = $Command -in @('--update-window-start', '--update-status')
    Invoke-ProofProcess $Fixture $Fixture.Exe @($Command, '--peers-dir', $Fixture.Peers) -Capture:$capture
}

function Assert-Report($Fixture, $Report, [string]$Version) {
    if ($Report.installed -ne $true) { throw 'Unexpected update report field: installed.' }
    if ($Report.current_version -cne $Version) { throw 'Unexpected update report field: current_version.' }
    # Rust preserves the slash in Sidenote/updates. Join-Path uses the native separator.
    $updateDir = ([string]$Report.update_dir).Replace([IO.Path]::AltDirectorySeparatorChar, [IO.Path]::DirectorySeparatorChar)
    if ($updateDir -ine $Fixture.Updates) { throw 'Unexpected update report field: update_dir.' }
    if ($Report.data_dir -ine $Fixture.Env.SIDENOTE_DATA_DIR) { throw 'Unexpected update report field: data_dir.' }
    if ($Report.peers_dir -ine $Fixture.Peers) { throw 'Unexpected update report field: peers_dir.' }
    if ($Report.feed -ine $Fixture.Feed) { throw 'Unexpected update report field: feed.' }
    if ($Report.listener_task -cne $Fixture.Task) { throw 'Unexpected update report field: listener_task.' }
}

function Assert-ChecksumRefusal([int]$ExitCode, $Status) {
    if ($ExitCode -ne 1 -or $Status.phase -cne 'Error' -or $Status.error -cne $checksumError) {
        throw 'Apply did not report the expected checksum refusal.'
    }
}

function Get-Listener($Fixture) {
    $listeners = @(Get-CimInstance Win32_Process -Filter "Name = 'sidenote.exe'" | Where-Object {
        $_.ExecutablePath -ieq $Fixture.Exe -and $_.CommandLine -match '(?:^|\s)--listen(?:\s|$)'
    })
    if ($listeners.Count -gt 1) { throw 'More than one isolated listener is running.' }
    $listeners
}

function Wait-Proof([scriptblock]$Probe, [string]$Failure) {
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    do {
        $result = & $Probe
        if ($result) { return $result }
        Start-Sleep -Milliseconds 250
    } while ($deadline.Elapsed.TotalSeconds -lt 90)
    throw $Failure
}

function Assert-Fixture($Fixture, [string]$Version) {
    foreach ($entry in $Fixture.Sentinels.GetEnumerator()) {
        if ((Get-FileHash -LiteralPath $entry.Key).Hash -ne $entry.Value) { throw 'Synthetic fixture content changed.' }
    }
    foreach ($marker in @('update-listener.stop', 'update-listener.resume', 'pipeline-install.pending')) {
        if (Test-Path -LiteralPath (Join-Path $Fixture.Updates $marker)) { throw "Unexpected marker: $marker." }
    }
    if ([IO.File]::ReadAllText((Join-Path $Fixture.Updates 'install-hook.complete')) -cne $Version) {
        throw "Install hook did not complete for $Version."
    }
    $pointer = Read-Json (Join-Path $Fixture.Scripts '.pipeline-current.json')
    $manifestPath = Join-Path $Fixture.Scripts ".pipeline-generations/$($pointer.current.generation)/.generation.json"
    if ((Get-FileHash -LiteralPath $manifestPath).Hash -ine $pointer.current.manifest_sha256 -or
        (Read-Json $manifestPath).version -cne $Version) {
        throw "The isolated pipeline did not publish a sealed generation for $Version."
    }
    $pointer
}

function Invoke-SelfTest([string]$Root, [string]$Source) {
    $fixture = New-ProofFixture $Root $Source
    if ((Test-Path -LiteralPath $fixture.Install) -or (Test-Path -LiteralPath $fixture.Updates) -or
        (Test-Path -LiteralPath $fixture.Feed) -or $fixture.Env.SIDENOTE_PEERS_DIR -eq $fixture.Peers -or
        $fixture.Sentinels.Count -ne 3 -or
        -not (Test-Path -LiteralPath (Join-Path $fixture.Scripts 'hex-meeting-watch.ps1') -PathType Leaf)) {
        throw 'Fixture setup must leave a clean install and seed isolated paths.'
    }
    $report = @{
        installed = $true; current_version = '0.1.0'; update_dir = [IO.Path]::Combine($fixture.Env.LOCALAPPDATA, 'Sidenote/updates')
        data_dir = $fixture.Env.SIDENOTE_DATA_DIR; peers_dir = $fixture.Peers; feed = $fixture.Feed; listener_task = $fixture.Task
    }
    Assert-Report $fixture $report '0.1.0'
    $report.update_dir = Join-Path $fixture.Env.LOCALAPPDATA 'Sidenote/wrong'
    $rejected = $false
    try { Assert-Report $fixture $report '0.1.0' }
    catch {
        if ($_.Exception.Message -cne 'Unexpected update report field: update_dir.') { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw 'Report assertion accepted a different update directory.' }
    $manifest = @('[package]', 'name = "sidenote"', 'default-run = "sidenote"', 'version = "0.0.1"', '') -join [char]10
    $sidenoteEntry = @('[[package]]', 'name = "sidenote"', 'version = "0.0.1"', '') -join [Environment]::NewLine
    $otherEntry = @('[[package]]', 'name = "other"', 'version = "0.0.1"', '', '') -join [Environment]::NewLine
    [IO.File]::WriteAllText((Join-Path $Root 'Cargo.toml'), $manifest)
    [IO.File]::WriteAllText((Join-Path $Root 'Cargo.lock'), $otherEntry + $sidenoteEntry)
    Set-SourceVersion $Root '0.0.2'
    $expectedLock = $otherEntry + $sidenoteEntry.Replace('0.0.1', '0.0.2')
    if ([IO.File]::ReadAllText((Join-Path $Root 'Cargo.toml')) -cne $manifest.Replace('0.0.1', '0.0.2') -or
        [IO.File]::ReadAllText((Join-Path $Root 'Cargo.lock')) -cne $expectedLock) {
        throw 'Version substitution changed more than the Sidenote version.'
    }
    Assert-ChecksumRefusal 1 @{ phase = 'Error'; error = $checksumError }
    foreach ($case in @(@(0, $checksumError), @(1, 'A different failure.'))) {
        try { Assert-ChecksumRefusal $case[0] @{ phase = 'Error'; error = $case[1] } }
        catch {
            if ($_.Exception.Message -cne 'Apply did not report the expected checksum refusal.') { throw }
            continue
        }
        throw 'Refusal assertion accepted a success or an unrelated failure.'
    }
    Write-Output 'Release update fixture, report and refusal self-test passed.'
}

if (-not $SelfTest -and (-not $IsWindows -or $env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted')) {
    throw 'Run the install proof only on a GitHub-hosted Windows runner.'
}
$root = [IO.Directory]::CreateTempSubdirectory('sidenote-update-proof-')
$repo = Split-Path -Parent $PSScriptRoot
$fixture = $null
try {
    if ($SelfTest) { Invoke-SelfTest $root.FullName $repo; return }

    $n = '0.1.0'
    $next = '0.1.1'
    $source = Join-Path $root.FullName 'source'
    $archive = Join-Path $root.FullName 'source.tar'
    [void][IO.Directory]::CreateDirectory($source)
    & git -C $repo archive "--output=$archive" HEAD
    if ($LASTEXITCODE) { throw 'Could not export the source for the proof builds.' }
    & tar.exe -xf $archive -C $source
    if ($LASTEXITCODE) { throw 'Could not extract the source for the proof builds.' }
    $fixture = New-ProofFixture $root.FullName $source
    $setup = Join-Path $root.FullName 'sidenote-stable-Setup.exe'
    $feedSetup = Join-Path $fixture.Feed 'sidenote-stable-Setup.exe'
    $env:GITHUB_ACTIONS = $null # Disable ANSI in the pinned vpk help captured by the packer.
    Push-Location -LiteralPath $source
    try {
        & cargo +stable-x86_64-pc-windows-gnu fetch --locked --target x86_64-pc-windows-gnu
        if ($LASTEXITCODE) { throw 'Could not fetch the locked Windows GNU dependencies.' }
        foreach ($version in @($n, $next)) {
            Set-SourceVersion $source $version
            & (Join-Path $source 'tools/pack_release.ps1') -Feed $fixture.Feed -Channel stable
            if ($version -eq $n) { Copy-Item -LiteralPath $feedSetup -Destination $setup }
        }
    } finally { Pop-Location }
    $index = Read-Json (Join-Path $fixture.Feed 'releases.stable.json')
    foreach ($expected in @(@($n, 'Full'), @($next, 'Full'), @($next, 'Delta'))) {
        $kind = $expected[1].ToLowerInvariant()
        $name = "sidenote-$($expected[0])-stable-$kind.nupkg"
        $assets = @($index.Assets | Where-Object FileName -CEQ $name)
        $package = Get-Item -LiteralPath (Join-Path $fixture.Feed $name)
        if ($assets.Count -ne 1 -or $assets[0].PackageId -cne 'sidenote' -or
            $assets[0].Version -cne $expected[0] -or $assets[0].Type -cne $expected[1] -or
            $package.Length -le 0 -or $package.Length -ne $assets[0].Size -or
            (Get-FileHash -LiteralPath $package.FullName).Hash -ine $assets[0].SHA256) {
            throw "Expected a verified feed asset: $name."
        }
    }
    Write-Output 'Verified both full packages and the delta in the isolated feed.'

    if ((Test-Path -LiteralPath $fixture.Install) -or (Test-Path -LiteralPath $fixture.Updates)) {
        throw 'The isolated install is not clean.'
    }
    if (Get-CimInstance Win32_Process -Filter "Name = 'hex.exe' OR Name = 'hex.com' OR Name = 'sidenote.exe' OR Name = 'hex-meetings.exe'") {
        throw 'The runner already has a Hex or Sidenote process.'
    }
    [void](Get-ScheduledTask -TaskPath '\')
    $setupLog = Join-Path $root.FullName 'setup.log'
    $setupExit = Invoke-ProofProcess $fixture $setup @('--silent', '--installto', $fixture.Install, '--log', $setupLog)
    if (Test-Path -LiteralPath $setupLog) {
        Select-String -LiteralPath $setupLog -Pattern 'hook| WARN | ERROR ' | ForEach-Object { Write-Host $_.Line }
    }
    if ($setupExit -ne 0) { throw "Setup exited with code $setupExit." }
    $reportPath = Join-Path $fixture.Updates 'update-report.json'
    if (Test-Path -LiteralPath $reportPath) {
        $hookStatus = (Read-Json $reportPath).status
        Write-Host "Setup hook reported $($hookStatus.phase). $($hookStatus.error)"
    }
    # Setup runs the fast hook, then stops app processes. Let normal startup reconcile its listener.
    if ((Invoke-UpdateCommand $fixture '--update-window-start') -ne 0) { throw 'Clean install startup failed.' }
    $listener = Wait-Proof { Get-Listener $fixture } 'The clean installation did not start its isolated listener.'
    if ((Invoke-UpdateCommand $fixture '--update-status') -ne 0) { throw 'Clean install status failed.' }
    Assert-Report $fixture (Read-Json $reportPath) $n
    $initialPointer = Assert-Fixture $fixture $n
    Write-Output "Verified the clean $n install, listener and pipeline."

    $checkExit = Invoke-UpdateCommand $fixture '--update-check'
    $report = Read-Json $reportPath
    Assert-Report $fixture $report $n
    if ($checkExit -ne 0) { throw "Update check failed: $($report.status.error)" }
    if ($report.status.phase -cne 'Ready' -or $report.status.version -cne $next -or $report.status.progress -ne 100) {
        throw "Update check did not download $next."
    }
    $readyPath = Join-Path $fixture.Updates 'update-ready.json'
    $original = [IO.File]::ReadAllBytes($readyPath)
    $ready = Read-Json $readyPath
    $hash = $ready[1].SHA256
    $replacement = if ($hash[0] -eq '0') { '1' } else { '0' }
    $ready[1].SHA256 = $replacement + $hash.Substring(1)
    try {
        ConvertTo-Json -InputObject $ready -Depth 10 | Set-Content -LiteralPath $readyPath -NoNewline
        $exitCode = Invoke-UpdateCommand $fixture '--update-apply'
        $refusal = Read-Json $reportPath
        Assert-Report $fixture $refusal $n
        Assert-ChecksumRefusal $exitCode $refusal.status
        if ((Get-Listener $fixture).ProcessId -ne $listener.ProcessId) { throw 'Checksum refusal disturbed the listener.' }
        [void](Assert-Fixture $fixture $n)
    } finally { [IO.File]::WriteAllBytes($readyPath, $original) }
    Write-Output 'Verified checksum refusal before the listener or install changed.'

    $applyStarted = Get-Date
    if ((Invoke-UpdateCommand $fixture '--update-apply') -ne 0) {
        throw "Valid update apply failed: $((Read-Json $reportPath).status.error)"
    }
    $updated = Wait-Proof {
        $report = Read-Json $reportPath
        # Only the restarted CLI gets the peers flag. The Velopack hooks use the environment fallback.
        if ($report.current_version -ceq $next -and $report.peers_dir -ieq $fixture.Peers) { $report }
    } "The restarted app did not report $next."
    Assert-Report $fixture $updated $next
    if ($updated.status.phase -cne 'Idle' -or $null -ne $updated.status.error) { throw 'Updated app reported an error.' }
    $newListener = Wait-Proof {
        Get-Listener $fixture | Where-Object { $_.ProcessId -ne $listener.ProcessId -and $_.CreationDate -ge $applyStarted }
    } 'The update did not restart the isolated listener.'
    $finalPointer = Assert-Fixture $fixture $next
    if ($finalPointer.previous.generation -cne $initialPointer.current.generation) {
        throw 'The update did not retain the previous pipeline generation.'
    }
    $current = Split-Path -Parent $fixture.Exe
    foreach ($file in @('sidenote-sync-worker.exe', 'onnxruntime.dll', 'onnxruntime_providers_shared.dll',
        'ONNXRuntime-LICENSE.txt', 'ONNXRuntime-ThirdPartyNotices.txt', 'LICENSE', 'THIRD_PARTY.md',
        'DEPENDENCY_NOTICES.md', 'DEPENDENCY_NOTICE_COVERAGE.json', 'assets/fonts/Inter-LICENSE.txt',
        'assets/fonts/Fraunces-LICENSE.txt', 'assets/fonts/Figtree-LICENSE.txt', 'assets/fonts/NotoSansDevanagari-LICENSE.txt',
        'assets/fonts/NotoSansTelugu-LICENSE.txt', 'assets/licenses/webpki-roots-CDLA-Permissive-2.0.txt',
        'assets/licenses/lucide-LICENSE.txt')) {
        if ((Get-Item -LiteralPath (Join-Path $current $file)).Length -le 0) { throw "Missing bundle file: $file." }
    }
    Write-Output "Verified $next, the restarted listener (PID $($newListener.ProcessId)), preserved fixtures and bundled runtime and notices."
} finally {
    if ($fixture -and -not $SelfTest) {
        if (Get-ScheduledTask -TaskPath '\' -TaskName $fixture.Task -ErrorAction SilentlyContinue) {
            Unregister-ScheduledTask -TaskPath '\' -TaskName $fixture.Task -Confirm:$false
        }
        Get-Process -Name sidenote,Update -ErrorAction SilentlyContinue | Where-Object {
            $_.Path -and $_.Path.StartsWith($fixture.Install + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
        } | Stop-Process -Force
    }
    $root.Delete($true)
}
