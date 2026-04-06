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

### Manual Installation (Vim)

Copy the contents of the `vim/` directory to your `.vim` configuration folder:

```bash
mkdir -p ~/.vim/syntax ~/.vim/ftdetect
cp vim/syntax/zeru.vim ~/.vim/syntax/
cp vim/ftdetect/zeru.vim ~/.vim/ftdetect/
```

### Manual Installation (Neovim)

Copy the contents of the `vim/` directory to your Neovim configuration folder:

```bash
mkdir -p ~/.config/nvim/syntax ~/.config/nvim/ftdetect
cp vim/syntax/zeru.vim ~/.config/nvim/syntax/
cp vim/ftdetect/zeru.vim ~/.config/nvim/ftdetect/
```

### Using a Plugin Manager (Vim/Neovim)

If you use a plugin manager like [vim-plug](https://github.com/junegunn/vim-plug), add the following to your configuration:

```vim
Plug 'Miguevrgo/zeru-editors', { 'rtp': 'vim' }
```

For Neovim with `lazy.nvim`:

```lua
{
  "Miguevrgo/zeru-editors",
  lazy = false,
  config = function(plugin)
    vim.opt.rtp:append(plugin.dir .. "/vim")
  end,
  init = function()
    vim.api.nvim_create_autocmd({ "BufRead", "BufNewFile" }, {
      pattern = "*.zr",
      command = "set filetype=zeru",
    })
  end,
}
```

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
