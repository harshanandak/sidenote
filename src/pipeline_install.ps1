$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = New-Object Text.UTF8Encoding($false)
$utf8 = New-Object Text.UTF8Encoding($false)
$mutex = New-Object Threading.Mutex($false, 'Local\SidenotePipelineInstall')
$held = $false
function Write-Durable([string]$Path, [byte[]]$Bytes) {
    $file = [IO.FileStream]::new($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write,
        [IO.FileShare]::None, 4096, [IO.FileOptions]::WriteThrough)
    try { $file.Write($Bytes, 0, $Bytes.Length); $file.Flush($true) } finally { $file.Dispose() }
}
function Hash-Bytes([byte[]]$Bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
function Verify-Generation($Selection) {
    if ($Selection.generation -cnotmatch '\A(?:generation|legacy)-[0-9a-f]{32}\z' -or
        $Selection.manifest_sha256 -cnotmatch '\A[0-9a-f]{64}\z') { throw 'Invalid selection' }
    $folder = Join-Path $generations $Selection.generation
    $manifestPath = Join-Path $folder '.generation.json'
    if ((Get-Item -LiteralPath $manifestPath).Length -gt 8192 -or
        (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash -ine $Selection.manifest_sha256) { throw 'Invalid manifest' }
    $manifest = [IO.File]::ReadAllText($manifestPath) | ConvertFrom-Json
    $members = if ($manifest.files.PSObject.Properties.Name -ccontains 'job-contract.json') { $names } else { $legacyNames }
    if ($manifest.schema -cne 'sidenote.pipeline/1' -or @($manifest.files.PSObject.Properties).Count -ne $members.Count) { throw 'Invalid manifest' }
    foreach ($property in $manifest.files.PSObject.Properties) {
        if ($property.Name -cnotin $members) { throw 'Invalid manifest' }
    }
    foreach ($name in $members) {
        if ($manifest.files.$name -cnotmatch '\A[0-9a-f]{64}\z' -or
            (Get-FileHash -LiteralPath (Join-Path $folder $name) -Algorithm SHA256).Hash -ine $manifest.files.$name) { throw 'Generation incomplete' }
    }
    return $manifest
}
function New-Generation([string]$Version, $Files, $Hashes, [string]$Prefix, [string[]]$Members) {
    $id = $Prefix + '-' + [guid]::NewGuid().ToString('N')
    $folder = Join-Path $generations $id
    New-Item -ItemType Directory -Path $folder | Out-Null
    foreach ($name in $Members) {
        $path = Join-Path $folder $name
        Write-Durable $path $Files[$name]
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $Hashes[$name]) { throw 'Hash mismatch' }
    }
    # The seal is written last, after all expected bytes have been verified.
    $manifest = @{schema='sidenote.pipeline/1'; version=$Version; files=$Hashes} | ConvertTo-Json -Depth 5
    $manifestPath = Join-Path $folder '.generation.json'
    Write-Durable $manifestPath $utf8.GetBytes($manifest)
    $selection = @{generation=$id; manifest_sha256=(Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()}
    [void](Verify-Generation $selection)
    return $selection
}
function Publish-Pointer($Current, $Previous) {
    $json = @{schema='sidenote.pipeline/1'; current=$Current; previous=$Previous} | ConvertTo-Json -Depth 5
    $temp = Join-Path $root ('.pipeline-pointer-' + [guid]::NewGuid().ToString('N') + '.tmp')
    Write-Durable $temp $utf8.GetBytes($json)
    if ([IO.File]::Exists($pointerPath)) {
        [IO.File]::Replace($temp, $pointerPath, (Join-Path $root '.pipeline-current.previous.json'))
    } else { [IO.File]::Move($temp, $pointerPath) }
}
try {
    try { $held = $mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $held = $true }
    if (-not $held) { throw 'Installation busy' }
    $payload = [Console]::In.ReadToEnd() | ConvertFrom-Json
    if (-not $env:USERPROFILE) { throw 'User profile missing' }
    $root = Join-Path $env:USERPROFILE '.config\hex\scripts'
    $pointerPath = Join-Path $root '.pipeline-current.json'
    $entryPath = Join-Path $root 'hex-meeting-watch.ps1'
    if (-not [IO.File]::Exists($pointerPath) -and -not [IO.File]::Exists($entryPath)) { throw 'Meeting scripts are not configured' }
    foreach ($setting in @('HEX_MEETING_PROJECT','HEX_MEETING_PROJECT_PATTERN','HEX_MEETING_PROJECT_CONTEXT_PATTERN',
        'HEX_MEETING_KB_REPO','HEX_MEETING_OWNER_NAME','HEX_MEETING_COMPANY_NAME','HEX_MEETING_PRODUCT_NAME','HEX_TE_ASR_MODEL')) {
        $value = [Environment]::GetEnvironmentVariable($setting)
        if (-not $value) { $value = [Environment]::GetEnvironmentVariable($setting, 'User') }
        if (-not $value) { throw 'Private settings missing' }
    }
    $names = @('hex-meeting-compare-delayed.ps1','hex-meeting-compare-granola.py','hex-meeting-english.ps1',
        'hex-meeting-notes.ps1','hex-meeting-publish.py','hex-meeting-quick-title.ps1','hex-meeting-render.py',
        'hex-meeting-tasks.py','hex-meeting-watch.ps1','hex-pipeline-cli.ps1','hex-pipeline-run.py','job-contract.json',
        'meeting-audio.py','meeting-notes-prompt.md','meeting-translate-prompt.md')
    $legacyNames = @($names | Where-Object { $_ -cne 'job-contract.json' })
    if (@($payload.files.PSObject.Properties).Count -ne $names.Count -or
        @($payload.hashes.PSObject.Properties).Count -ne $names.Count -or -not $payload.version -or -not $payload.entry) { throw 'Invalid bundle' }
    $files = @{}; $hashes = @{}
    foreach ($name in $names) {
        if ($payload.files.$name -isnot [string] -or -not $payload.files.$name -or $payload.hashes.$name -cnotmatch '\A[0-9a-f]{64}\z') { throw 'Invalid bundle' }
        $files[$name] = $utf8.GetBytes($payload.files.$name); $hashes[$name] = $payload.hashes.$name
        if ((Hash-Bytes $files[$name]) -cne $hashes[$name]) { throw 'Invalid bundle' }
    }
    $entryBytes = $utf8.GetBytes($payload.entry)
    if ((Hash-Bytes $entryBytes) -cne $payload.entry_hash) { throw 'Invalid entry' }
    $generations = Join-Path $root '.pipeline-generations'
    New-Item -ItemType Directory -Path $generations -Force | Out-Null
    if ([IO.File]::Exists($pointerPath)) {
        if ((Get-Item -LiteralPath $pointerPath).Length -gt 2048) { throw 'Invalid pointer' }
        $pointer = [IO.File]::ReadAllText($pointerPath) | ConvertFrom-Json
        if ($pointer.schema -cne 'sidenote.pipeline/1') { throw 'Invalid pointer' }
        $current = $pointer.current
        $manifest = Verify-Generation $current
    } else {
        # Preserve exact owner bytes before changing the sole legacy entrypoint.
        $backup = Join-Path (Join-Path $root '.pipeline-backups') ([guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $backup | Out-Null
        $oldFiles = @{}; $oldHashes = @{}
        $oldNames = if ([IO.File]::Exists((Join-Path $root 'job-contract.json'))) { $names } else { $legacyNames }
        foreach ($name in $oldNames) {
            $source = Join-Path $root $name
            if ((Get-Item -LiteralPath $source).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Invalid legacy source' }
            $oldFiles[$name] = [IO.File]::ReadAllBytes($source)
            $oldHashes[$name] = Hash-Bytes $oldFiles[$name]
            $saved = Join-Path $backup $name
            Write-Durable $saved $oldFiles[$name]
            if ((Get-FileHash -LiteralPath $saved -Algorithm SHA256).Hash -ine $oldHashes[$name]) { throw 'Backup mismatch' }
        }
        # Relocate the one legacy helper that otherwise reads prompts from the root.
        $comparison = 'hex-meeting-compare-granola.py'
        $text = $utf8.GetString($oldFiles[$comparison])
        $oldSelector = "SCRIPTS = Path.home() / '.config' / 'hex' / 'scripts'"
        $newSelector = 'SCRIPTS = Path(__file__).resolve().parent'
        if ($text.Contains($oldSelector)) {
            $oldFiles[$comparison] = $utf8.GetBytes($text.Replace($oldSelector, $newSelector))
            $oldHashes[$comparison] = Hash-Bytes $oldFiles[$comparison]
        } elseif (-not $text.Contains($newSelector)) { throw 'Unsupported legacy script root' }
        $current = New-Generation 'legacy' $oldFiles $oldHashes 'legacy' $oldNames
        Publish-Pointer $current $null
        $manifest = Verify-Generation $current
    }
    if ([IO.File]::Exists($entryPath) -and ((Get-Item -LiteralPath $entryPath).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid entry' }
    if (-not [IO.File]::Exists($entryPath) -or (Get-FileHash -LiteralPath $entryPath -Algorithm SHA256).Hash -ine $payload.entry_hash) {
        $backup = Join-Path (Join-Path $root '.pipeline-backups') ([guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $backup | Out-Null
        $temp = Join-Path $root ('.pipeline-entry-' + [guid]::NewGuid().ToString('N') + '.tmp')
        Write-Durable $temp $entryBytes
        if ((Get-FileHash -LiteralPath $temp -Algorithm SHA256).Hash -ine $payload.entry_hash) { throw 'Entry hash mismatch' }
        if ([IO.File]::Exists($entryPath)) {
            [IO.File]::Replace($temp, $entryPath, (Join-Path $backup 'hex-meeting-watch.ps1'))
        } else { [IO.File]::Move($temp, $entryPath) }
    }
    $same = $manifest.version -ceq $payload.version
    foreach ($name in $names) { $same = $same -and $manifest.files.$name -ceq $hashes[$name] }
    if (-not $same) {
        $next = New-Generation $payload.version $files $hashes 'generation' $names
        # The only commit point: all generation files are sealed and immutable.
        Publish-Pointer $next $current
    }
} catch { if ($_.Exception.Message -eq 'Meeting scripts are not configured') { exit 3 }; exit 1 } finally {
    if ($held) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
