# Contributing / Developer Guide

## Prerequisites

| Tool | Purpose | Install |
|------|---------|---------|
| Rust toolchain | Build | [rustup.rs](https://rustup.rs/) |
| Visual Studio Build Tools | MSVC linker, signtool | VS Installer > "C++ Build Tools" |
| .NET SDK | WiX CLI host | [dot.net](https://dot.net/) |
| WiX v7 | MSI installer | `dotnet tool install --global wix` + `wix eula accept wix7` |
| ARM64 target *(optional)* | ARM64 cross-compile | `rustup target add aarch64-pc-windows-msvc` + VS ARM64 component |

Code signing requires the HANA certificate installed in the Windows certificate store.

## Build Commands

```powershell
# ── Development ──
.\build.ps1 -Debug             # debug build (no signing)
.\build.ps1 -Check             # type check only
.\build.ps1 -Clippy            # lint
.\build.ps1 -Run               # build + launch

# ── Release ──
.\build.ps1                    # release x64 + sign
.\build.ps1 -Arm64             # release x64 + ARM64 + sign
.\build.ps1 -Msi               # release + signed MSI installer
.\build.ps1 -Msi -Arm64        # both architectures, both MSIs

# ── Skip git checks (testing) ──
.\build.ps1 -Msi -IgnoreCommit -IgnorePush
```

All options: `Get-Help .\build.ps1 -Examples`

## Version Management

Build number is managed automatically by hash-based change detection:

```
Cargo.toml version  →  0.1.3        (manual semver)
BUILD_NUMBER.txt    →  10059        (auto-incremented when source changes)
VERSION.txt         →  0.1.3.10059  (composed by build.ps1)
```

`build.ps1` computes a SHA-1 hash of `src/`, `Cargo.toml`, `Cargo.lock`, `build.rs`. If any file changed since the last build, `BUILD_NUMBER.txt` increments by 1.

## Release Flow

```
.\build.ps1 -Msi

  1. Hash check → auto-bump BUILD_NUMBER if source changed
  2. cargo build --release --target x86_64-pc-windows-msvc
  3. Invoke-Sign(EXE)         ← signtool (YubiKey PIN dialog)
  4. dist/ cleaned → portable EXE copied
  5. Assert-Signed(EXE)       ← unsigned → MSI creation blocked
  6. Test-ReleaseReady()      ← uncommitted/unpushed → blocked
  7. wix build → MSI (cab embedded, no pdb)
  8. Invoke-Sign(MSI)         ← signtool (YubiKey PIN dialog)
  9. dist/tail-f_plot-{ver}-x64.msi
```

**Signing is mandatory for distribution.** Unsigned binaries cannot produce MSI packages. YubiKey PIN dialog appears twice per release (EXE + MSI).

## Distribution Artifacts

```
dist/
├── tail-f_plot.exe                      ← portable standalone (signed)
└── tail-f_plot-{ver}-x64.msi            ← installer (signed)
```

With `-Arm64`:
```
dist/
├── tail-f_plot.exe                      ← x64 portable
├── tail-f_plot-arm64.exe                ← ARM64 portable
├── tail-f_plot-{ver}-x64.msi            ← x64 installer
└── tail-f_plot-{ver}-arm64.msi          ← ARM64 installer
```

## MSI Installer Details

- **Scope:** per-user (no UAC / admin elevation)
- **Install path:** `%LOCALAPPDATA%\HANA\tail-f_plot\`
- **PATH:** added to user environment variable
- **Start Menu:** `HANA-Cryogenics\tail-f_plot`
- **Upgrade:** in-place upgrade via stable UpgradeCode
- **Uninstall:** Windows Settings > Apps

## Project Structure

```
├── build.ps1                # Build / sign / package orchestrator
├── mssign.bat               # Authenticode signing (SHA256, DigiCert timestamp)
├── BUILD_NUMBER.txt         # Auto-incremented build number
├── VERSION.txt              # Full version (composed by build.ps1)
├── Cargo.toml
├── build.rs                 # Windows resource embedding (version, icon, manifest)
├── installer/
│   └── main.wxs             # WiX v7 MSI definition
├── src/
│   ├── main.rs              # Entry point, CLI parsing, console handling
│   ├── app.rs               # egui application logic
│   ├── csv_watcher.rs       # File watching + CSV parsing
│   ├── plot_state.rs        # Plot data management
│   ├── toolbar.rs           # Toolbar UI
│   ├── tooltip.rs           # Tooltip rendering
│   └── custom_legend.rs     # Legend rendering
├── hana.ico                 # Application icon
└── tail-f_plot.exe.manifest # Windows manifest (DPI, compatibility)
```
