#Requires -Version 7.4
[CmdletBinding(DefaultParameterSetName = 'Source')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Test')][switch]$SelfTest,
    [Parameter(Mandatory, ParameterSetName = 'Package')][string]$PackageDirectory,
    [Parameter(Mandatory, ParameterSetName = 'Package')][string]$ExpectedTag,
    [Parameter(ParameterSetName = 'Source')]
    [Parameter(ParameterSetName = 'Package')][string]$DiagnosticsDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$checksumError = 'The update checksum does not match. Try again.'
$strictTaskError = 'The meeting listener task does not target this Sidenote installation with the expected arguments.'
$installMarkers = @('update-listener.stop', 'update-listener.resume', 'pipeline-install.pending', 'install-hook.complete')

function Read-Json([string]$Path) {
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -AsHashtable -NoEnumerate
}

function Copy-VerifiedPackageFiles([string]$Directory, [string]$Feed) {
    $manifest = Get-Item -LiteralPath (Join-Path $Directory 'SHA256SUMS')
    [void][IO.Directory]::CreateDirectory($Feed)
    $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($line in [IO.File]::ReadAllLines($manifest.FullName)) {
        if ($line -cnotmatch '\A([0-9A-Fa-f]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)\z') {
            throw 'Invalid SHA256SUMS entry.'
        }
        $hash, $name = $Matches[1], $Matches[2]
        if ($name -ieq 'SHA256SUMS' -or -not $names.Add($name)) { throw "Unexpected SHA256SUMS entry: $name." }
        $file = Get-Item -LiteralPath (Join-Path $Directory $name)
        if ($file -isnot [IO.FileInfo] -or $file.Name -cne $name -or
            ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Expected a regular package file: $name."
        }
        $copy = Join-Path $Feed $name
        Copy-Item -LiteralPath $file.FullName -Destination $copy
        if ((Get-FileHash -LiteralPath $copy).Hash -ine $hash) { throw "SHA256SUMS mismatch: $name." }
        Write-Output "Verified SHA256SUMS for $name ($hash)."
    }
    if (-not $names.Count) { throw 'SHA256SUMS is empty.' }
    # Publishing uploads every file in the directory, so each one must be covered.
    $unlisted = @(Get-ChildItem -LiteralPath $Directory -Force | Where-Object { $_.Name -cne 'SHA256SUMS' -and -not $names.Contains($_.Name) })
    if ($unlisted.Count) { throw "Unlisted package file: $($unlisted[0].Name)." }
}

function Get-ProofPackage([string]$Feed, [string]$Tag) {
    . (Join-Path $PSScriptRoot 'release_ci.ps1') -ValidateOnly
    $full = @(Get-ChildItem -LiteralPath $Feed -File -Filter '*-full.nupkg')
    if ($full.Count -ne 1 -or $full[0].Name -cnotmatch
        '\Asidenote-(\d+\.\d+\.\d+(?:-dev\.(?:0|[1-9]\d*))?)-(stable|dev)-full\.nupkg\z') {
        throw 'Expected one versioned Sidenote full package.'
    }
    $version, $channel = $Matches[1], $Matches[2]
    if (($version.Contains('-dev.') -ne ($channel -ceq 'dev')) -or ($Tag -and $Tag -cne "v$version")) {
        throw 'The package version or channel does not match the release.'
    }
    [void](Get-ReleaseAssets $Feed $version $channel)
    return @{ Version = $version; Channel = $channel; Full = $full[0] }
}

function Assert-PackageExecutable($Fixture, $Release) {
    $archive = [IO.Compression.ZipFile]::OpenRead($Release.Full.FullName)
    try {
        $entry = @($archive.Entries | Where-Object FullName -CMatch '\Alib/[^/]+/sidenote\.exe\z')
        if ($entry.Count -ne 1) { throw 'Expected one Sidenote executable in the full package.' }
        $stream = $entry[0].Open()
        try { $hash = (Get-FileHash -InputStream $stream).Hash }
        finally { $stream.Dispose() }
    } finally { $archive.Dispose() }
    if ((Get-FileHash -LiteralPath $Fixture.Exe).Hash -ine $hash) {
        throw 'The installed executable does not match the verified full package.'
    }
    Write-Output "Installed executable matches $($Release.Full.Name) ($hash)."
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

function New-ProofFixture([string]$Root, [string]$Source, [string]$Feed = (Join-Path $Root 'feed')) {
    $proofProfile = Join-Path $Root 'profile'
    $install = Join-Path $Root 'install'
    $local = Join-Path $proofProfile 'local'
    $scripts = Join-Path $proofProfile '.config/hex/scripts'
    $state = Join-Path $Root 'state'
    $meetings = Join-Path $Root 'meetings'
    $peers = Join-Path $Root 'peers'
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

function Invoke-ProofProcess($Fixture, [string]$Exe, [string[]]$Arguments, [switch]$Capture, [switch]$Quiet) {
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
        if ($Capture -and -not $Quiet -and $process.ExitCode -ne 0) {
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

function Assert-SetupPreflight($Fixture, $OldListener) {
    if (Test-Path -LiteralPath $Fixture.Install) { throw 'The isolated target install is not fresh.' }
    $processes = @(Get-CimInstance Win32_Process -Filter "Name = 'hex.exe' OR Name = 'hex.com' OR Name = 'sidenote.exe' OR Name = 'hex-meetings.exe'")
    if ($OldListener) {
        if ($processes.Count -ne 1 -or $processes[0].ProcessId -ne $OldListener.ProcessId -or
            $processes[0].CreationDate -ne $OldListener.CreationDate -or $processes[0].ExecutablePath -ine $OldListener.ExecutablePath) {
            throw 'Expected only the recorded old fixture listener before Setup.'
        }
    } elseif ($processes.Count) {
        throw 'The runner already has a Hex or Sidenote process.'
    }
}

function Read-SetupHookLog([string[]]$Lines) {
    $result = @{ Starts = 0; Successes = 0; Failures = 0; HookExit = $null; Timeouts = 0; Milliseconds = -1 }
    foreach ($line in $Lines) {
        switch -Regex -CaseSensitive ($line) {
            'Running --veloapp-install hook\.\.\.$' { $result.Starts++ }
            'Hook executed successfully \(took (\d+(?:\.\d+)?)ms\)$' {
                if ($result.Starts) {
                    $result.Successes++
                    $result.Milliseconds = [double]$Matches[1]
                }
            }
            'Hook exited with non-zero exit code: (\d+)$' {
                $result.Failures++
                $result.HookExit = [int]$Matches[1]
            }
            'Failed to start hook --veloapp-install:|Error waiting for process to finish:' { $result.Failures++ }
            'Process timed out after \d+s$' { $result.Timeouts++ }
        }
    }
    $result
}

function Assert-SetupHookSucceeded($Setup) {
    if ($Setup.Exit -ne 0 -or $Setup.Starts -ne 1 -or $Setup.Successes -ne 1 -or
        $Setup.Failures -or $Setup.Timeouts -or $Setup.Milliseconds -lt 0 -or $Setup.Milliseconds -ge 30000) {
        throw 'Raw Setup did not log one successful install hook below 30 seconds.'
    }
}

function Invoke-RawSetup($Fixture, [string]$SetupFeed, [string]$Version, [string]$Case, $OldListener) {
    Assert-SetupPreflight $Fixture $OldListener
    $setupLog = $Fixture.Install + '-setup.log'
    $installer = Join-Path $PSScriptRoot 'install_release.ps1'
    $setupExit = Invoke-ProofProcess $Fixture (Join-Path $PSHOME 'pwsh.exe') -Capture -Quiet -Arguments @(
        '-NoProfile', '-File', $installer, '-Feed', $SetupFeed, '-Version', $Version, '-Log', $setupLog,
        '-ProofInstallDirectory', $Fixture.Install)
    $lines = @()
    if (Test-Path -LiteralPath $setupLog) {
        $lines = @(Get-Content -LiteralPath $setupLog)
    }
    $result = Read-SetupHookLog $lines
    $result.Exit = $setupExit
    $result.Case = $Case
    $result.ExecutablePresent = Test-Path -LiteralPath $Fixture.Exe -PathType Leaf
    $result.ReceiptPresent = Test-Path -LiteralPath (Join-Path $Fixture.Updates 'install-hook.complete')
    Write-Host (ConvertTo-Json -InputObject $result -Compress)
    if ($DiagnosticsDirectory) {
        [void][IO.Directory]::CreateDirectory($DiagnosticsDirectory)
        ConvertTo-Json -InputObject $result | Set-Content -LiteralPath (Join-Path $DiagnosticsDirectory "$Case.json")
    }
    $result
}

function Install-ProofFixture($Fixture, [string]$SetupFeed, [string]$Version, [string]$Case) {
    if (Test-Path -LiteralPath $Fixture.Updates) { throw 'The isolated update directory is not clean.' }
    [void](Get-ScheduledTask -TaskPath '\')
    $setup = Invoke-RawSetup $Fixture $SetupFeed $Version $Case $null
    if ($setup.Exit -ne 0) { throw "Setup exited with code $($setup.Exit)." }
    return Start-ProofFixture $Fixture $Version
}

function Start-ProofFixture($Fixture, [string]$Version) {
    $reportPath = Join-Path $Fixture.Updates 'update-report.json'
    # Setup runs the fast hook, then stops app processes. Let normal startup reconcile its listener.
    if ((Invoke-UpdateCommand $Fixture '--update-window-start') -ne 0) { throw 'Installed fixture startup failed.' }
    $listener = Wait-Proof { Get-Listener $Fixture } 'The installed fixture did not start its isolated listener.'
    if ((Invoke-UpdateCommand $Fixture '--update-status') -ne 0) { throw 'Installed fixture status failed.' }
    Assert-Report $Fixture (Read-Json $reportPath) $Version
    $initialPointer = Assert-Fixture $Fixture $Version
    Write-Host "Verified the $Version install, listener and pipeline."

    return @{ Listener = $listener; Pointer = $initialPointer }
}

function Resolve-TaskSid([string]$User) {
    if ($User -match '^S-1-') { return $User }
    ([Security.Principal.NTAccount]$User).Translate([Security.Principal.SecurityIdentifier]).Value
}

function Register-PreviousInstallFixture($Fixture, [string]$SeedInstall, [string]$Kind) {
    $old = $Fixture.Clone()
    $old.Install = $Fixture.OldInstall
    $old.Exe = Join-Path $old.Install 'current/sidenote.exe'
    Copy-Item -LiteralPath $SeedInstall -Destination $old.Install -Recurse
    $oldSentinel = Join-Path $old.Install 'proof-data.txt'
    [IO.File]::WriteAllText($oldSentinel, 'Synthetic previous-install fixture.')
    foreach ($path in @($oldSentinel, $old.Exe)) { $Fixture.Sentinels[$path] = (Get-FileHash -LiteralPath $path).Hash }
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $arguments = '--listen --listener-isolated --data-dir "{0}" --listener-localappdata "{1}"' -f
        $Fixture.Env.SIDENOTE_DATA_DIR, $Fixture.Env.LOCALAPPDATA
    $action = New-ScheduledTaskAction -Execute $old.Exe -Argument $arguments -WorkingDirectory (Split-Path -Parent $old.Exe)
    $logon = New-ScheduledTaskTrigger -AtLogOn -User $sid
    $retry = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(30) -RepetitionInterval (New-TimeSpan -Minutes 5)
    $level = if ($Kind -eq 'elevated') { 'Highest' } else { 'Limited' }
    $principal = New-ScheduledTaskPrincipal -UserId $sid -LogonType Interactive -RunLevel $level
    $settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -Disable:($Kind -eq 'disabled')
    Register-ScheduledTask -TaskPath '\' -TaskName $Fixture.Task -Action $action -Trigger @($logon, $retry) `
        -Principal $principal -Settings $settings | Out-Null
    # Set the fixture owner explicitly if the elevated runner token assigned it to Administrators.
    $service = New-Object -ComObject 'Schedule.Service'
    $service.Connect()
    $registered = $service.GetFolder('\').GetTask($Fixture.Task)
    $security = [Security.AccessControl.RawSecurityDescriptor]::new($registered.GetSecurityDescriptor(7))
    $owner = [Security.Principal.SecurityIdentifier]$sid
    if ($security.Owner -ne $owner) {
        $security.Owner = $owner
        [void]$registered.SetSecurityDescriptor($security.GetSddlForm('Owner, Group, Access'), 16)
    }
    $old
}

function Get-ProofTaskXml($Fixture) {
    Export-ScheduledTask -TaskPath '\' -TaskName $Fixture.Task
}

function Assert-HandoffTask($Fixture, [string]$Arguments) {
    $task = Get-ScheduledTask -TaskPath '\' -TaskName $Fixture.Task
    $actions = @($task.Actions)
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if ($actions.Count -ne 1 -or $actions[0].Execute -ine $Fixture.Exe -or $actions[0].Arguments -cne $Arguments -or
        $actions[0].WorkingDirectory -ine (Split-Path -Parent $Fixture.Exe) -or
        (Resolve-TaskSid $task.Principal.UserId) -ne $sid -or $task.Principal.LogonType -ne 'Interactive' -or
        $task.Principal.RunLevel -ne 'Limited' -or -not $task.Settings.Enabled) {
        throw 'Setup did not transfer the expected task action and principal.'
    }
    $logon = @($task.Triggers | Where-Object {
        $_.CimClass.CimClassName -eq 'MSFT_TaskLogonTrigger' -and $_.Enabled -and (Resolve-TaskSid $_.UserId) -eq $sid
    })
    $retry = @($task.Triggers | Where-Object {
        $_.CimClass.CimClassName -eq 'MSFT_TaskTimeTrigger' -and $_.Enabled -and
        $_.Repetition.Interval -eq 'PT5M' -and -not $_.Repetition.Duration
    })
    if ($logon.Count -ne 1 -or $retry.Count -ne 1 -or @($task.Triggers).Count -ne 2 -or
        [string]$task.Settings.MultipleInstances -notin @('IgnoreNew', '2') -or $task.Settings.ExecutionTimeLimit -ne 'PT0S') {
        throw 'The transferred task does not have the required restart schedule.'
    }
}

function Assert-HandoffRecord($Record, $Expected, [string]$Version, [bool]$WasRunning) {
    if ($Record.schema -ne 1 -or $Record.phase -cne 'Complete' -or $Record.version -cne $Version) {
        throw 'The handoff record is incomplete or for a different version.'
    }
    $prior = $Record.plan.prior
    if ($prior.exe -ine $Expected.Command -or $prior.args -cne $Expected.Arguments -or
        $prior.working_dir -ine $Expected.WorkingDirectory -or $prior.enabled -ne $true -or $prior.running -ne $WasRunning) {
        throw 'The handoff record lost part of the previous task action.'
    }
}

function Get-ProofState($Fixture) {
    $paths = @((Join-Path $Fixture.Scripts '.pipeline-current.json'))
    $paths += $installMarkers | ForEach-Object { Join-Path $Fixture.Updates $_ }
    $paths += $Fixture.Sentinels.Keys | Sort-Object
    foreach ($path in $paths) {
        if (Test-Path -LiteralPath $path) { (Get-FileHash -LiteralPath $path).Hash } else { 'absent' }
    }
}

function Invoke-SetupHandoffCases([string]$Root, [string]$Source, [string]$Feed, [string]$Version, [string]$SeedInstall, $Release) {
    $cases = @{
        registered = 'setup_adopts_registered_previous_install'
        running = 'setup_hands_off_running_previous_listener'
        disabled = 'setup_refuses_disabled_or_elevated_task-disabled'
        elevated = 'setup_refuses_disabled_or_elevated_task-elevated'
    }
    $refusals = @{
        disabled = 'The meeting listener task is disabled.'
        elevated = 'The meeting listener task is configured to run with elevated privileges.'
    }
    foreach ($kind in @('registered', 'running', 'disabled', 'elevated')) {
        $case = $cases[$kind]
        Write-Host "Running $case."
        $fixture = New-ProofFixture (Join-Path $Root $case) $Source $Feed
        $fixture.OldInstall = $fixture.Install + '-previous'
        try {
            $old = Register-PreviousInstallFixture $fixture $SeedInstall $kind
            $priorXml = Get-ProofTaskXml $fixture
            $priorAction = ([xml]$priorXml).Task.Actions.Exec
            if ($priorAction.WorkingDirectory -ine (Split-Path -Parent $old.Exe)) {
                throw 'The previous fixture task needs its WorkingDirectory.'
            }
            $before = @(Get-ProofState $fixture) -join ','
            $oldListener = $null
            if ($kind -eq 'running') {
                Start-ScheduledTask -TaskPath '\' -TaskName $fixture.Task
                $listenerLock = Join-Path $fixture.Updates 'update-listener.lock'
                $oldListener = Wait-Proof {
                    $listener = Get-Listener $old
                    if ($listener -and (Test-Path -LiteralPath $listenerLock)) {
                        try { [IO.File]::Open($listenerLock, 'Open', 'Read', 'None').Dispose() }
                        catch [IO.IOException] {
                            if (($_.Exception.GetBaseException().HResult -band 0xffff) -ne 32) { throw }
                            $listener
                        }
                    }
                } 'The old listener did not acquire its shared lock.'
            }
            $setup = Invoke-RawSetup $fixture $Feed $Version $case $oldListener
            $statusPath = Join-Path $fixture.Updates 'updates.json'
            if ($refusals.ContainsKey($kind)) {
                $status = Read-Json $statusPath
                if ($setup.Exit -ne 0 -or $setup.Starts -ne 1 -or -not $setup.ExecutablePresent -or
                    $setup.Successes -or $setup.Failures -ne 1 -or $setup.HookExit -ne 1 -or $setup.Timeouts -or
                    $status.phase -cne 'Error' -or $status.error -cne $refusals[$kind]) {
                    throw "$case did not report its expected refusal."
                }
                if ((Get-ProofTaskXml $fixture) -cne $priorXml -or (@(Get-ProofState $fixture) -join ',') -cne $before) {
                    throw "$case changed the task, pipeline, markers or synthetic data."
                }
                Write-Host "Verified $case."
                continue
            }

            Assert-SetupHookSucceeded $setup
            $status = Read-Json $statusPath
            if ($status.phase -cne 'Idle' -or $null -ne $status.error) { throw 'Raw Setup left an installation error.' }
            [void](Assert-Fixture $fixture $Version)
            if ($Release) { Assert-PackageExecutable $fixture $Release }
            Assert-HandoffTask $fixture $priorAction.Arguments
            $records = @(Get-ChildItem -LiteralPath $fixture.Updates -File -Filter 'listener-handoff-*.json' | ForEach-Object {
                $record = Read-Json $_.FullName
                if ($record.task -ceq $fixture.Task -and $record.target -ieq $fixture.Exe) {
                    @{ Path = $_.FullName; Data = $record }
                }
            })
            if ($records.Count -ne 1) { throw 'Expected one durable handoff record.' }
            Assert-HandoffRecord $records[0].Data $priorAction $Version ($kind -eq 'running')
            if ($oldListener -and (Get-CimInstance Win32_Process -Filter "ProcessId = $($oldListener.ProcessId)" |
                Where-Object CreationDate -EQ $oldListener.CreationDate)) { throw 'The recorded old process survived Setup.' }
            if (Get-Listener $old) { throw 'A previous-root listener survived Setup.' }
            $taskXml = Get-ProofTaskXml $fixture
            $recordHash = (Get-FileHash -LiteralPath $records[0].Path).Hash
            [void](Start-ProofFixture $fixture $Version)
            [void](Start-ProofFixture $fixture $Version)
            $strictExit = Invoke-ProofProcess $old $old.Exe @('--update-window-start', '--peers-dir', $old.Peers) -Capture -Quiet
            $strictStatus = (Read-Json (Join-Path $old.Updates 'update-report.json')).status
            if ($strictExit -ne 1 -or $strictStatus.phase -cne 'Error' -or $strictStatus.error -cne $strictTaskError -or
                (Get-Listener $old) -or -not (Get-Listener $fixture) -or (Get-ProofTaskXml $fixture) -cne $taskXml -or
                @(Get-ChildItem -LiteralPath $fixture.Updates -File -Filter 'listener-handoff-*.json').Count -ne 1 -or
                (Get-FileHash -LiteralPath $records[0].Path).Hash -cne $recordHash) {
                throw 'startup_cannot_take_back_listener failed.'
            }
            [void](Assert-Fixture $fixture $Version)
            Write-Host "Verified $case and startup_cannot_take_back_listener."
        } finally { Remove-ProofFixture $fixture }
    }
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
    $packages = Join-Path $Root 'packages'
    [void][IO.Directory]::CreateDirectory($packages)
    $file = Join-Path $packages 'sidenote-dev-Setup.exe'
    [IO.File]::WriteAllText($file, 'Synthetic bytes')
    $sum = Join-Path $packages 'SHA256SUMS'
    $entry = "$((Get-FileHash -LiteralPath $file).Hash)  sidenote-dev-Setup.exe"
    [IO.File]::WriteAllText($sum, $entry)
    Copy-VerifiedPackageFiles $packages (Join-Path $Root 'verified') | Out-Null
    foreach ($case in @(
        @{ Entry = $entry.Replace('sidenote-dev-Setup.exe', '../sidenote-dev-Setup.exe'); Error = 'Invalid SHA256SUMS entry.' }
        @{ Entry = "$('0' * 64)  sidenote-dev-Setup.exe"; Error = 'SHA256SUMS mismatch: sidenote-dev-Setup.exe.' }
        @{ Entry = $entry; Stray = $true; Error = 'Unlisted package file: stray.txt.' }
    )) {
        [IO.File]::WriteAllText($sum, $case.Entry)
        if ($case['Stray']) { [IO.File]::WriteAllText((Join-Path $packages 'stray.txt'), 'Unlisted bytes') }
        $rejected = $false
        try { Copy-VerifiedPackageFiles $packages (Join-Path $Root 'verified') | Out-Null }
        catch {
            if ($_.Exception.Message -cne $case.Error) { throw }
            $rejected = $true
        }
        if (-not $rejected) { throw 'Package checksum verification accepted invalid input.' }
    }
    $start = 'Running --veloapp-install hook...'
    $success = 'Hook executed successfully (took 1234ms)'
    $setup = Read-SetupHookLog @($start, $success)
    $setup.Exit = 0
    Assert-SetupHookSucceeded $setup
    foreach ($lines in @(@($start, 'Hook exited with non-zero exit code: 1'), @($start, $success, 'Process timed out after 30s'),
        @($start, $success.Replace('1234', '30000')), @($success, $start))) {
        $setup = Read-SetupHookLog $lines
        $setup.Exit = 0
        try { Assert-SetupHookSucceeded $setup }
        catch {
            if ($_.Exception.Message -cne 'Raw Setup did not log one successful install hook below 30 seconds.') { throw }
            continue
        }
        throw 'Setup assertion accepted an invalid hook log.'
    }
    $prior = @{ Command = 'old.exe'; Arguments = '--listen'; WorkingDirectory = 'previous' }
    $record = @{ schema = 1; phase = 'Complete'; version = '0.1.0'; plan = @{
        prior = @{ exe = 'old.exe'; args = '--listen'; working_dir = 'previous'; enabled = $true; running = $false }
    } }
    Assert-HandoffRecord $record $prior '0.1.0' $false
    $record.plan.prior.working_dir = ''
    try { Assert-HandoffRecord $record $prior '0.1.0' $false }
    catch {
        if ($_.Exception.Message -cne 'The handoff record lost part of the previous task action.') { throw }
        Write-Output 'Release fixture, package checksum, Setup hook and previous-action self-tests passed.'
        return
    }
    throw 'Handoff assertion accepted a dropped working directory.'
}

function Remove-ProofFixture($Fixture) {
    if (Get-ScheduledTask -TaskPath '\' -TaskName $Fixture.Task -ErrorAction SilentlyContinue) {
        Unregister-ScheduledTask -TaskPath '\' -TaskName $Fixture.Task -Confirm:$false
    }
    $roots = @($Fixture.Install, $Fixture['OldInstall']) | Where-Object { $_ }
    $processes = @(Get-Process | Where-Object {
        $path = $_.Path
        $path -and ($roots | Where-Object {
            $path.StartsWith($_ + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
        })
    })
    $processes | Stop-Process -Force
    $processes | Wait-Process -Timeout 10
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
    $channel = 'stable'
    $release = $null
    $feed = Join-Path $root.FullName 'feed'
    if ($PackageDirectory) {
        Copy-VerifiedPackageFiles $PackageDirectory $feed
        $release = Get-ProofPackage $feed $ExpectedTag
        $next = $release.Version
        $channel = $release.Channel
        # Velopack needs a version core of at least 0.0.1; prereleases of 0.0.1 are accepted.
        $n = if ($channel -ceq 'dev') { '0.0.1-dev.0' } else { '0.0.1' }
        if ([System.Management.Automation.SemanticVersion]$next -le [System.Management.Automation.SemanticVersion]$n) {
            throw "The release needs to be newer than the synthetic baseline $n."
        }
    }
    $source = Join-Path $root.FullName 'source'
    $archive = Join-Path $root.FullName 'source.tar'
    [void][IO.Directory]::CreateDirectory($source)
    & git -C $repo archive "--output=$archive" HEAD
    if ($LASTEXITCODE) { throw 'Could not export the source for the proof builds.' }
    & tar.exe -xf $archive -C $source
    if ($LASTEXITCODE) { throw 'Could not extract the source for the proof builds.' }
    if ($release) {
        $fixture = New-ProofFixture (Join-Path $root.FullName 'clean') $source $feed
        [void](Install-ProofFixture $fixture $feed $next 'clean-exact')
        Assert-PackageExecutable $fixture $release
        $seedInstall = $fixture.Install
        Remove-ProofFixture $fixture
        $fixture = $null
    }

    $fixture = New-ProofFixture $root.FullName $source $feed
    $baselineSetup = Join-Path $root.FullName 'baseline-setup'
    [void][IO.Directory]::CreateDirectory($baselineSetup)
    $buildFeed = if ($release) { Join-Path $root.FullName 'baseline-feed' } else { $feed }
    $feedSetup = Join-Path $buildFeed "sidenote-$channel-Setup.exe"
    $versions = if ($release) { @($n) } else { @($n, $next) }
    $actions = $env:GITHUB_ACTIONS
    Push-Location -LiteralPath $source
    try {
        $env:GITHUB_ACTIONS = $null # Disable ANSI in the pinned vpk help captured by the packer.
        & cargo +stable-x86_64-pc-windows-gnu fetch --locked --target x86_64-pc-windows-gnu
        if ($LASTEXITCODE) { throw 'Could not fetch the locked Windows GNU dependencies.' }
        foreach ($version in $versions) {
            Set-SourceVersion $source $version
            & (Join-Path $source 'tools/pack_release.ps1') -Feed $buildFeed -Channel $channel
            if ($version -ceq $n) { Copy-Item -LiteralPath $feedSetup -Destination $baselineSetup }
        }
    } finally {
        Pop-Location
        $env:GITHUB_ACTIONS = $actions
    }
    if (-not $release) {
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
    }

    if ($release) {
        Invoke-SetupHandoffCases $root.FullName $source $feed $next $seedInstall $release
        $installed = Install-ProofFixture $fixture $baselineSetup $n 'clean-baseline'
    } else {
        [void](Install-ProofFixture $fixture $baselineSetup $n 'clean-baseline')
        Remove-ProofFixture $fixture
        Invoke-SetupHandoffCases $root.FullName $source $feed $next $fixture.Install $null
        $installed = Start-ProofFixture $fixture $n
    }
    $listener = $installed.Listener
    $initialPointer = $installed.Pointer
    $reportPath = Join-Path $fixture.Updates 'update-report.json'

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
    if ($release) {
        $download = Join-Path $fixture.Install "packages/$($release.Full.Name)"
        if ($ready[0] -cne $channel -or $ready[1].FileName -cne $release.Full.Name -or
            (Get-FileHash -LiteralPath $release.Full.FullName).Hash -ine $hash -or
            (Get-FileHash -LiteralPath $download).Hash -ine $hash) {
            throw 'The ready update does not match the verified full package.'
        }
        Write-Output "Downloaded update matches $($release.Full.Name) ($hash)."
    }
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
    if ($release) { Assert-PackageExecutable $fixture $release }
    Write-Output "Verified $next, the restarted listener (PID $($newListener.ProcessId)), preserved fixtures and bundled runtime and notices."
} finally {
    if ($fixture) { Remove-ProofFixture $fixture }
    $root.Delete($true)
}
