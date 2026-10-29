# Run with: ./scripts/bump-version.ps1 <version>  (e.g. 0.2.0)
# Sets the version in core/Cargo.toml, core/Cargo.lock, shim/src/dllmain.cpp and workshop/Info.json, and turns CHANGELOG.md's
# "Unreleased" section into a dated section for that version, which the release workflow uses as the release notes.

param([Parameter(Mandatory)][string]$Version)

$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    Write-Error "Usage: ./scripts/bump-version.ps1 <version>  (e.g. 0.2.0)"
}

$root = Split-Path $PSScriptRoot -Parent

function Update-File([string]$RelativePath, [string]$Pattern, [string]$Replacement) {
    $path = Join-Path $root $RelativePath
    $contents = [IO.File]::ReadAllText($path)
    $regex = [regex]::new($Pattern, 'Multiline')
    if (-not $regex.IsMatch($contents)) {
        Write-Error "Could not find a version matching $Pattern in $RelativePath"
    }
    [IO.File]::WriteAllText($path, $regex.Replace($contents, $Replacement, 1))
    Write-Output "Updated $RelativePath"
}

Update-File 'core/Cargo.toml' '^version = "[^"]*"' "version = `"$Version`""
Update-File 'core/Cargo.lock' '(?<=name = "simpleminimap_core"\r?\nversion = ")[^"]*' $Version
Update-File 'shim/src/dllmain.cpp' '(?<=ModVersion = STR\(")[^"]*' $Version
Update-File 'workshop/Info.json' '(?<="Version": ")[^"]*' $Version

$changelogPath = Join-Path $root 'CHANGELOG.md'
$changelog = [IO.File]::ReadAllText($changelogPath)
$section = [regex]::Match($changelog, '(?ms)^## \[Unreleased\][^\n]*\n(.*?)(?=^## \[|\z)')
if (-not $section.Success) {
    Write-Error 'Could not find "## [Unreleased]" in CHANGELOG.md'
}
$body = $section.Groups[1].Value.Trim()
if (-not $body) {
    Write-Warning 'The "## [Unreleased]" section in CHANGELOG.md is empty, so this release has no notes.'
}
$date = Get-Date -Format 'yyyy-MM-dd'
$released = if ($body) { "## [Unreleased]`n`n## [$Version] - $date`n`n$body`n" } else { "## [Unreleased]`n`n## [$Version] - $date`n" }
$rest = $changelog.Substring($section.Index + $section.Length)
if ($rest) { $released += "`n" }
$changelog = $changelog.Substring(0, $section.Index) + $released + $rest
[IO.File]::WriteAllText($changelogPath, $changelog)
Write-Output 'Updated CHANGELOG.md'

Write-Output "`nBumped to $Version. Tag it as v$Version."
