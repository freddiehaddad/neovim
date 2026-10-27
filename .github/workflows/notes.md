```
${NVIM_VERSION}
```

## Release notes

- [Changelog](https://github.com/freddiehaddad/neovim/commit/${NVIM_COMMIT}) (fixes + features)
- [News](./runtime/doc/news.txt) (`:help news` in Nvim)

## Install

### Windows

#### Zip

1. Download **nvim-win64.zip** (or **nvim-win-arm64.zip** for ARM)
2. Extract the zip
3. Run `bin\nvim.exe` in your terminal

Note: On Windows "Server" you may need to [install `vcruntime*.dll`](https://neovim.io/doc/install/#windows).

### macOS (x86_64)

1. Download **nvim-macos-x86_64.tar.gz**
2. Run `xattr -c ./nvim-macos-x86_64.tar.gz` (to avoid "unknown developer" warning)
3. Extract: `tar xzvf nvim-macos-x86_64.tar.gz`
4. Run `./nvim-macos-x86_64/bin/nvim`

### macOS (arm64)

1. Download **nvim-macos-arm64.tar.gz**
2. Run `xattr -c ./nvim-macos-arm64.tar.gz` (to avoid "unknown developer" warning)
3. Extract: `tar xzvf nvim-macos-arm64.tar.gz`
4. Run `./nvim-macos-arm64/bin/nvim`

### Linux (x86_64)

1. Download **nvim-linux-x86_64.tar.gz**
2. Extract: `tar xzvf nvim-linux-x86_64.tar.gz`
3. Run `./nvim-linux-x86_64/bin/nvim`

### Linux (arm64)

1. Download **nvim-linux-arm64.tar.gz**
2. Extract: `tar xzvf nvim-linux-arm64.tar.gz`
3. Run `./nvim-linux-arm64/bin/nvim`
