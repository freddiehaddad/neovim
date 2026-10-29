# Installing Neovim (Rust Port)

You can install this Rust build of Neovim from [pre-built release archives](#pre-built-releases) or [build from source](#install-from-source) with a single Cargo command.

> [!NOTE]
> System package managers (`winget`, `brew`, `apt`, `pacman`, `dnf`, etc.) distribute upstream C Neovim (`neovim/neovim`). To run this Rust port, use the release binaries from [`freddiehaddad/neovim/releases`](https://github.com/freddiehaddad/neovim/releases) or `cargo xtask install`.

---

## Pre-Built Releases

Pre-built binaries for Windows, macOS, and Linux are available on the [Releases](https://github.com/freddiehaddad/neovim/releases) page:

- **Latest release**: [`github.com/freddiehaddad/neovim/releases/latest`](https://github.com/freddiehaddad/neovim/releases/latest)
- **Nightly prerelease**: [`github.com/freddiehaddad/neovim/releases/tag/nightly`](https://github.com/freddiehaddad/neovim/releases/tag/nightly)

### Windows (`x86_64` & `arm64`)

#### Portable `.zip`

1. Download `nvim-win64.zip` (or `nvim-win-arm64.zip`) from the [Releases](https://github.com/freddiehaddad/neovim/releases) page.
2. Extract it anywhere (no administrator privileges required). `$VIMRUNTIME` resolves automatically relative to `bin\nvim.exe`.
3. Add the extracted `bin` directory to your `PATH` and run `nvim`.

#### Windows `.msi` Installer

```pwsh
iwr -Uri "https://github.com/freddiehaddad/neovim/releases/latest/download/nvim-win64.msi" -OutFile nvim-win64.msi
msiexec /i nvim-win64.msi /passive
Remove-Item nvim-win64.msi
```

### macOS (`arm64` & `x86_64`)

```bash
# Apple Silicon (arm64)
curl -LO https://github.com/freddiehaddad/neovim/releases/latest/download/nvim-macos-arm64.tar.gz
tar xzf nvim-macos-arm64.tar.gz
./nvim-macos-arm64/bin/nvim

# Intel (x86_64)
curl -LO https://github.com/freddiehaddad/neovim/releases/latest/download/nvim-macos-x86_64.tar.gz
tar xzf nvim-macos-x86_64.tar.gz
./nvim-macos-x86_64/bin/nvim
```

### Linux (`x86_64` & `arm64`)

#### Tarball (`.tar.gz`)

```bash
curl -LO https://github.com/freddiehaddad/neovim/releases/latest/download/nvim-linux-x86_64.tar.gz
sudo rm -rf /opt/nvim-linux-x86_64
sudo tar -C /opt -xzf nvim-linux-x86_64.tar.gz
export PATH="$PATH:/opt/nvim-linux-x86_64/bin"
```

#### AppImage

```bash
curl -LO https://github.com/freddiehaddad/neovim/releases/latest/download/nvim-linux-x86_64.appimage
chmod u+x nvim-linux-x86_64.appimage
./nvim-linux-x86_64.appimage
```

---

## Install from Source

If you have [Rust and a C compiler](./BUILD.md#platform-prerequisites) installed, building and installing from source is one command:

```bash
git clone https://github.com/freddiehaddad/neovim
cd neovim
cargo xtask install
```

- On Linux, macOS, and BSD, this installs to `~/.local` (`~/.local/bin/nvim`).
- On Windows, this installs to `%USERPROFILE%\.local\bin\nvim-win64`.
- To install to a custom directory:
  ```bash
  cargo xtask install --prefix "$HOME/neovim"
  export PATH="$HOME/neovim/bin:$PATH"
  ```

## Uninstall

Remove the installed binaries and runtime directory from your install prefix:

```bash
rm -f ~/.local/bin/nvim ~/.local/bin/nvim-tee ~/.local/bin/xxd
rm -rf ~/.local/lib/nvim ~/.local/share/nvim ~/.local/share/man/man1/nvim.1
```

