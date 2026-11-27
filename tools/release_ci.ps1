#Requires -Version 7.4
[CmdletBinding(DefaultParameterSetName = 'Package')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Package')]
    [string]$OutputDirectory,
    [Parameter(Mandatory, ParameterSetName = 'Validate')]
    [switch]$ValidateOnly,
    [Parameter(Mandatory, ParameterSetName = 'Test')]
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-ReleaseIdentity {
    param([string]$Manifest, [string]$EventName, [string]$Ref,
        [string]$RequestedTag, [string]$RequestedChannel, [string]$RunNumber)

    $package = [regex]::Match($Manifest, '(?ms)^\[package\][ \t]*\r?\n(.*?)(?=^\[|\z)').Groups[1].Value
    $version = [regex]::Match($package, '(?m)^version = "([^"]+)"\r?$').Groups[1].Value
    $number = '(?:0|[1-9][0-9]*)'
    $semver = "$number\.$number\.$number"
    $channel = switch -CaseSensitive -Regex ($version) {
        "\A$semver\z" { 'stable' }
        "\A$semver-dev\.$number\z" { 'dev' }
        default { throw 'Cargo.toml [package] version must be X.Y.Z or X.Y.Z-dev.N.' }
    }
    $tag = "v$version"

    switch -CaseSensitive ($EventName) {
        'workflow_run' {
            if ($Ref -cne 'refs/heads/main' -or $RunNumber -cnotmatch '\A[1-9][0-9]*\z') {
                throw 'Expected a main workflow run with a positive run number.'
            }
            if ($channel -cne 'dev') { throw 'Latest requires a Dev prerelease version in Cargo.toml.' }
            $version = "$version.main.$RunNumber"
            $tag = "v$version"
            $channel = 'latest'
        }
        'push' {
            if ($Ref -cne "refs/tags/$tag") { throw "Release tag must match Cargo.toml version ($tag)." }
        }
        'workflow_dispatch' {
            if ($RequestedTag -cne $tag) { throw "Requested tag must match Cargo.toml version ($tag)." }
            if ($Ref.StartsWith('refs/tags/') -and $Ref -cne "refs/tags/$tag") {
                throw "Selected tag ref must match Cargo.toml version ($tag)."
            }
            if ($RequestedChannel -cne $channel) { throw "Requested channel must be $channel for $version." }
        }
        'pull_request' {
            if ($Ref -cnotmatch '\Arefs/pull/[1-9][0-9]*/merge\z') { throw 'Expected a PR merge ref for the packaging proof.' }
        }
        default { throw 'Expected a tag push, main workflow run, manual dry run, or pull request.' }
    }

    [pscustomobject]@{ Tag = $tag; Version = $version; Channel = $channel }
}

function Get-ReleaseAssets {
    param([string]$Feed, [string]$Version, [string]$Channel)

    $setup = "sidenote-$Channel-Setup.exe"
    $index = "releases.$Channel.json"
    $full = "sidenote-$Version-$Channel-full.nupkg"
    $delta = "sidenote-$Version-$Channel-delta.nupkg"
    $sidecars = @("RELEASES-$Channel", "assets.$Channel.json", "sidenote-$Channel-Portable.zip")
    $files = [Collections.Generic.Dictionary[string, IO.FileInfo]]::new([StringComparer]::Ordinal)
    foreach ($item in Get-ChildItem -LiteralPath $Feed -Force) {
        if ($item -isnot [IO.FileInfo] -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Release feed must contain regular files only.'
        }
        if ($item.Name -cnotin (@($setup, $index, $full, $delta) + $sidecars)) {
            throw 'Unexpected file in the Velopack output.'
        }
        $files.Add($item.Name, $item)
    }
    foreach ($name in @($setup, $index, $full)) {
        if (-not $files.ContainsKey($name) -or $files[$name].Length -le 0) {
            throw "Missing or empty required release file: $name."
        }
    }

    $feedIndex = Get-Content -LiteralPath $files[$index].FullName -Raw | ConvertFrom-Json -AsHashtable
    if ($feedIndex -isnot [Collections.IDictionary] -or $feedIndex['Assets'] -isnot [array]) {
        throw 'Release index must contain an Assets array.'
    }
    $references = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($asset in $feedIndex['Assets']) {
        if ($asset -isnot [Collections.IDictionary]) { throw 'Invalid release index entry.' }
        $name = switch -CaseSensitive ($asset['Type']) {
            'Full' { $full }
            'Delta' { $delta }
            default { throw 'Unexpected package type in release index.' }
        }
        if ($asset['PackageId'] -cne 'sidenote' -or $asset['Version'] -cne $Version -or $asset['FileName'] -cne $name) {
            throw 'Package identity in release index does not match the build.'
        }
        if (-not $files.ContainsKey($name) -or -not $references.Add($name)) {
            throw "Release index references a missing or duplicate package: $name."
        }
        if ($files[$name].Length -le 0 -or $files[$name].Length -ne $asset['Size'] -or
            (Get-FileHash -LiteralPath $files[$name].FullName -Algorithm SHA256).Hash -ne $asset['SHA256']) {
            throw "Release index size or SHA-256 mismatch: $name."
        }
    }
    if (-not $references.Contains($full) -or $files.ContainsKey($delta) -ne $references.Contains($delta)) {
        throw 'Release index must reference the full package and any generated delta.'
    }

    $files[$setup]
    $files[$index]
    $files[$full]
    if ($references.Contains($delta)) { $files[$delta] }
}

function Write-ReleaseChecksums {
    param([string]$OutputDirectory)

    $files = @(Get-ChildItem -LiteralPath $OutputDirectory -Force | Where-Object {
        $_ -is [IO.FileInfo] -and $_.Name -cne 'SHA256SUMS'
    })
    if ($files.Count -eq 0) { throw 'Cannot write SHA256SUMS without staged files.' }
    $byName = [Collections.Generic.Dictionary[string, IO.FileInfo]]::new([StringComparer]::Ordinal)
    foreach ($file in $files) { $byName.Add($file.Name, $file) }
    $names = @($byName.Keys)
    [Array]::Sort($names, [StringComparer]::Ordinal)
    $lines = foreach ($name in $names) {
        $hash = (Get-FileHash -LiteralPath $byName[$name].FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $name"
    }
    $content = ($lines -join "`n") + "`n"
    [IO.File]::WriteAllText((Join-Path $OutputDirectory 'SHA256SUMS'), $content, [Text.UTF8Encoding]::new($false))
}

function Assert-Rejected {
    param([string]$Message, [scriptblock]$Attempt)
    try { & $Attempt | Out-Null } catch {
        if ($_.Exception.Message.Contains($Message, [StringComparison]::Ordinal)) { return }
        throw
    }
    throw "Expected rejection: $Message."
}

function Invoke-SelfTest {
    $stable = "[workspace]`n[package]`nversion = `"0.1.0`"`n[dependencies]`n"
    $dev = $stable.Replace('0.1.0', '0.1.1-dev.2')
    $stableBuild = Resolve-ReleaseIdentity $stable push 'refs/tags/v0.1.0'
    $devBuild = Resolve-ReleaseIdentity $dev workflow_dispatch 'refs/heads/main' 'v0.1.1-dev.2' dev
    $prBuild = Resolve-ReleaseIdentity $dev pull_request 'refs/pull/1/merge'
    if ($stableBuild.Channel -cne 'stable' -or $devBuild.Channel -cne 'dev' -or $prBuild.Tag -cne $devBuild.Tag) {
        throw 'Valid release identities did not resolve to the expected tag and channel.'
    }
    Assert-Rejected 'Release tag must match' { Resolve-ReleaseIdentity $stable push 'refs/tags/v0.1.1' }
    Assert-Rejected 'Release tag must match' { Resolve-ReleaseIdentity $dev push 'refs/tags/dev' }
    $latest = Resolve-ReleaseIdentity $dev workflow_run 'refs/heads/main' -RunNumber 7
    if ($latest.Channel -cne 'latest' -or $latest.Tag -cne 'v0.1.1-dev.2.main.7' -or $latest.Version -cne '0.1.1-dev.2.main.7') {
        throw 'Latest release identity did not match the expected version and channel.'
    }
    Assert-Rejected 'Expected a main workflow run' { Resolve-ReleaseIdentity $dev workflow_run 'refs/heads/topic' -RunNumber 7 }
    Assert-Rejected 'Expected a main workflow run' { Resolve-ReleaseIdentity $dev workflow_run 'refs/heads/main' -RunNumber '07' }
    Assert-Rejected 'Latest requires a Dev prerelease' { Resolve-ReleaseIdentity $stable workflow_run 'refs/heads/main' -RunNumber 7 }
    Assert-Rejected 'Release tag must match' { Resolve-ReleaseIdentity $stable push 'refs/heads/v0.1.0' }
    Assert-Rejected 'Requested tag must match' { Resolve-ReleaseIdentity $dev workflow_dispatch 'refs/heads/main' 'v0.1.1' dev }
    Assert-Rejected 'Selected tag ref must match' {
        Resolve-ReleaseIdentity $dev workflow_dispatch 'refs/tags/v0.1.1-dev.1' 'v0.1.1-dev.2' dev
    }
    Assert-Rejected 'Requested channel must be dev' {
        Resolve-ReleaseIdentity $dev workflow_dispatch 'refs/heads/main' 'v0.1.1-dev.2' stable
    }
    Assert-Rejected 'Requested channel must be stable' {
        Resolve-ReleaseIdentity $stable workflow_dispatch 'refs/heads/main' 'v0.1.0' dev
    }
    foreach ($version in @('0.01.0', '0.1.0-rc.1', '0.1.0-dev.02', '0.1.0+build', '0.1.0-DEV.2')) {
        Assert-Rejected 'Cargo.toml [package] version must be' {
            Resolve-ReleaseIdentity ($stable.Replace('0.1.0', $version)) push "refs/tags/v$version"
        }
    }

    $temp = [IO.Directory]::CreateTempSubdirectory('sidenote-release-test-')
    try {
        $setup = 'sidenote-dev-Setup.exe'
        $index = 'releases.dev.json'
        $full = 'sidenote-0.1.1-dev.2-dev-full.nupkg'
        $delta = 'sidenote-0.1.1-dev.2-dev-delta.nupkg'
        $assets = @(
            @{ PackageId = 'sidenote'; Version = '0.1.1-dev.2'; Type = 'Full'; FileName = $full }
            @{ PackageId = 'sidenote'; Version = '0.1.1-dev.2'; Type = 'Delta'; FileName = $delta }
        )
        foreach ($name in @($setup, $full, $delta, 'RELEASES-dev', 'assets.dev.json', 'sidenote-dev-Portable.zip')) {
            Set-Content -LiteralPath (Join-Path $temp.FullName $name) -Value 'fixture' -NoNewline
        }
        foreach ($asset in $assets) {
            $path = Join-Path $temp.FullName $asset.FileName
            $asset.Size = (Get-Item -LiteralPath $path).Length
            $asset.SHA256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        }
        $indexPath = Join-Path $temp.FullName $index
        @{ Assets = $assets } | ConvertTo-Json | Set-Content -LiteralPath $indexPath
        $inspect = { Get-ReleaseAssets $temp.FullName '0.1.1-dev.2' dev }
        if ((@(& $inspect).Name -join ',') -cne "$setup,$index,$full,$delta") {
            throw 'Allow-list must include only the installer, feed and indexed packages.'
        }

        $extra = Join-Path $temp.FullName 'unlisted.nupkg'
        New-Item -ItemType File -Path $extra | Out-Null
        Assert-Rejected 'Unexpected file' $inspect
        Remove-Item -LiteralPath $extra
        $assets[1].FileName = "../$delta"
        @{ Assets = $assets } | ConvertTo-Json | Set-Content -LiteralPath $indexPath
        Assert-Rejected 'Package identity' $inspect
        @{ Assets = @($assets[0], $assets[0]) } | ConvertTo-Json | Set-Content -LiteralPath $indexPath
        Assert-Rejected 'missing or duplicate package' $inspect

        @{ Assets = @($assets[0]) } | ConvertTo-Json | Set-Content -LiteralPath $indexPath
        Assert-Rejected 'any generated delta' $inspect
        Remove-Item -LiteralPath (Join-Path $temp.FullName $delta)
        if ((@(& $inspect).Name -join ',') -cne "$setup,$index,$full") {
            throw 'The first build must be allowed without a delta.'
        }
        Set-Content -LiteralPath (Join-Path $temp.FullName $full) -Value 'altered' -NoNewline
        Assert-Rejected 'SHA-256 mismatch' $inspect
        Remove-Item -LiteralPath (Join-Path $temp.FullName $full)
        Assert-Rejected 'Missing or empty required release file' $inspect
    } finally {
        $temp.Delete($true)
    }

    $checksumTemp = [IO.Directory]::CreateTempSubdirectory('sidenote-checksum-test-')
    try {
        foreach ($fixture in @(
            @{ Name = 'z-last.txt'; Value = 'zeta' },
            @{ Name = 'a-first.txt'; Value = 'alpha' },
            @{ Name = 'm-middle.txt'; Value = 'middle' }
        )) {
            Set-Content -LiteralPath (Join-Path $checksumTemp.FullName $fixture.Name) -Value $fixture.Value -NoNewline
        }
        Write-ReleaseChecksums -OutputDirectory $checksumTemp.FullName
        $sumPath = Join-Path $checksumTemp.FullName 'SHA256SUMS'
        $expectedNames = @('a-first.txt', 'm-middle.txt', 'z-last.txt')
        $expectedLines = foreach ($name in $expectedNames) {
            $hash = (Get-FileHash -LiteralPath (Join-Path $checksumTemp.FullName $name) -Algorithm SHA256).Hash.ToLowerInvariant()
            "$hash  $name"
        }
        $expected = ($expectedLines -join "`n") + "`n"
        $actual = [IO.File]::ReadAllText($sumPath)
        if ($actual -cne $expected) { throw 'SHA256SUMS content or file-name order does not match the fixture.' }
        if ($actual.Contains("`r")) { throw 'SHA256SUMS must use LF line endings.' }

        Set-Content -LiteralPath (Join-Path $checksumTemp.FullName 'a-first.txt') -Value 'altered' -NoNewline
        $mismatches = foreach ($line in [IO.File]::ReadAllLines($sumPath)) {
            $name = $line.Substring(66)
            $hash = (Get-FileHash -LiteralPath (Join-Path $checksumTemp.FullName $name) -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($line -cne "$hash  $name") { $name }
        }
        if (($mismatches -join ',') -cne 'a-first.txt') { throw 'A changed fixture file must no longer match SHA256SUMS.' }
    } finally {
        $checksumTemp.Delete($true)
    }
    Write-Output 'Release identity, asset allow-list and checksum self-test passed.'
}

if ($MyInvocation.InvocationName -eq '.') { return }
if ($SelfTest) {
    Invoke-SelfTest
    return
}

$root = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
$build = Resolve-ReleaseIdentity $manifest $env:GITHUB_EVENT_NAME $env:GITHUB_REF $env:RELEASE_TAG $env:RELEASE_CHANNEL $env:GITHUB_RUN_NUMBER
Write-Output "Validated $($build.Tag) for the $($build.Channel) channel."
if ($env:GITHUB_OUTPUT) {
    "tag=$($build.Tag)" | Add-Content -LiteralPath $env:GITHUB_OUTPUT -Encoding utf8
}
if ($ValidateOnly) { return }

if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory) -or (Test-Path -LiteralPath $OutputDirectory)) {
    throw 'OutputDirectory must be a new absolute directory.'
}
$feed = [IO.Directory]::CreateTempSubdirectory('sidenote-release-feed-')
Push-Location -LiteralPath $root
try {
    if ($build.Channel -ceq 'latest') {
        . (Join-Path $PSScriptRoot 'release_version.ps1')
        Set-SourceVersion $root $build.Version
    }
    & cargo +stable-x86_64-pc-windows-gnu fetch --locked --target x86_64-pc-windows-gnu
    if ($LASTEXITCODE -ne 0) { throw 'Could not fetch the locked Windows GNU dependencies.' }
    & (Join-Path $PSScriptRoot 'pack_release.ps1') -Feed $feed.FullName -Channel $build.Channel
    $assets = @(Get-ReleaseAssets $feed.FullName $build.Version $build.Channel)
    New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
    foreach ($asset in $assets) {
        Copy-Item -LiteralPath $asset.FullName -Destination (Join-Path $OutputDirectory $asset.Name)
        Write-Output "Staged $($asset.Name)"
    }
    Write-ReleaseChecksums -OutputDirectory $OutputDirectory
    Write-Output 'Staged SHA256SUMS'
} finally {
    Pop-Location
    $feed.Delete($true)
}
