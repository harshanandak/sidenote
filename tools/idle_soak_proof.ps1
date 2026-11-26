$ErrorActionPreference='Stop'
$soak=Join-Path $PSScriptRoot 'idle_soak.ps1'
. $soak
function Get-IdleSoakWin32Imports {
    @'
    public static string ProofCase;
    public static double ProofElapsed;
    public static int ProofLaunches, ProofAssignments, ProofJobCloses;
    private static IntPtr CreateJobObjectW(IntPtr attributes, IntPtr name) { return (IntPtr)1; }
    private static bool SetInformationJobObject(IntPtr job, int kind, ref ExtendedLimits limits, int size) {
        return job == (IntPtr)1 && kind == 9 && limits.Basic.Flags == 0x2000;
    }
    private static bool CreateProcessW(string app, StringBuilder args, IntPtr processAttributes,
        IntPtr threadAttributes, bool inheritHandles, uint flags, IntPtr environment, string directory,
        ref StartupInfo startup, out ProcessInfo info) {
        ProofLaunches++;
        info = new ProcessInfo { Process = (IntPtr)2, Thread = (IntPtr)3, ProcessId = 100 };
        return flags == 4 && !inheritHandles;
    }
    private static bool AssignProcessToJobObject(IntPtr job, IntPtr process) {
        ProofAssignments++;
        return job == (IntPtr)1 && process == (IntPtr)2;
    }
    private static uint ResumeThread(IntPtr thread) { return ProofAssignments == 1 && thread == (IntPtr)3 ? 1U : uint.MaxValue; }
    private static uint WaitForSingleObject(IntPtr handle, uint milliseconds) {
        return ProofCase == "warmup-crash" && ProofElapsed >= 4 ? 0U : 258U;
    }
    private static bool QueryInformationJobObject(IntPtr job, int kind, out Accounting info, int size, IntPtr returned) {
        // PID 101 ran between t=25 and t=26. The OS returns its final user and kernel ticks in the job totals.
        info = new Accounting();
        if (ProofCase == "short-lived-child" && ProofElapsed >= 26) { info.UserTime = 7500000; info.KernelTime = 12500000; }
        return job == (IntPtr)1 && kind == 1 && (ProofCase != "final-cpu-query" || ProofElapsed < 112);
    }
    private static bool TerminateProcess(IntPtr process, uint code) { return true; }
    private static bool CloseHandle(IntPtr handle) { if (handle == (IntPtr)1) ProofJobCloses++; return true; }
'@
}
Initialize-IdleSoakJobType
$proofRoot=Join-Path ([IO.Path]::GetTempPath()) ('sidenote-synthetic-idle-proof-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $proofRoot | Out-Null
function Assert-Proof([bool]$Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
function New-Fixture([string]$Path,[string]$EndState='completed',[int]$Hours=24,[double[]]$MissingCpuHours=@(),[double[]]$MissingResourceHours=@(),[double]$MemoryGrowth=0,[double]$HandleGrowth=0,[double]$CpuSecondsPerHour=9) {
    $cpu=@(); $resources=@()
    foreach ($hour in (@(0.0,0.5)+@(1..$Hours|ForEach-Object {$_+0.5}))) {
        $cpuMissing=$MissingCpuHours -contains $hour
        $cpuTotal=3600.0*[math]::Min($hour/0.5,1.0)+$CpuSecondsPerHour*[math]::Max(0.0,$hour-0.5)
        $cpu+=[pscustomobject]@{elapsed_seconds=$hour*3600.0;total_cpu_seconds=$(if ($cpuMissing) {$null} else {$cpuTotal});missing=$cpuMissing}
        $resourceMissing=$MissingResourceHours -contains $hour
        $resources+=[pscustomobject]@{elapsed_seconds=$hour*3600.0;working_set_bytes=$(if ($resourceMissing) {$null} else {200MB+$MemoryGrowth*$hour});private_bytes=$(if ($resourceMissing) {$null} else {150MB+$MemoryGrowth*$hour});handle_count=$(if ($resourceMissing) {$null} else {300+$HandleGrowth*$hour});missing=$resourceMissing}
    }
    [pscustomobject]@{schema_version=1;end_state=$EndState;cpu_samples=$cpu;resource_samples=$resources}|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $Path -Encoding UTF8
}
function Invoke-ProofCase([string]$Name,[string]$Fixture,[switch]$NoApprove,[switch]$Prepopulate,[string]$CandidatePath=$candidate) {
    $caseRoot=Join-Path $proofRoot ($Name+'-case')
    if ($Prepopulate) { New-Item -ItemType Directory -Path $caseRoot | Out-Null; Set-Content -LiteralPath (Join-Path $caseRoot 'already.txt') -Value 'not empty' }
    $engine=(Get-Process -Id $PID).Path
    $arguments=@('-NoProfile','-File',$soak,'-FixtureSamplesPath',$Fixture,'-SyntheticDataRoot',$caseRoot,'-CandidateExePath',$CandidatePath)
    if (-not $NoApprove) { $arguments+='-ApproveSyntheticDataRoot' }
    $output=& $engine @arguments 2>&1|Out-String
    [pscustomobject]@{name=$Name;root=$caseRoot;exit_code=$LASTEXITCODE;output=$output}
}
function Assert-CaseOutput($Case,[int]$ExitCode,[string]$Text) {
    Assert-Proof ($Case.exit_code -eq $ExitCode) "$($Case.name) exited $($Case.exit_code), expected $ExitCode. Output: $($Case.output)"
    Assert-Proof ($Case.output.IndexOf($Text,[StringComparison]::OrdinalIgnoreCase) -ge 0) "$($Case.name) did not report $Text. Output: $($Case.output)"
    Write-Output "CASE $($Case.name): exit=$($Case.exit_code); reason=$Text"
}
$candidateFolder=Join-Path $proofRoot 'synthetic-candidate'
New-Item -ItemType Directory -Path $candidateFolder | Out-Null
$candidate=Join-Path $candidateFolder 'sidenote.exe'
[IO.File]::WriteAllText($candidate,'synthetic candidate bytes')
$fixtures=Join-Path $proofRoot 'fixtures'
New-Item -ItemType Directory -Path $fixtures | Out-Null
$passFixture=Join-Path $fixtures 'pass.json'
New-Fixture $passFixture
$pass=Invoke-ProofCase 'pass' $passFixture
Assert-Proof ($pass.exit_code -eq 0) "passing fixture failed: $($pass.output)"
Assert-Proof ($pass.output.Contains('PASS: idle soak')) "passing fixture missing summary: $($pass.output)"
$reportPath=Join-Path $pass.root 'idle-soak-report.json'
$markdownPath=Join-Path $pass.root 'idle-soak-report.md'
Assert-Proof ((Test-Path -LiteralPath $reportPath) -and (Test-Path -LiteralPath $markdownPath)) 'final report files were not written'
$report=Get-Content -LiteralPath $reportPath -Raw|ConvertFrom-Json
Assert-Proof ($report.passed -and $report.sample_source -eq 'fixture') 'passing report did not record a fixture pass'
Assert-Proof ([math]::Abs([double]$report.metrics.average_cpu_percent_of_one_core-0.25) -lt 0.01) "warm-up CPU was not excluded: $($report.metrics.average_cpu_percent_of_one_core)"
Assert-Proof ($report.candidate.sha256 -ceq ((Get-FileHash -LiteralPath $candidate).Hash.ToLowerInvariant())) 'candidate exe hash missing or wrong'
Write-Output "CASE pass: exit=0; avg_cpu_percent_of_one_core=$([math]::Round([double]$report.metrics.average_cpu_percent_of_one_core,3)); reports=json+markdown"
foreach ($case in @(
    @{name='timeout';text='timeout';fixture='timeout.json';hours=12},
    @{name='crash';text='crash';fixture='crash.json';hours=12}
)) {
    $path=Join-Path $fixtures $case.fixture
    New-Fixture $path -EndState $case.name -Hours $case.hours
    Assert-CaseOutput (Invoke-ProofCase $case.name $path) 1 $case.text
}
$missingFixture=Join-Path $fixtures 'missing.json'
New-Fixture $missingFixture -MissingCpuHours @(10.5,11.5,12.5) -MissingResourceHours @(10.5,11.5,12.5)
Assert-CaseOutput (Invoke-ProofCase 'missing' $missingFixture) 1 'missing samples'
$trendFixture=Join-Path $fixtures 'trend.json'
New-Fixture $trendFixture -MemoryGrowth (10MB) -HandleGrowth 10
Assert-CaseOutput (Invoke-ProofCase 'trend' $trendFixture) 1 'upward trend'
$busyFixture=Join-Path $fixtures 'above-one-percent.json'
New-Fixture $busyFixture -CpuSecondsPerHour 45
Assert-CaseOutput (Invoke-ProofCase 'above-one-percent' $busyFixture) 1 'average idle cpu exceeds one percent'
foreach ($kind in @('cpu','resource')) {
    $path=Join-Path $fixtures "missing-final-$kind.json"
    if ($kind -eq 'cpu') { New-Fixture $path -MissingCpuHours @(24.5) }
    else { New-Fixture $path -MissingResourceHours @(24.5) }
    Assert-CaseOutput (Invoke-ProofCase "missing-final-$kind" $path) 1 "final $kind samples do not cover"
}
$finalFixture=Join-Path $fixtures 'final-hour-growth.json'
New-Fixture $finalFixture
$final=Get-Content -LiteralPath $finalFixture -Raw|ConvertFrom-Json
$finalResource=$final.resource_samples[-1]
$final.resource_samples[-1]=[pscustomobject]@{elapsed_seconds=$finalResource.elapsed_seconds;missing=$true}
$finalResource.elapsed_seconds+=3601
$finalResource.working_set_bytes+=100MB
$finalResource.private_bytes+=100MB
$final.resource_samples+=@($finalResource)
$final|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $finalFixture -Encoding UTF8
Assert-CaseOutput (Invoke-ProofCase 'final-hour-growth' $finalFixture) 1 'final growth'
$shortFixture=Join-Path $fixtures 'short-23h30m.json'
New-Fixture $shortFixture
$short=Get-Content -LiteralPath $shortFixture -Raw|ConvertFrom-Json
$short.cpu_samples=@($short.cpu_samples|Where-Object {[double]$_.elapsed_seconds -lt 88200})
$short.resource_samples=@($short.resource_samples|Where-Object {[double]$_.elapsed_seconds -lt 88200})
$short|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $shortFixture -Encoding UTF8
Assert-CaseOutput (Invoke-ProofCase 'short-23h30m' $shortFixture) 1 'do not cover'
$deadlines=@(Get-IdleSoakSkippedDeadlines 1800 12601 3600)
Assert-Proof (($deadlines -join ',') -eq '1800,5400,9000') 'three skipped hourly deadlines were not reconstructed'
$gapFixture=Join-Path $fixtures 'deadline-gap.json'
New-Fixture $gapFixture -MissingCpuHours @(0.5,1.5,2.5) -MissingResourceHours @(0.5,1.5,2.5)
Assert-CaseOutput (Invoke-ProofCase 'deadline-gap' $gapFixture) 1 'missing samples'
Assert-CaseOutput (Invoke-ProofCase 'nonempty-root' $passFixture -Prepopulate) 2 'nonempty'
Assert-CaseOutput (Invoke-ProofCase 'unapproved-root' $passFixture -NoApprove) 2 'not approved'
$hexRefused=$false
try { Assert-IdleSoakNoLiveHex @([pscustomobject]@{Name='hex.exe'}) } catch { $hexRefused=$_.Exception.Message.Contains('live Hex') }
Assert-Proof $hexRefused 'a fake live Hex process was not refused'
Write-Output 'CASE live-hex: refused before sampling'
function Invoke-NativeProof([string]$Name) {
    $SyntheticDataRoot=Join-Path $proofRoot ($Name+'-case')
    $ApproveSyntheticDataRoot=$true
    $CandidateExePath=$candidate
    $DurationHours=100/3600.0
    $WarmupMinutes=12/60.0
    $CpuSampleSeconds=10.0
    $ResourceSampleSeconds=10.0
    $LaunchCandidate=$Name -ne 'untracked-root'
    $RootProcessId=if ($LaunchCandidate) {0} else {100}
    [IdleSoakJob]::ProofCase=$Name
    [IdleSoakJob]::ProofElapsed=-2.0
    [IdleSoakJob]::ProofLaunches=[IdleSoakJob]::ProofAssignments=[IdleSoakJob]::ProofJobCloses=0
    function Get-CimInstance {
        if ($Name -eq 'final-resource-query' -and [IdleSoakJob]::ProofElapsed -ge 112) { throw 'resource query failed' }
        if ($Name -eq 'entry-live-hex') { [pscustomobject]@{Name='hex.exe'} }
        else { [pscustomobject]@{ProcessId=100;ParentProcessId=99;KernelModeTime=0;UserModeTime=0;ExecutablePath=$candidate} }
    }
    function Start-Sleep {}
    function Get-IdleSoakElapsedSeconds { [IdleSoakJob]::ProofElapsed+=2.0; [IdleSoakJob]::ProofElapsed }
    function Get-Process { [pscustomobject]@{WorkingSet64=200MB;PrivateMemorySize64=150MB;HandleCount=300} }
    if ($Name -eq 'entry-live-hex') {
        $reason=try { Invoke-IdleSoak|Out-Null } catch { $_.Exception.Message }
        Assert-Proof ($reason -like 'a live Hex process is present*' -and [IdleSoakJob]::ProofLaunches -eq 0) 'the native entry did not refuse live Hex before launch'
        Write-Output 'CASE entry-live-hex: refused before sampling'
        return
    }
    if ($Name -in @('untracked-root','brokered-work')) {
        $result=Invoke-IdleSoak
        Assert-Proof ($result.end_state -eq 'unobservable' -and [IdleSoakJob]::ProofLaunches -eq 0) "$Name did not refuse before launch"
    } else {
        $root=Assert-IdleSoakSyntheticRoot $SyntheticDataRoot $true
        $candidateInfo=Get-IdleSoakCandidate $candidate
        $job=[IdleSoakJob]::new($candidateInfo.path,$candidateInfo.path,$root)
        try { $samples=Invoke-IdleSoakSampler $job 112 12 $CpuSampleSeconds $ResourceSampleSeconds 113 }
        finally { $job.Dispose() }
        Assert-Proof ([IdleSoakJob]::ProofLaunches -eq 1 -and [IdleSoakJob]::ProofAssignments -eq 1 -and [IdleSoakJob]::ProofJobCloses -eq 1) "$Name did not bind and close the job"
        $result=Complete-IdleSoakReport $samples $candidateInfo $root 'process-tree' 112 12 113 ''
    }
    Assert-Proof (-not $result.passed) "$Name did not produce a failed collection"
    Assert-Proof ((Test-Path -LiteralPath $result.report_json_path) -and (Test-Path -LiteralPath $result.report_markdown_path)) "$Name did not write both reports"
    $report=Get-Content -LiteralPath $result.report_json_path -Raw|ConvertFrom-Json
    $reason=switch ($Name) {
        'warmup-crash' { Assert-Proof ($report.end_state -eq 'crash' -and $report.resource_samples.Count -eq 0) 'expected a crash before resource sampling'; 'crash' }
        'short-lived-child' { Assert-Proof ($report.cpu_samples[-1].total_cpu_seconds -eq 2.0) 'short-lived child CPU was dropped'; 'average idle cpu exceeds one percent' }
        'untracked-root' { 'process-tree cpu is unobservable: an existing process may already have untracked descendants' }
        'brokered-work' { 'process-tree cpu is unobservable: broker-created work outside the Job Object' }
        'final-cpu-query' { 'final cpu samples do not cover' }
        'final-resource-query' { 'final resource samples do not cover' }
    }
    Assert-Proof (($report.failure_reasons -join ' ') -like "*$reason*") "$Name did not report $reason"
    Write-Output "CASE ${Name}: $reason"
}
foreach ($name in @('entry-live-hex','untracked-root','brokered-work','warmup-crash','short-lived-child','final-cpu-query','final-resource-query')) {
    Invoke-NativeProof $name
}
$installRoot=Join-Path $proofRoot 'synthetic-install-root'
$installed=Join-Path (Join-Path $installRoot 'Sidenote') 'sidenote.exe'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $installed) | Out-Null
[IO.File]::WriteAllText($installed,'installed fixture bytes')
$installRefused=$false
try { Get-IdleSoakCandidate $installed @($installRoot)|Out-Null } catch { $installRefused=$_.Exception.Message.Contains('real installation') }
Assert-Proof $installRefused 'an installed candidate path was not refused'
Write-Output 'CASE installed-candidate: refused before sampling'
$defaultRoots=@(
    (Join-Path $proofRoot 'program-files/Sidenote/sidenote.exe'),
    (Join-Path $proofRoot 'program-files-x86/Sidenote/sidenote.exe'),
    (Join-Path $proofRoot 'local-app-data/Programs/Sidenote/sidenote.exe'),
    (Join-Path $proofRoot 'local-app-data/Sidenote/sidenote.exe')
)
if ($IsLinux) {
    $defaultRoots+=@((Join-Path $proofRoot '.local/bin/sidenote'),'/usr/local/libexec/sidenote/sidenote')
}
$installEnvironment=@{
    HOME=$proofRoot
    ProgramFiles=(Join-Path $proofRoot 'program-files')
    'ProgramFiles(x86)'=(Join-Path $proofRoot 'program-files-x86')
    LOCALAPPDATA=(Join-Path $proofRoot 'local-app-data')
}
$previousInstallEnvironment=@{}
try {
    foreach ($name in $installEnvironment.Keys) {
        $previousInstallEnvironment[$name]=[Environment]::GetEnvironmentVariable($name)
        [Environment]::SetEnvironmentVariable($name,$installEnvironment[$name])
    }
    for ($index=0; $index -lt $defaultRoots.Count; $index++) {
        Assert-CaseOutput (Invoke-ProofCase "default-install-$index" $passFixture -CandidatePath $defaultRoots[$index]) 2 'real installation'
    }
} finally {
    foreach ($name in $previousInstallEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name,$previousInstallEnvironment[$name]) }
}
Write-Output 'PASS: passing fixture and all negative cases behaved as required.'
Write-Output "Evidence: $proofRoot"
exit 0
