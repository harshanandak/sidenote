function Assert-CanonicalInstallState([string]$Updates, [string]$Version) {
    foreach ($marker in @('install-hook.complete', 'update-listener.resume', 'pipeline-install.pending')) {
        if (Test-Path -LiteralPath (Join-Path $Updates $marker) -ErrorAction Stop) { throw "Unexpected marker: $marker." }
    }
    try {
        $raw = [IO.File]::ReadAllText((Join-Path $Updates 'install-state.json'))
        if (-not $raw.TrimStart().StartsWith('{')) { throw 'Expected a JSON object.' }
        $state = ConvertFrom-Json -InputObject $raw -ErrorAction Stop
    } catch { throw 'Install state is missing or unreadable.' }
    $keys = @($state.PSObject.Properties | ForEach-Object { $_.Name })
    foreach ($key in @('schema_version', 'app_version', 'listener_work', 'pipeline_work')) {
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
