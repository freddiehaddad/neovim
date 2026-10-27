#!/bin/bash
set -e

while (($# > 0)); do
  case $1 in
  --test) # install test dependencies
    TEST=1
    shift
    ;;
  esac
done

OS=$(uname -s)
ARCH=$(uname -m)
if [[ $OS == Linux ]]; then
  sudo apt-get update
  sudo apt-get install -y build-essential curl gettext

  if [[ -n $TEST ]]; then
    sudo apt-get install -y locales-all cpanminus attr libattr1-dev fish gdb inotify-tools xdg-utils

    # Use default CC to avoid compilation problems when installing Python modules
    CC=cc python3 -m pip -q install --user --upgrade --break-system-packages pynvim

    # Skip installing npm on aarch64 as it tends to cause intermittent segmentation faults.
    # See https://github.com/neovim/neovim/issues/32339.
    if [[ $ARCH != aarch64 ]]; then
      npm install -g neovim
      npm link neovim
    fi
  fi
elif [[ $OS == Darwin ]]; then
  if [[ -n $TEST ]]; then
    brew untap aws/tap
    brew install cpanminus fish fswatch

    npm install -g neovim
    npm link neovim

    # Use default CC to avoid compilation problems when installing Python modules
    CC=cc python3 -m pip -q install --user --upgrade --break-system-packages pynvim
  fi
fi
