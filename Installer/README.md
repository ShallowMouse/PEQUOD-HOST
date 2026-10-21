# Installer and automatic updates

Every push to `main` builds:

* `PEQUOD-Host-Setup-1.0.<build>.exe`: the installer. It installs for your user account (no admin prompt), adds Start-menu and desktop shortcuts and an uninstaller, and puts the controller firmware in `Controller firmware` next to the program.
* `PEQUOD-Host-1.0.<build>-portable.zip`: the same files without an installer.

The build publishes each version as a release on the **public** repo `ShallowMouse/PEQUOD-Host-Releases`. That repo holds only installers; the source stays private. When PEQUOD Host starts, it checks that repo. If a newer version is there, an **UPDATE** button appears in the top bar. Clicking it downloads the installer, closes PEQUOD, installs the update and starts PEQUOD again. Presets and settings are kept.

## One-time setup

1. Create the releases repo: GitHub → **New repository** → name `PEQUOD-Host-Releases` → **Public** → tick **Add a README file** → **Create**.
2. Create a token the build can publish with: GitHub → Settings → Developer settings → **Fine-grained personal access tokens** → **Generate new token**.
   * Repository access: **Only select repositories** → `PEQUOD-Host-Releases`
   * Permissions → Repository permissions → **Contents: Read and write**
   * Copy the token.
3. Give it to the build: `PEQUOD-HOST` repo → Settings → Secrets and variables → **Actions** → **New repository secret**, Name `RELEASES_TOKEN`, paste the token.

Until the secret is added, builds still make the installer (download it from the run's Artifacts) but don't publish updates, and the run shows a warning saying so.

## Building the installer yourself

With [Inno Setup 6](https://jrsoftware.org/isinfo.php), after a Release build and with the firmware in `fw/`:

```
iscc /DAppVersion=1.0.0 Installer\PEQUOD-Host.iss
```
