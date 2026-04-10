#Requires -Version 5.1
<#
.SYNOPSIS
    Build, sign, and package tail-f_plot.

.EXAMPLE
    .\build.ps1                                   # release x64 + sign
    .\build.ps1 -Debug                            # debug build (no signing)
    .\build.ps1 -Arm64                            # x64 + ARM64
    .\build.ps1 -Msi                              # release + signed MSI
    .\build.ps1 -Msi -IgnoreCommit -IgnorePush    # skip git checks
    .\build.ps1 -Check                            # cargo check only
    .\build.ps1 -Clippy                           # cargo clippy only
    .\build.ps1 -Run                              # build + run
#>
param(
    [switch]$Debug,
    [switch]$Arm64,
    [switch]$Msi,
    [switch]$Run,
    [switch]$Check,
    [switch]$Clippy,
    [switch]$Audit,
    [switch]$IgnoreCommit,
    [switch]$IgnorePush
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$AppName     = "tail-f_plot"
$ExeName     = "$AppName.exe"
$SignScript   = ".\mssign.bat"
$BuildProfile = if ($Debug) { "debug" } else { "release" }

# ── Quick exits ──────────────────────────────────────────────
if ($Check)  { cargo check;                    exit $LASTEXITCODE }
if ($Clippy) { cargo clippy -- -W clippy::all; exit $LASTEXITCODE }
if ($Audit)  { cargo audit;                    exit $LASTEXITCODE }

# ── Version management ───────────────────────────────────────
$CargoVer = (Select-String -Path Cargo.toml -Pattern '^version').Line -replace '.*"(.+)".*','$1'

function Compute-Hash([string[]]$Paths) {
    $lines = @()
    foreach ($p in $Paths) {
        if (Test-Path $p -PathType Container) {
            $files  = @(git ls-files $p)
            $files += @(git ls-files --others --exclude-standard $p)
            $files  = $files | Where-Object { $_ -and $_ -notmatch 'BUILD_NUMBER\.txt$' } |
                      Sort-Object -Unique
            foreach ($f in $files) {
                if (Test-Path $f -PathType Leaf) { $lines += git hash-object $f }
            }
        } elseif (Test-Path $p -PathType Leaf) {
            $lines += git hash-object $p
        }
    }
    return ($lines -join "`n") | git hash-object --stdin
}

function Maybe-Bump {
    $bnFile   = "BUILD_NUMBER.txt"
    $hashFile = ".last_build_hash"

    if (-not (Test-Path $bnFile)) { Set-Content $bnFile "100" -NoNewline }

    $script:BuildChanged = $false
    $currentBn = [int](Get-Content $bnFile -Raw).Trim()
    $newHash   = Compute-Hash "src/","Cargo.toml","Cargo.lock","build.rs"
    $oldHash   = if (Test-Path $hashFile) { (Get-Content $hashFile -Raw).Trim() } else { "" }

    if ($newHash -ne $oldHash) {
        $currentBn++
        Set-Content $bnFile   "$currentBn" -NoNewline
        Set-Content $hashFile "$newHash"   -NoNewline
        $script:BuildChanged = $true
        Write-Host "  Build number bumped -> $currentBn"
    }

    $script:FullVersion = "$CargoVer.$currentBn"
    Set-Content "VERSION.txt" $FullVersion -NoNewline
}

Maybe-Bump
Write-Host "=== $AppName v$FullVersion ($BuildProfile) ==="

# ── Signing helpers ──────────────────────────────────────────
function Test-CodeSigned([string]$Path) {
    $sig = Get-AuthenticodeSignature $Path
    # Accept Valid (trusted CA) and UnknownError (self-signed).
    # Reject NotSigned and HashMismatch.
    return ($sig.Status -ne 'NotSigned' -and $sig.Status -ne 'HashMismatch')
}

function Invoke-Sign([string]$Path) {
    Write-Host "[Sign] Signing $Path..."
    & cmd /c $SignScript $Path
    if ($LASTEXITCODE -ne 0) {
        throw "FATAL: signtool failed for $Path"
    }
}

function Assert-Signed([string]$Path) {
    if (-not (Test-CodeSigned $Path)) {
        throw "ERROR: $Path is NOT signed — distribution blocked."
    }
}

# ── Pre-release checks ──────────────────────────────────────
function Test-ReleaseReady {
    if (-not $IgnoreCommit) {
        if ($BuildChanged) {
            throw @"
BUILD_NUMBER.txt changed — commit first:
  git add BUILD_NUMBER.txt VERSION.txt && git commit -m "Bump build"
  (or use -IgnoreCommit)
"@
        }
        git diff --quiet 2>$null
        $d1 = $LASTEXITCODE
        git diff --cached --quiet 2>$null
        $d2 = $LASTEXITCODE
        if ($d1 -ne 0 -or $d2 -ne 0) {
            throw "Uncommitted changes — commit first (or use -IgnoreCommit)"
        }
    }
    if (-not $IgnorePush) {
        $local  = git rev-parse HEAD
        $remote = git rev-parse '@{u}' 2>$null
        if ($remote -and $local -ne $remote) {
            throw "Unpushed commits — git push first (or use -IgnorePush)"
        }
    }
}

# ── Build ────────────────────────────────────────────────────
$X64Target   = "x86_64-pc-windows-msvc"
$Arm64Target = "aarch64-pc-windows-msvc"

function Invoke-CargoBuild([string]$Target) {
    Write-Host "[Build] $Target"
    $args_ = @("build", "--target", $Target)
    if ($BuildProfile -eq "release") { $args_ += "--release" }
    & cargo @args_
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

Invoke-CargoBuild $X64Target
$X64Exe = "target\$X64Target\$BuildProfile\$ExeName"

$Arm64Exe = $null
if ($Arm64) {
    Invoke-CargoBuild $Arm64Target
    $Arm64Exe = "target\$Arm64Target\$BuildProfile\$ExeName"
}

# ── Sign binaries (release only) ────────────────────────────
if ($BuildProfile -eq "release" -and (Test-Path $SignScript)) {
    Invoke-Sign $X64Exe
    if ($Arm64Exe) { Invoke-Sign $Arm64Exe }
}

# ── Dist: clean and prepare ─────────────────────────────────
$Dist = "dist"
if ($BuildProfile -eq "release") {
    if (Test-Path $Dist) { Remove-Item $Dist -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $Dist | Out-Null

    $portableX64 = "$Dist\$ExeName"
    Copy-Item $X64Exe $portableX64 -Force
    Write-Host "  -> $portableX64"

    if ($Arm64Exe) {
        $portableArm64 = "$Dist\$AppName-arm64.exe"
        Copy-Item $Arm64Exe $portableArm64 -Force
        Write-Host "  -> $portableArm64"
    }
}

# ── MSI (requires signed binary) ────────────────────────────
if ($Msi) {
    if ($BuildProfile -ne "release") { throw "-Msi requires release build" }

    Write-Host "=== MSI ==="
    Assert-Signed $X64Exe
    Test-ReleaseReady

    if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
        throw "WiX v4 CLI not found. Install: dotnet tool install --global wix"
    }

    # WiX intermediate files go to temp, not dist
    $wixObj = "target\wix"
    New-Item -ItemType Directory -Force -Path $wixObj | Out-Null

    # x64 MSI
    $msiX64     = "$Dist\$AppName-$FullVersion-x64.msi"
    $exeAbsPath = (Resolve-Path $X64Exe).Path
    & wix build installer\main.wxs `
        -d "Version=$FullVersion" -d "ExePath=$exeAbsPath" `
        -arch x64 -o $msiX64 -intermediateFolder $wixObj -pdbtype none
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Invoke-Sign $msiX64
    Write-Host "  -> $msiX64"

    # ARM64 MSI
    if ($Arm64Exe) {
        Assert-Signed $Arm64Exe
        $msiArm64      = "$Dist\$AppName-$FullVersion-arm64.msi"
        $armExeAbsPath = (Resolve-Path $Arm64Exe).Path
        & wix build installer\main.wxs `
            -d "Version=$FullVersion" -d "ExePath=$armExeAbsPath" `
            -arch arm64 -o $msiArm64 -intermediateFolder $wixObj -pdbtype none
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        Invoke-Sign $msiArm64
        Write-Host "  -> $msiArm64"
    }
}

# ── Winget manifest (disabled — requires microsoft/winget-pkgs PR per version)
# To re-enable, uncomment and run with -Msi:
#
# $remoteUrl  = git remote get-url origin
# $githubRepo = $remoteUrl -replace '.*(github\.com)[:/](.+?)(\.git)?$','$2'
# $releaseUrl = "https://github.com/$githubRepo/releases/download/v$FullVersion"
# $pkgId      = "HANA.tail-f-plot"
# $wingetDir  = "dist\winget\$pkgId\$FullVersion"
# New-Item -ItemType Directory -Force -Path $wingetDir | Out-Null
#
# # version manifest, locale manifest, installer manifest with SHA256...
# # See build.sh for full template.

# ── Run ──────────────────────────────────────────────────────
if ($Run) { & ".\$X64Exe" }

Write-Host "=== Done ==="
