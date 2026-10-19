# Zeru Editor Support

This repository contains syntax highlighting and editor support for the Zeru programming language across various editors.

## Supported Editors

- [Visual Studio Code](#visual-studio-code)
- [Vim / Neovim](#vim--neovim)
- [Emacs](#emacs)

---

## Visual Studio Code

### Installation

1. Clone this repository or download the source code.
2. Copy the `vscode/` folder to your VS Code extensions directory:
   - **Linux/macOS:** `~/.vscode/extensions/zeru-syntax`
   - **Windows:** `%USERPROFILE%\.vscode\extensions\zeru-syntax`
3. Restart Visual Studio Code.

Alternatively, if you have `vsce` installed, you can package it yourself:
```bash
cd vscode
vsce package
# Then install the resulting .vsix file in VS Code
```

---

## Vim / Neovim

### Neovim (0.11 or newer)

The plugin gives `.zr` files their filetype, highlighting and the Zeru
language server, `zeru-lsp`, built from `server/` in this repository: syntax
errors as you type, the compiler's errors on open and on save, hover, go to
definition, references, rename, completion, signature help, symbols and
highlighting by meaning.

Building it needs Rust (`cargo`). The compiler's errors need `zeru` on your
`PATH` or in `~/.zeru/bin`; everything else works without it.

With [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{ "Miguevrgo/zeru-editors", lazy = false, build = "cargo build --release --manifest-path server/Cargo.toml" }
```

The server is configured as `zeru` through `vim.lsp.config`, so any setting can
be changed from your own config, such as which compiler it runs (`ZERU_PATH`
in the environment does the same):

```lua
vim.lsp.config("zeru", { init_options = { zeruPath = "/path/to/zeru" } })
```

### Vim

Highlighting only. With [vim-plug](https://github.com/junegunn/vim-plug):

```vim
Plug 'Miguevrgo/zeru-editors'
```

Or copy `ftdetect/zeru.vim` and `syntax/zeru.vim` into the same folders under
`~/.vim`.

---

## Emacs

### Installation

1. Add the `emacs/` directory to your Emacs `load-path`:
   ```elisp
   (add-to-list 'load-path "/path/to/zeru-editors/emacs")
   ```
2. Load and require `zeru-mode`:
   ```elisp
   (require 'zeru-mode)
   ```

### Features

- Syntax highlighting for Zeru source files (`.zr`).
- Automatic indentation.
- Commenting support.
