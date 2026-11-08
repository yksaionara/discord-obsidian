# Ships a new version to everyone who installed Discord-Setup.exe: bumps the
# version, builds, makes the installer, commits, pushes, and publishes a
# GitHub release. Each install finds the release within 6 hours (or at its
# next start), downloads it, and installs it on its next start or from the
# rail's Update button.
#
# Usage, from the repository:  powershell -File scripts\ship.ps1 [-Version 0.5.0] [-Notes "what changed"]
# Without -Version, the patch number goes up by one.
param(
    [string]$Version,
    [string]$Notes = "",
    [string]$Repository = "yksaionara/dc",
    [string]$Remote = "ours",
    # Extra lines for the end of the commit message, such as Co-Authored-By.
    [string]$Trailer = ""
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

# The version, in Cargo.toml's [package].
$cargo = Get-Content Cargo.toml -Raw
if ($cargo -notmatch '(?m)^version = "(\d+)\.(\d+)\.(\d+)"') { throw "no version in Cargo.toml" }
$current = "$($Matches[1]).$($Matches[2]).$($Matches[3])"
if (-not $Version) { $Version = "$($Matches[1]).$($Matches[2]).$([int]$Matches[3] + 1)" }
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "version must look like 1.2.3" }
Write-Host "Shipping $current -> $Version"
$regex = [regex]'(?m)^version = "\d+\.\d+\.\d+"'
$cargo = $regex.Replace($cargo, "version = `"$Version`"", 1)
[System.IO.File]::WriteAllText((Resolve-Path Cargo.toml), $cargo)

# Build. A running copy locks the exe, but Windows lets it be renamed.
$skia = Get-ChildItem .local\windows\skia\prepared\*.tar.gz | Select-Object -First 1
if (-not $skia) { throw "no prepared Skia archive in .local\windows\skia\prepared" }
$env:SKIA_BINARIES_URL = "file://" + ($skia.FullName -replace '\\', '/')
$exe = "target\release\yapfast.exe"
if (Test-Path $exe) {
    Remove-Item "target\release\yapfast.old.exe" -ErrorAction SilentlyContinue
    try { Move-Item $exe "target\release\yapfast.old.exe" } catch { }
}
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "the build failed" }

# The installer.
& "C:\Program Files\Inno Setup 7\ISCC.exe" /Q "/DAppVersion=$Version" packaging\windows\personal-setup.iss
if ($LASTEXITCODE -ne 0) { throw "the installer didn't compile" }
$installer = "target\dist\personal\Discord-Setup.exe"

# Commit, tag and push.
git add -A
$body = if ($Notes) { $Notes } else { "Release v$Version." }
if ($Trailer) { git commit -m "Ship v$Version" -m $body -m $Trailer } else { git commit -m "Ship v$Version" -m $body }
if ($LASTEXITCODE -ne 0) { throw "nothing to commit, or the commit failed" }
git tag "v$Version"
git push $Remote HEAD:main "v$Version"
if ($LASTEXITCODE -ne 0) { throw "the push failed" }

# The release the apps update from.
$title = "Discord v$Version"
if (-not $Notes) { $Notes = "Update v$Version." }
gh release create "v$Version" $installer --repo $Repository --title $title --notes $Notes
if ($LASTEXITCODE -ne 0) { throw "the release didn't publish" }
Write-Host "Shipped v${Version}: https://github.com/$Repository/releases/tag/v$Version"
