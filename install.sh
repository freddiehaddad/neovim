#!/bin/zsh

# Cleanup old neovim files
rm -rf ~/.local/share/nvim ~/.cache/nvim

# Install packer
git clone --depth 1 https://github.com/wbthomason/packer.nvim ~/.local/share/nvim/site/pack/packer/start/packer.nvim

# Install plugins
nvim --headless -c 'autocmd User PackerComplete quitall' -c 'PackerSync'

# Launch to install/configure language servers
nvim
