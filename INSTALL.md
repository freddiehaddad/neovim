# Installing Nero

You can install **Nero** from [pre-built release archives](#pre-built-releases) or [build from source](#install-from-source) with a single Cargo command.

> [!NOTE]
> System package managers (`winget`, `brew`, `apt`, `pacman`, `dnf`, etc.) distribute upstream C Neovim (`neovim/neovim`). To run Nero, use the release binaries from [`freddiehaddad/nero/releases`](https://github.com/freddiehaddad/nero/releases) or `cargo xtask install`.

---

## Pre-Built Releases

Pre-built binaries for Windows, macOS, and Linux are available on the [Releases](https://github.com/freddiehaddad/nero/releases) page:

- **Latest release**: [`github.com/freddiehaddad/nero/releases/latest`](https://github.com/freddiehaddad/nero/releases/latest)
- **Nightly prerelease**: [`github.com/freddiehaddad/nero/releases/tag/nightly`](https://github.com/freddiehaddad/nero/releases/tag/nightly)

### Windows (`x86_64` & `arm64`)

#### Portable `.zip`

1. Download `nero-win64.zip` (or `nero-win-arm64.zip`) from the [Releases](https://github.com/freddiehaddad/nero/releases) page.
2. Extract it anywhere (no administrator privileges required). `$VIMRUNTIME` resolves automatically relative to `bin\nero.exe`.
3. Add the extracted `bin` directory to your `PATH` and run `nero`.

#### Windows `.msi` Installer

```pwsh
iwr -Uri "https://github.com/freddiehaddad/nero/releases/latest/download/nero-win64.msi" -OutFile nero-win64.msi
msiexec /i nero-win64.msi /passive
Remove-Item nero-win64.msi
```

### macOS (`arm64` & `x86_64`)

```bash
# Apple Silicon (arm64)
curl -LO https://github.com/freddiehaddad/nero/releases/latest/download/nero-macos-arm64.tar.gz
tar xzf nero-macos-arm64.tar.gz
./nero-macos-arm64/bin/nero

# Intel (x86_64)
curl -LO https://github.com/freddiehaddad/nero/releases/latest/download/nero-macos-x86_64.tar.gz
tar xzf nero-macos-x86_64.tar.gz
./nero-macos-x86_64/bin/nero
```

### Linux (`x86_64` & `arm64`)

#### Tarball (`.tar.gz`)

```bash
curl -LO https://github.com/freddiehaddad/nero/releases/latest/download/nero-linux-x86_64.tar.gz
sudo rm -rf /opt/nero-linux-x86_64
sudo tar -C /opt -xzf nero-linux-x86_64.tar.gz
export PATH="$PATH:/opt/nero-linux-x86_64/bin"
```

#### AppImage

```bash
curl -LO https://github.com/freddiehaddad/nero/releases/latest/download/nero-linux-x86_64.appimage
chmod u+x nero-linux-x86_64.appimage
./nero-linux-x86_64.appimage
```

---

## Install from Source

If you have [Rust and a C compiler](./BUILD.md#platform-prerequisites) installed, building and installing from source is one command:

```bash
git clone https://github.com/freddiehaddad/nero
cd nero
cargo xtask install
```

- On Linux, macOS, and BSD, this installs to `~/.local` (`~/.local/bin/nero`).
- On Windows, this installs to `%LOCALAPPDATA%\Programs\Nero` (`%LOCALAPPDATA%\Programs\Nero\bin\nero.exe`).
- To install to a custom directory:
  ```bash
  cargo xtask install --prefix "$HOME/nero"
  export PATH="$HOME/nero/bin:$PATH"
  ```

## Uninstall

Remove the installed binaries and runtime directory from your install prefix:

```bash
rm -f ~/.local/bin/nero ~/.local/bin/tee ~/.local/bin/xxd
rm -rf ~/.local/lib/nvim ~/.local/share/nvim ~/.local/share/man/man1/nero.1
```

