[CmdletBinding()]
param([int]$RootProcessId=0,[string]$SyntheticDataRoot='',[string]$CandidateExePath='',[switch]$ApproveSyntheticDataRoot,[double]$DurationHours=24,[double]$WarmupMinutes=30,[double]$CpuSampleSeconds=60,[double]$ResourceSampleSeconds=3600,[double]$TimeoutMinutes=0,[string]$FixtureSamplesPath='')
$ErrorActionPreference='Stop'
$script:IdleSoakThresholds=[ordered]@{
    average_cpu_percent_of_one_core=1.0; missing_sample_percent=5.0; consecutive_missing_samples=2
    working_set_slope_bytes_per_hour=4MB; private_bytes_slope_bytes_per_hour=4MB; handle_count_slope_per_hour=4.0
    working_set_final_growth_bytes=64MB; private_bytes_final_growth_bytes=64MB; handle_count_final_growth=64.0
}
function Get-IdleSoakFullPath([string]$Path) { [IO.Path]::GetFullPath($Path).TrimEnd('\') }
function Test-IdleSoakPathInside([string]$Path,[string]$Root) {
    $p=(Get-IdleSoakFullPath $Path)+'\'; $r=(Get-IdleSoakFullPath $Root)+'\'
    $p.StartsWith($r,[StringComparison]::OrdinalIgnoreCase)
}
function Assert-IdleSoakSyntheticRoot([string]$Path,[bool]$Approved) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw 'an explicit synthetic data root is required' }
    if (-not $Approved) { throw 'the synthetic data root is not approved' }
    if (-not [IO.Path]::IsPathRooted($Path)) { throw 'the synthetic data root must be an absolute path' }
    $full=Get-IdleSoakFullPath $Path
    if ($full.Length -le 3) { throw 'a drive root is not an approved synthetic data root' }
    if ($env:APPDATA) {
        foreach ($name in @('sidenote','sidenote-peers','hex','hex\meetings','hex-meetings','hex-meetings-peers')) {
            $protected=Join-Path $env:APPDATA $name
            if ((Test-IdleSoakPathInside $full $protected) -or (Test-IdleSoakPathInside $protected $full)) { throw 'the synthetic data root overlaps a real application data path' }
        }
    }
    foreach ($name in @('SIDENOTE_DATA_DIR','SIDENOTE_STATE_DIR','SIDENOTE_HEX_DIR','SIDENOTE_PEERS_DIR','SIDENOTE_CAPTURES_DIR')) {
        $value=[Environment]::GetEnvironmentVariable($name)
        if ($value -and ((Get-IdleSoakFullPath $value) -ieq $full)) { throw "the synthetic data root is exactly the live $name path" }
    }
    if (Test-Path -LiteralPath $full) {
        $item=Get-Item -LiteralPath $full -Force
        if (-not $item.PSIsContainer) { throw 'the synthetic data root is not a directory' }
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'the synthetic data root must not be a link' }
        if (@(Get-ChildItem -LiteralPath $full -Force | Select-Object -First 1).Count) { throw 'the synthetic data root is nonempty' }
    } else { New-Item -ItemType Directory -Force -Path $full | Out-Null }
    $full
}
function Get-IdleSoakCandidate([string]$Path,[string[]]$InstallRoots=@()) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw 'a candidate executable path is required' }
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw 'the candidate executable does not exist' }
    $full=(Get-Item -LiteralPath $Path).FullName
    if (-not $InstallRoots.Count) {
        $InstallRoots=@($env:ProgramFiles,[Environment]::GetEnvironmentVariable('ProgramFiles(x86)')) | Where-Object { $_ }
        if ($env:LOCALAPPDATA) { $InstallRoots+=@((Join-Path $env:LOCALAPPDATA 'Programs'),(Join-Path $env:LOCALAPPDATA 'Sidenote')) }
    }
    foreach ($root in $InstallRoots) { if ($root -and (Test-IdleSoakPathInside $full $root)) { throw 'the candidate executable is in a real installation location' } }
    [pscustomobject]@{path=$full;sha256=(Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash.ToLowerInvariant()}
}
function Assert-IdleSoakNoLiveHex([object[]]$Processes) {
    if (@($Processes|Where-Object {$_.Name -in @('hex.exe','hex.com')}).Count) { throw 'a live Hex process is present; the idle soak requires a synthetic environment without live Hex' }
}
function Get-IdleSoakProcessTree([int]$RootProcessId) {
    $rows=@(Get-CimInstance Win32_Process -ErrorAction Stop)
    $root=@($rows|Where-Object {$_.ProcessId -eq $RootProcessId}|Select-Object -First 1)
    if (-not $root.Count) { return $null }
    $tree=@($root[0]); $parents=@($RootProcessId)
    while ($parents.Count) {
        $children=@($rows|Where-Object {$parents -contains $_.ParentProcessId -and $_.ProcessId -ne $RootProcessId})
        if (-not $children.Count) { break }
        $tree+=$children; $parents=@($children|ForEach-Object {$_.ProcessId})
    }
    $tree
}
function Read-IdleSoakResourceSample($Tree) {
    $missing=$false; $workingSet=0.0; $privateBytes=0.0; $handles=0.0
    foreach ($row in $Tree) {
        try {
            $process=Get-Process -Id $row.ProcessId -ErrorAction Stop
            $workingSet+=[double]$process.WorkingSet64; $privateBytes+=[double]$process.PrivateMemorySize64; $handles+=[double]$process.HandleCount
        } catch { $missing=$true }
    }
    [pscustomobject]@{working_set_bytes=$workingSet;private_bytes=$privateBytes;handle_count=$handles;missing=$missing}
}
function Invoke-IdleSoakSampler([int]$RootProcessId,[string]$CandidatePath,[double]$DurationSeconds,[double]$CpuInterval,[double]$ResourceInterval,[double]$TimeoutSeconds) {
    $firstTree=Get-IdleSoakProcessTree $RootProcessId
    if (-not $firstTree) { throw 'the root process is not running' }
    if (-not $firstTree[0].ExecutablePath) { throw 'the root process executable path is not readable' }
    if ((Get-IdleSoakFullPath $firstTree[0].ExecutablePath) -ine (Get-IdleSoakFullPath $CandidatePath)) { throw 'the root process executable does not match the candidate executable' }
    $cpuByKey=@{}; $cpuSamples=@(); $resourceSamples=@(); $nextCpu=0.0; $nextResource=0.0
    $clock=[Diagnostics.Stopwatch]::StartNew(); $endState='timeout'
    while ($true) {
        $elapsed=$clock.Elapsed.TotalSeconds
        if ($elapsed -gt $TimeoutSeconds) { break }
        $queryFailed=$false
        try { $tree=Get-IdleSoakProcessTree $RootProcessId } catch { $queryFailed=$true; $tree=$null }
        if (-not $tree -and -not $queryFailed) { $endState='crash'; break }
        if ($elapsed -ge $nextCpu) {
            if ($tree) {
                foreach ($row in $tree) {
                    $key='{0}:{1}' -f $row.ProcessId,$row.CreationDate
                    $cpuByKey[$key]=([double]$row.KernelModeTime+[double]$row.UserModeTime)/10000000.0
                }
                $totalCpu=0.0; foreach ($value in $cpuByKey.Values) { $totalCpu+=$value }
                $cpuSamples+=[pscustomobject]@{elapsed_seconds=$elapsed;total_cpu_seconds=$totalCpu;missing=$false}
            } else { $cpuSamples+=[pscustomobject]@{elapsed_seconds=$elapsed;total_cpu_seconds=$null;missing=$true} }
            $nextCpu=$elapsed+$CpuInterval
        }
        if ($elapsed -ge $nextResource) {
            if ($tree) {
                $sample=Read-IdleSoakResourceSample $tree
                $sample|Add-Member -NotePropertyName elapsed_seconds -NotePropertyValue $elapsed -Force
                $resourceSamples+=$sample
            } else { $resourceSamples+=[pscustomobject]@{elapsed_seconds=$elapsed;working_set_bytes=$null;private_bytes=$null;handle_count=$null;missing=$true} }
            $nextResource=$elapsed+$ResourceInterval
        }
        if ($elapsed -ge $DurationSeconds) { $endState='completed'; break }
        $nextWake=[math]::Min([math]::Min($nextCpu,$nextResource),$DurationSeconds)
        Start-Sleep -Milliseconds ([math]::Min(1000,[math]::Ceiling([math]::Max(0.05,$nextWake-$elapsed)*1000)))
    }
    [pscustomobject]@{end_state=$endState;cpu_samples=@($cpuSamples);resource_samples=@($resourceSamples)}
}
function Read-IdleSoakFixture([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw 'the fixture sample file does not exist' }
    $fixture=Get-Content -LiteralPath $Path -Raw|ConvertFrom-Json
    if ([int]$fixture.schema_version -ne 1) { throw 'fixture sample schema_version must be 1' }
    if ($fixture.end_state -notin @('completed','timeout','crash')) { throw 'fixture end_state must be completed, timeout or crash' }
    [pscustomobject]@{end_state=$fixture.end_state;cpu_samples=@($fixture.cpu_samples);resource_samples=@($fixture.resource_samples)}
}
function Assert-IdleSoakSamples($CpuSamples,$ResourceSamples) {
    if (-not @($CpuSamples).Count -or -not @($ResourceSamples).Count) { throw 'cpu and resource sample series must both be nonempty' }
    $lastElapsed=-1.0; $lastCpu=$null
    foreach ($sample in @($CpuSamples)) {
        if ($null -eq $sample.elapsed_seconds) { throw 'every cpu sample needs elapsed_seconds' }
        $elapsed=[double]$sample.elapsed_seconds
        if ($elapsed -lt $lastElapsed) { throw 'cpu samples must use monotonic elapsed seconds' }
        $lastElapsed=$elapsed
        if ([bool]$sample.missing) { continue }
        if ($null -eq $sample.total_cpu_seconds) { throw 'a present cpu sample needs total_cpu_seconds' }
        $cpu=[double]$sample.total_cpu_seconds
        if ($cpu -lt 0 -or ($null -ne $lastCpu -and $cpu -lt $lastCpu)) { throw 'process-tree cpu time must be nonnegative and monotonic' }
        $lastCpu=$cpu
    }
    $lastElapsed=-1.0
    foreach ($sample in @($ResourceSamples)) {
        if ($null -eq $sample.elapsed_seconds) { throw 'every resource sample needs elapsed_seconds' }
        $elapsed=[double]$sample.elapsed_seconds
        if ($elapsed -lt $lastElapsed) { throw 'resource samples must use monotonic elapsed seconds' }
        $lastElapsed=$elapsed
        if ([bool]$sample.missing) { continue }
        foreach ($name in @('working_set_bytes','private_bytes','handle_count')) {
            if ($null -eq $sample.$name -or [double]$sample.$name -lt 0) { throw "a present resource sample needs nonnegative $name" }
        }
    }
}
function Get-IdleSoakMissingSummary($Samples) {
    $total=@($Samples).Count; $missing=0; $current=0; $maximum=0
    foreach ($sample in @($Samples)) {
        if ([bool]$sample.missing) { $missing++; $current++; $maximum=[math]::Max($maximum,$current) } else { $current=0 }
    }
    [pscustomobject]@{total=$total;missing=$missing;percent=$(if ($total) {100.0*$missing/$total} else {100.0});max_consecutive=$maximum}
}
function Get-IdleSoakLinearSlope($Points,[string]$Property) {
    $xMean=0.0; $yMean=0.0
    foreach ($point in $Points) {
        $xMean+=[double]$point.elapsed_seconds/3600.0/$Points.Count
        $yMean+=[double]$point.PSObject.Properties[$Property].Value/$Points.Count
    }
    $numerator=0.0; $denominator=0.0
    foreach ($point in $Points) {
        $x=[double]$point.elapsed_seconds/3600.0-$xMean
        $numerator+=$x*([double]$point.PSObject.Properties[$Property].Value-$yMean); $denominator+=$x*$x
    }
    if ($denominator -eq 0) {0.0} else {$numerator/$denominator}
}
function Measure-IdleSoakTrend($Samples,[string]$Property,[double]$SlopeLimit,[double]$FinalLimit,[double]$WarmupSeconds,[double]$DurationSeconds) {
    $points=@($Samples|Where-Object {-not [bool]$_.missing -and [double]$_.elapsed_seconds -ge $WarmupSeconds -and [double]$_.elapsed_seconds -le $DurationSeconds})
    if ($points.Count -lt 3) { return [pscustomobject]@{samples=$points.Count;slope_per_hour=$null;final_delta=$null;positive_step_fraction=$null;insufficient=$true;trend_detected=$false;final_growth_exceeded=$false} }
    $slope=Get-IdleSoakLinearSlope $points $Property
    $first=[double]$points[0].PSObject.Properties[$Property].Value; $last=[double]$points[-1].PSObject.Properties[$Property].Value
    $positive=0
    for ($index=1; $index -lt $points.Count; $index++) { if ([double]$points[$index].PSObject.Properties[$Property].Value -ge [double]$points[$index-1].PSObject.Properties[$Property].Value) { $positive++ } }
    $positiveFraction=$positive/($points.Count-1)
    [pscustomobject]@{samples=$points.Count;slope_per_hour=$slope;final_delta=$last-$first;positive_step_fraction=$positiveFraction;insufficient=$false;trend_detected=($slope -gt $SlopeLimit -and $positiveFraction -ge 0.6);final_growth_exceeded=(($last-$first) -gt $FinalLimit)}
}
function Measure-IdleSoakSeries($CpuSamples,$ResourceSamples,[string]$EndState,[double]$DurationSeconds,[double]$WarmupSeconds,[double]$CpuSampleSeconds) {
    Assert-IdleSoakSamples $CpuSamples $ResourceSamples
    $failures=@()
    if ($EndState -ceq 'crash') { $failures+='the root process tree ended before the requested duration (crash)' }
    elseif ($EndState -ceq 'timeout') { $failures+='the sampler reached its timeout before the requested duration' }
    elseif ($EndState -cne 'completed') { throw "unsupported end state: $EndState" }
    $lastCpuElapsed=[double](@($CpuSamples)[-1].elapsed_seconds)
    if ($lastCpuElapsed -lt ($DurationSeconds-[math]::Max(1.5*$CpuSampleSeconds,1.0))) { $failures+='final cpu samples do not cover the requested duration' }
    $cpuMissing=Get-IdleSoakMissingSummary $CpuSamples; $resourceMissing=Get-IdleSoakMissingSummary $ResourceSamples
    foreach ($entry in @(@('cpu',$cpuMissing),@('resource',$resourceMissing))) {
        if ($entry[1].percent -gt $script:IdleSoakThresholds.missing_sample_percent -or $entry[1].max_consecutive -gt $script:IdleSoakThresholds.consecutive_missing_samples) { $failures+="$($entry[0]) missing samples exceed the pass bar" }
    }
    $cpuPoints=@($CpuSamples|Where-Object {-not [bool]$_.missing -and [double]$_.elapsed_seconds -ge $WarmupSeconds -and [double]$_.elapsed_seconds -le $DurationSeconds})
    $averageCpu=$null
    if ($cpuPoints.Count -lt 2 -or [double]$cpuPoints[-1].elapsed_seconds -le [double]$cpuPoints[0].elapsed_seconds) { $failures+='post-warm-up cpu samples are insufficient' }
    else {
        $averageCpu=100.0*([double]$cpuPoints[-1].total_cpu_seconds-[double]$cpuPoints[0].total_cpu_seconds)/([double]$cpuPoints[-1].elapsed_seconds-[double]$cpuPoints[0].elapsed_seconds)
        if ($averageCpu -gt $script:IdleSoakThresholds.average_cpu_percent_of_one_core) { $failures+='average idle cpu exceeds one percent of one core' }
    }
    $trendKinds=@(
        @('working_set','working_set_bytes',$script:IdleSoakThresholds.working_set_slope_bytes_per_hour,$script:IdleSoakThresholds.working_set_final_growth_bytes),
        @('private_bytes','private_bytes',$script:IdleSoakThresholds.private_bytes_slope_bytes_per_hour,$script:IdleSoakThresholds.private_bytes_final_growth_bytes),
        @('handle_count','handle_count',$script:IdleSoakThresholds.handle_count_slope_per_hour,$script:IdleSoakThresholds.handle_count_final_growth)
    )
    $trends=[ordered]@{}
    foreach ($kind in $trendKinds) {
        $trend=Measure-IdleSoakTrend $ResourceSamples $kind[1] $kind[2] $kind[3] $WarmupSeconds $DurationSeconds
        $trends[$kind[0]]=$trend
        if ($trend.insufficient) { $failures+="$($kind[0]) resource samples are insufficient" }
        if ($trend.trend_detected) { $failures+="$($kind[0]) upward trend exceeds the pass bar" }
        if ($trend.final_growth_exceeded) { $failures+="$($kind[0]) final growth exceeds the pass bar" }
    }
    $passed=-not $failures.Count
    [pscustomobject]@{
        schema_version=1; passed=$passed; result=$(if ($passed) {'pass'} else {'fail'}); end_state=$EndState; failure_reasons=@($failures)
        metrics=[pscustomobject]@{average_cpu_percent_of_one_core=$averageCpu;cpu_missing=$cpuMissing;resource_missing=$resourceMissing;working_set=$trends['working_set'];private_bytes=$trends['private_bytes'];handle_count=$trends['handle_count']}
    }
}
function Write-IdleSoakReport($Result,[string]$Root) {
    $jsonPath=Join-Path $Root 'idle-soak-report.json'; $markdownPath=Join-Path $Root 'idle-soak-report.md'
    $Result|Add-Member -NotePropertyName report_json_path -NotePropertyValue $jsonPath
    $Result|Add-Member -NotePropertyName report_markdown_path -NotePropertyValue $markdownPath
    [IO.File]::WriteAllText($jsonPath,(ConvertTo-Json -InputObject $Result -Depth 9)+"`n")
    $lines=@('# Idle soak report','',"Result: $($Result.result.ToUpperInvariant())","End state: $($Result.end_state)",
        "Average post-warm-up CPU: $($Result.metrics.average_cpu_percent_of_one_core)% of one core",
        "CPU samples missing: $($Result.metrics.cpu_missing.missing)/$($Result.metrics.cpu_missing.total)",
        "Resource samples missing: $($Result.metrics.resource_missing.missing)/$($Result.metrics.resource_missing.total)",
        "Candidate SHA256: $($Result.candidate.sha256)",'','## Failure reasons')
    if ($Result.failure_reasons.Count) { foreach ($reason in $Result.failure_reasons) { $lines+="- $reason" } } else { $lines+='- none' }
    [IO.File]::WriteAllText($markdownPath,($lines -join "`n")+"`n")
}
function Invoke-IdleSoak {
    if ($DurationHours -le 0 -or $CpuSampleSeconds -le 0 -or $ResourceSampleSeconds -le 0) { throw 'duration and sample intervals must be positive' }
    $durationSeconds=$DurationHours*3600.0; $warmupSeconds=$WarmupMinutes*60.0
    if ($warmupSeconds -lt 0 -or $warmupSeconds -ge $durationSeconds) { throw 'warm-up must be shorter than the run' }
    $timeoutSeconds=if ($TimeoutMinutes -gt 0) {$TimeoutMinutes*60.0} else {$durationSeconds+900.0}
    if ($timeoutSeconds -lt $durationSeconds) { throw 'the timeout must not be shorter than the run' }
    $root=Assert-IdleSoakSyntheticRoot $SyntheticDataRoot ([bool]$ApproveSyntheticDataRoot)
    $candidate=Get-IdleSoakCandidate $CandidateExePath
    if ($FixtureSamplesPath) {
        $samples=Read-IdleSoakFixture $FixtureSamplesPath; $source='fixture'
    } else {
        if ($RootProcessId -le 0) { throw 'a running root process id is required unless fixture samples are used' }
        Assert-IdleSoakNoLiveHex @(Get-CimInstance Win32_Process -ErrorAction Stop)
        $samples=Invoke-IdleSoakSampler $RootProcessId $candidate.path $durationSeconds $CpuSampleSeconds $ResourceSampleSeconds $timeoutSeconds
        $source='process-tree'
    }
    $result=Measure-IdleSoakSeries $samples.cpu_samples $samples.resource_samples $samples.end_state $durationSeconds $warmupSeconds $CpuSampleSeconds
    $result|Add-Member -NotePropertyMembers @{generated_at_utc=([DateTimeOffset]::UtcNow.ToString('o'));sample_source=$source;candidate=$candidate;thresholds=([pscustomobject]$script:IdleSoakThresholds);
        configuration=([pscustomobject]@{duration_hours=$DurationHours;warmup_minutes=$WarmupMinutes;cpu_sample_seconds=$CpuSampleSeconds;resource_sample_seconds=$ResourceSampleSeconds;timeout_seconds=$timeoutSeconds});
        cpu_samples=@($samples.cpu_samples);resource_samples=@($samples.resource_samples)}
    Write-IdleSoakReport $result $root
    $result
}
if ($MyInvocation.InvocationName -ne '.') {
    try {
        $result=Invoke-IdleSoak
        Write-Output "$($result.result.ToUpperInvariant()): idle soak end=$($result.end_state); avg_cpu_percent_of_one_core=$($result.metrics.average_cpu_percent_of_one_core); reasons=$(($result.failure_reasons -join '; ')); report=$($result.report_json_path)"
        exit $(if ($result.passed) {0} else {1})
    } catch {
        Write-Output "FAIL: idle-soak input rejected: $($_.Exception.Message)"
        exit 2
    }
}
