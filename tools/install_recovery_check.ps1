function Test-DuplicateInstallRecoveryField([string]$Raw, [string[]]$Required) {
    $requiredFields = [Collections.Generic.HashSet[string]]::new($Required, [StringComparer]::OrdinalIgnoreCase)
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $depth = 0
    $end = 0
    $expectKey = $false
    # Match strings and comments as whole tokens so their punctuation cannot affect object depth.
    $pattern = '\G(?:\s+|//[^\r\n]*|/\*[\s\S]*?\*/|"(?:[^"\\]|\\[\s\S])*"|[{}\[\],:]|' +
        'true|false|null|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)'
    foreach ($token in [regex]::Matches($Raw, $pattern)) {
        $end = $token.Index + $token.Length
        switch ($token.Value) {
            '{' { $depth++; if ($depth -eq 1) { $expectKey = $true } }
            '[' { $depth++ }
            '}' { $depth-- }
            ']' { $depth-- }
            ',' { if ($depth -eq 1) { $expectKey = $true } }
            default {
                if ($depth -eq 1 -and $expectKey -and $token.Value.StartsWith('"')) {
                    $key = (ConvertFrom-Json -InputObject ('{"key":' + $token.Value + '}') -ErrorAction Stop).key
                    if ($requiredFields.Contains($key) -and -not $seen.Add($key)) { return $true }
                    $expectKey = $false
                }
            }
        }
    }
    if ($end -ne $Raw.Length) { throw 'Unexpected install state token.' }
    return $false
}

function Assert-CanonicalInstallState([string]$Updates, [string]$Version) {
    foreach ($marker in @('install-hook.complete', 'update-listener.resume', 'pipeline-install.pending')) {
        if (Test-Path -LiteralPath (Join-Path $Updates $marker) -ErrorAction Stop) { throw "Unexpected marker: $marker." }
    }
    $required = @('schema_version', 'app_version', 'listener_work', 'pipeline_work')
    try {
        $raw = [IO.File]::ReadAllText((Join-Path $Updates 'install-state.json'))
        if (-not $raw.TrimStart().StartsWith('{')) { throw 'Expected a JSON object.' }
        $duplicate = Test-DuplicateInstallRecoveryField $raw $required
        if (-not $duplicate) { $state = ConvertFrom-Json -InputObject $raw -ErrorAction Stop }
    } catch { throw 'Install state is missing or unreadable.' }
    if ($duplicate) { throw 'Install state contains a duplicate required field.' }
    $keys = @($state.PSObject.Properties | ForEach-Object { $_.Name })
    foreach ($key in $required) {
        if ($keys -cnotcontains $key) { throw "Install state did not complete for $Version." }
    }
    if (($state.schema_version -isnot [int] -and $state.schema_version -isnot [long]) -or $state.schema_version -ne 1 -or
        $state.app_version -isnot [string] -or $state.app_version -cne $Version -or
        $state.listener_work -isnot [string] -or $state.listener_work -cne 'Complete' -or
        $state.pipeline_work -isnot [string] -or $state.pipeline_work -cnotin @('Complete', 'NotConfigured')) {
        throw "Install state did not complete for $Version."
    }
}

function Get-InstallRecoveryEvidence([string]$Updates, [string]$Version) {
    if (Test-Path -LiteralPath (Join-Path $Updates 'install-state.json') -ErrorAction Stop) {
        Assert-CanonicalInstallState $Updates $Version
        return 'canonical record complete'
    }
    if (Test-Path -LiteralPath (Join-Path $Updates 'pipeline-install.pending') -ErrorAction Stop) {
        throw 'Legacy pipeline install is pending.'
    }
    return 'legacy pipeline marker absent'
}
