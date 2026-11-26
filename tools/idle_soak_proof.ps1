$ErrorActionPreference='Stop'
$soak=Join-Path $PSScriptRoot 'idle_soak.ps1'
. $soak
$proofRoot=Join-Path ([IO.Path]::GetTempPath()) ('sidenote-synthetic-idle-proof-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $proofRoot | Out-Null
function Assert-Proof([bool]$Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
function New-Fixture([string]$Path,[string]$EndState='completed',[int]$Hours=24,[double[]]$MissingCpuHours=@(),[double[]]$MissingResourceHours=@(),[double]$MemoryGrowth=0,[double]$HandleGrowth=0) {
    $cpu=@(); $resources=@()
    foreach ($hour in (@(0.0,0.5)+@(1..$Hours|ForEach-Object {$_+0.5}))) {
        $cpuMissing=$MissingCpuHours -contains $hour
        $cpuTotal=3600.0*[math]::Min($hour/0.5,1.0)+9.0*[math]::Max(0.0,$hour-0.5)
        $cpu+=[pscustomobject]@{elapsed_seconds=$hour*3600.0;total_cpu_seconds=$(if ($cpuMissing) {$null} else {$cpuTotal});missing=$cpuMissing}
        $resourceMissing=$MissingResourceHours -contains $hour
        $resources+=[pscustomobject]@{elapsed_seconds=$hour*3600.0;working_set_bytes=$(if ($resourceMissing) {$null} else {200MB+$MemoryGrowth*$hour});private_bytes=$(if ($resourceMissing) {$null} else {150MB+$MemoryGrowth*$hour});handle_count=$(if ($resourceMissing) {$null} else {300+$HandleGrowth*$hour});missing=$resourceMissing}
    }
    [pscustomobject]@{schema_version=1;end_state=$EndState;cpu_samples=$cpu;resource_samples=$resources}|ConvertTo-Json -Depth 6|Set-Content -LiteralPath $Path -Encoding UTF8
}
function Invoke-ProofCase([string]$Name,[string]$Fixture,[switch]$NoApprove,[switch]$Prepopulate) {
    $caseRoot=Join-Path $proofRoot ($Name+'-case')
    if ($Prepopulate) { New-Item -ItemType Directory -Path $caseRoot | Out-Null; Set-Content -LiteralPath (Join-Path $caseRoot 'already.txt') -Value 'not empty' }
    $engine=Join-Path $PSHOME 'pwsh.exe'; if (-not (Test-Path -LiteralPath $engine)) { $engine=Join-Path $PSHOME 'powershell.exe' }
    $arguments=@('-NoProfile','-File',$soak,'-FixtureSamplesPath',$Fixture,'-SyntheticDataRoot',$caseRoot,'-CandidateExePath',$candidate)
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
Assert-CaseOutput (Invoke-ProofCase 'nonempty-root' $passFixture -Prepopulate) 2 'nonempty'
Assert-CaseOutput (Invoke-ProofCase 'unapproved-root' $passFixture -NoApprove) 2 'not approved'
$hexRefused=$false
try { Assert-IdleSoakNoLiveHex @([pscustomobject]@{Name='hex.exe'}) } catch { $hexRefused=$_.Exception.Message.Contains('live Hex') }
Assert-Proof $hexRefused 'a fake live Hex process was not refused'
Write-Output 'CASE live-hex: refused before sampling'
$installRoot=Join-Path $proofRoot 'synthetic-install-root'
$installed=Join-Path $installRoot 'Sidenote\sidenote.exe'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $installed) | Out-Null
[IO.File]::WriteAllText($installed,'installed fixture bytes')
$installRefused=$false
try { Get-IdleSoakCandidate $installed @($installRoot)|Out-Null } catch { $installRefused=$_.Exception.Message.Contains('real installation') }
Assert-Proof $installRefused 'an installed candidate path was not refused'
Write-Output 'CASE installed-candidate: refused before sampling'
Write-Output 'PASS: passing fixture plus 8 negative cases (timeout, crash, missing samples, upward trend, nonempty root, unapproved root, live Hex, installed candidate) behaved as required.'
Write-Output "Evidence: $proofRoot"
