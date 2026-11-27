param(
    [string]$Feed = (Join-Path $PSScriptRoot '../artifacts/releases'),
    [string]$DotnetRoot = $env:DOTNET_ROOT,
    [string]$GnuBin = $env:RUST_GNU_BIN,
    [string]$ReleaseNotes = '',
    [ValidateSet('stable', 'dev')] [string]$Channel
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
$feedPath = [IO.Path]::GetFullPath($Feed)
if ($Feed -match '^[a-z]+://' -or -not [IO.Path]::IsPathRooted($Feed)) { throw 'Feed must be an absolute local folder or file share.' }
if ($DotnetRoot) {
    $env:DOTNET_ROOT = $DotnetRoot
    $env:PATH = "$DotnetRoot;$DotnetRoot/tools;$env:PATH"
}
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:PATH = "$env:USERPROFILE/.cargo/bin;$env:PATH"
if ($GnuBin) { $env:PATH = "$GnuBin;$env:PATH" }
if (-not (Get-Command as.exe -ErrorAction SilentlyContinue)) { throw 'Set RUST_GNU_BIN to a full MinGW GNU bin folder, or put it on PATH.' }
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'

function Copy-CpuRuntime([string]$Destination, [string]$Target) {
    # This is the CPU archive used by sherpa-onnx v2.0.1 for Windows x64 Release /MD.
    $name = 'onnxruntime-win-x64-MD-Release-1.28.2'
    $url = "https://github.com/csukuangfj/onnxruntime-libs/releases/download/v1.28.2/$name.tar.bz2"
    $hash = 'cdb3c68e317db991f85504e0f0ce5959528fba2c240b6c22e93366e80b3d2d58'
    $cache = Join-Path $Target 'cpu-runtime'
    New-Item -ItemType Directory -Force -Path $cache | Out-Null
    $archive = Join-Path $cache "$name.tar.bz2"
    if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) {
        $download = "$archive.$PID.part"
        try {
            & curl.exe --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --max-time 120 --output $download $url
            if ($LASTEXITCODE -or (Get-FileHash -LiteralPath $download).Hash -ne $hash) {
                throw 'The pinned ONNX Runtime download failed verification.'
            }
            Move-Item -LiteralPath $download -Destination $archive
        } finally {
            if (Test-Path -LiteralPath $download) { Remove-Item -LiteralPath $download }
        }
    }
    if ((Get-FileHash -LiteralPath $archive).Hash -ne $hash) {
        throw 'The cached ONNX Runtime archive failed verification.'
    }
    $files = @("$name/lib/onnxruntime.dll", "$name/lib/onnxruntime_providers_shared.dll",
        "$name/LICENSE", "$name/ThirdPartyNotices.txt")
    $extract = Join-Path $cache ([guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $extract | Out-Null
    & tar.exe -xf $archive -C $extract @files
    if ($LASTEXITCODE) { throw 'Extracting the pinned ONNX Runtime failed.' }
    $dll = Join-Path $extract $files[0]
    if ((Get-Item -LiteralPath $dll).Length -ne 17136128 -or
        (Get-FileHash -LiteralPath $dll).Hash -ne '422d776ab0e3218260f7f628fcb84606aaa5c21116f720c8619a6da5e0b2e0f9') {
        throw 'The bundled ONNX Runtime DLL does not match the app pin.'
    }
    Copy-Item -LiteralPath $dll -Destination (Join-Path $Destination 'onnxruntime.dll')
    Copy-Item -LiteralPath (Join-Path $extract $files[1]) -Destination (Join-Path $Destination 'onnxruntime_providers_shared.dll')
    Copy-Item -LiteralPath (Join-Path $extract $files[2]) -Destination (Join-Path $Destination 'ONNXRuntime-LICENSE.txt')
    Copy-Item -LiteralPath (Join-Path $extract $files[3]) -Destination (Join-Path $Destination 'ONNXRuntime-ThirdPartyNotices.txt')
}

Push-Location -LiteralPath $root
try {
    $version = [regex]::Match((Get-Content Cargo.toml -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
    if (-not $Channel) { $Channel = if ($version -match '-dev\.') { 'dev' } else { 'stable' } }
    $Channel = $Channel.ToLowerInvariant()
    $versionPattern = if ($Channel -eq 'dev') { '^\d+\.\d+\.\d+-dev\.(0|[1-9]\d*)$' } else { '^\d+\.\d+\.\d+$' }
    if ($version -notmatch $versionPattern) { throw "Cargo.toml version does not match the $Channel release channel." }
    $vpkHelp = & vpk --skip-updates --help
    if ($LASTEXITCODE -ne 0 -or -not ($vpkHelp -match '^\s*Velopack CLI 0\.0\.1298,')) {
        throw 'Velopack CLI 0.0.1298 is required.'
    }
    $features = if ($Channel -eq 'dev') { 'asr-runtime,dev' } else { 'asr-runtime' }
    $metadataJson = & cargo metadata --offline --locked --features $features --filter-platform x86_64-pc-windows-gnu --format-version 1
    if ($LASTEXITCODE) { throw 'Could not resolve locked dependency notices from the Cargo cache.' }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    $stage = "$root/artifacts/updates-core/package-$version-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    . (Join-Path $PSScriptRoot 'dependency_notices.ps1')
    $noticesResult = Write-DependencyNotices -Metadata $metadata -Destination $stage
    if ($noticesResult.uncovered.Count) {
        $uncovered = @($noticesResult.uncovered | ForEach-Object { "$($_.name) $($_.version): $($_.license) ($($_.reason))" })
        throw "Dependency notices are incomplete: $($uncovered -join '; '). Coverage: $stage/DEPENDENCY_NOTICE_COVERAGE.json"
    }
    & cargo +stable-x86_64-pc-windows-gnu build --release --workspace --locked --bins --target x86_64-pc-windows-gnu --features $features
    if ($LASTEXITCODE) { throw 'Release build failed.' }
    New-Item -ItemType Directory -Force -Path $feedPath | Out-Null
    Copy-Item -LiteralPath "$($metadata.target_directory)/x86_64-pc-windows-gnu/release/sidenote.exe" -Destination "$stage/sidenote.exe" -Force
    Copy-Item -LiteralPath "$($metadata.target_directory)/x86_64-pc-windows-gnu/release/sidenote-sync-worker.exe" -Destination "$stage/sidenote-sync-worker.exe" -Force
    Copy-CpuRuntime $stage $metadata.target_directory
    $notices = @('LICENSE', 'THIRD_PARTY.md', 'assets/fonts/Inter-LICENSE.txt',
        'assets/fonts/Fraunces-LICENSE.txt', 'assets/fonts/Figtree-LICENSE.txt',
        'assets/fonts/NotoSansDevanagari-LICENSE.txt', 'assets/fonts/NotoSansTelugu-LICENSE.txt',
        'assets/licenses/webpki-roots-CDLA-Permissive-2.0.txt', 'assets/licenses/lucide-LICENSE.txt',
        'assets/licenses/solar-LICENSE.txt')
    foreach ($notice in $notices) {
        $destination = Join-Path $stage $notice
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
        Copy-Item -LiteralPath (Join-Path $root $notice) -Destination $destination -Force
    }
    $packArgs = @('pack', '--packId', 'sidenote', '--packVersion', $version, '--channel', $Channel, '--packDir', $stage,
        '--mainExe', 'sidenote.exe', '--icon', "$root/assets/icon/meetings.ico", '--packTitle', 'Sidenote', '--outputDir', $feedPath,
        '--shortcuts', 'StartMenuRoot', '--framework', 'vcredist143-x64')
    if ($ReleaseNotes) { $packArgs += @('--releaseNotes', (Resolve-Path -LiteralPath $ReleaseNotes).Path) }
    & vpk --skip-updates --yes @packArgs
    if ($LASTEXITCODE) { throw 'Velopack pack failed.' }
    $index = Join-Path $feedPath "releases.$Channel.json"
    $releases = Get-Content -LiteralPath $index -Raw | ConvertFrom-Json
    $full = @($releases.Assets | Where-Object Type -eq 'Full')
    $ordered = @($full | Sort-Object { (Get-Item -LiteralPath (Join-Path $feedPath $_.FileName)).LastWriteTimeUtc } -Descending)
    $keep = @($ordered | Select-Object -First 3 | ForEach-Object Version)
    $remove = @($releases.Assets | Where-Object { $_.Version -notin $keep })
    foreach ($asset in $remove) {
        if ($asset.FileName -notmatch '^[a-zA-Z0-9._-]+\.nupkg$') { throw 'Invalid package filename in feed.' }
    }
    $releases.Assets = @($releases.Assets | Where-Object { $_.Version -in $keep })
    [IO.File]::WriteAllText("$index.tmp", ($releases | ConvertTo-Json -Depth 20))
    Move-Item -LiteralPath "$index.tmp" -Destination $index -Force
    foreach ($asset in $remove) { Remove-Item -LiteralPath (Join-Path $feedPath $asset.FileName) -ErrorAction SilentlyContinue }
    $legacy = Join-Path $feedPath "RELEASES-$Channel"
    if (Test-Path -LiteralPath $legacy) {
        $names = @($releases.Assets | ForEach-Object FileName)
        $lines = @(Get-Content -LiteralPath $legacy | Where-Object { ($_ -split ' ')[1] -in $names })
        $lines | Set-Content -LiteralPath $legacy -Encoding ascii
    }
    Write-Output "Packed unsigned Sidenote $version into $feedPath. Kept $($keep.Count) full packages."
} finally { Pop-Location }
