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
language server, `zeru-lsp`, built from `server/` in this repository:

- syntax errors as you type, and the compiler's own errors when you open or save a file
- hover with signatures and documentation, go to definition, references and rename
- inferred types shown in grey after each name, as `var n = 3` getting `: i32`
- completion with signatures, documentation and argument placeholders; only
  types where a type goes, and module names inside `import std.`
- std modules, types and functions you have not imported yet: accepting
  `HashMap` adds `import std.collections.hashmap::{HashMap};`
- signature help, document symbols, and highlighting by meaning
- formatting with `zeru fmt`, as LazyVim does on save

#### Install

1. Install Zeru with its `install.sh` (see the
   [Zeru README](https://github.com/Miguevrgo/Zeru#-installation): one `curl` line), so `zeru`
   is in `~/.zeru/bin` or on your `PATH`. The compiler's errors need a `zeru`
   that has the `check` command: `zeru --help` lists it. `zeru update` later
   installs the build of Zeru's latest commit.
2. Have Rust installed (`cargo`): the plugin builds its server on install.
3. Add the plugin. With [lazy.nvim](https://github.com/folke/lazy.nvim):

   ```lua
   { "Miguevrgo/zeru-editors", lazy = false, build = "cargo build --release --manifest-path server/Cargo.toml" }
   ```

4. Restart Neovim and open a `.zr` file.

With LazyVim's keys: `K` shows the documentation, `gd` goes to the definition,
`gr` lists references, `<leader>cr` renames, and completion pops up as you type.
`:Lazy update zeru-editors` updates the plugin and rebuilds the server.

#### Optional: blink.cmp

With [blink.cmp](https://github.com/saghen/blink.cmp), as in LazyVim, the
signature window does not open by itself, and completion labels stay white
because Zeru has no tree-sitter grammar. This turns on the window and gives
each label its kind's color, as the icons have:

```lua
{
  "saghen/blink.cmp",
  opts = {
    signature = { enabled = true },
    completion = {
      menu = {
        draw = {
          components = {
            label = {
              highlight = function(ctx)
                local menu = require("blink.cmp.config.completion.menu")
                local highlights = menu.default.draw.components.label.highlight(ctx)
                if vim.bo.filetype == "zeru" and not ctx.deprecated then
                  highlights[1].group = ctx.kind_hl
                end
                return highlights
              end,
            },
          },
        },
      },
    },
  },
}
```

#### If something does not work

- **Everything is white:** lazy is still on an older version of the plugin,
  pinned by `lazy-lock.json`. Run `:Lazy update zeru-editors` and restart.
- **No errors from the compiler, only syntax errors:** your `zeru` has no
  `check` command, or the server cannot find it. Update Zeru, or point the
  server at it (setting `ZERU_PATH` in the environment does the same):

  ```lua
  vim.lsp.config("zeru", { init_options = { zeruPath = "/path/to/zeru" } })
  ```

- **No grey types after names:** inlay hints are off; `:lua vim.lsp.inlay_hint.enable()`
  turns them on.
- **No completion or hover at all:** run `:checkhealth vim.lsp`; the `zeru`
  client should be attached. If the server is missing, run `:Lazy build zeru-editors`.

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
