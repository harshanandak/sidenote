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
