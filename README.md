# Zeru Editor Support

Highlighting, indentation and a language server for the
[Zeru](https://github.com/Miguevrgo/Zeru) programming language.

| Editor | Highlighting | Indentation | Language server |
|---|---|---|---|
| [Neovim 0.11+](#neovim) | yes | yes | yes, set up by the plugin |
| [Emacs](#emacs) | yes | yes | yes, through eglot |
| [Kakoune](#kakoune) | yes | yes | yes, through kakoune-lsp |
| [Vim](#vim) | yes | no | no |
| [VS Code](#vs-code) | yes | brackets and comments | no |

## The language server

`zeru-lsp`, built from `server/`, gives:

- syntax errors as you type, and the compiler's own errors when a file is opened or saved
- hover with signatures and documentation, go to definition, references and rename
- inferred types shown after each name, as `var n = 3` getting `: i32`, and parameter
  names before arguments
- completion with signatures, documentation and argument placeholders: only types where
  a type goes, module names inside `import std.`, and only what may be called from the
  current file
- std modules, types and functions not imported yet: accepting `HashMap` adds
  `import std.collections.hashmap::{HashMap};`
- signature help, document symbols, highlighting by meaning, and formatting with `zeru fmt`

It needs `zeru` in `~/.zeru/bin` or on the `PATH`, installed with the
[Zeru installer](https://github.com/Miguevrgo/Zeru#-installation), to report the compiler's
errors and to format. `zeru update` installs the latest build. Building the server needs
Rust (`cargo`).

## Neovim

With [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{ "Miguevrgo/zeru-editors", lazy = false, build = "cargo build --release --manifest-path server/Cargo.toml" }
```

Restart Neovim and open a `.zr` file. The plugin sets the filetype, highlighting,
indentation and the language server. With LazyVim's keys, `K` shows the documentation,
`gd` goes to the definition, `gr` lists references, `<leader>cr` renames, and completion
pops up as you type. `:Lazy update zeru-editors` updates the plugin and rebuilds the
server.

### Optional: blink.cmp

With [blink.cmp](https://github.com/saghen/blink.cmp), as in LazyVim, the signature window
does not open by itself, and completion labels stay white because Zeru has no tree-sitter
grammar. This turns the window on and colors each label as its kind's icon:

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

### If something does not work

- **Everything is white:** lazy is still on an older version of the plugin, pinned by
  `lazy-lock.json`. Run `:Lazy update zeru-editors` and restart.
- **Only syntax errors, none from the compiler:** the server cannot find `zeru`. Update
  Zeru, or point the server at it (setting `ZERU_PATH` in the environment does the same):

  ```lua
  vim.lsp.config("zeru", { init_options = { zeruPath = "/path/to/zeru" } })
  ```

- **No types after names:** inlay hints are off; `:lua vim.lsp.inlay_hint.enable()` turns
  them on.
- **No completion or hover at all:** `:checkhealth vim.lsp` should show the `zeru` client
  attached. If the server is missing, run `:Lazy build zeru-editors`.

## Emacs

Clone this repository, build the server, and add to your init file:

```bash
git clone https://github.com/Miguevrgo/zeru-editors ~/zeru-editors
cargo build --release --manifest-path ~/zeru-editors/server/Cargo.toml
```

```elisp
(add-to-list 'load-path "~/zeru-editors/emacs")
(require 'zeru-mode)

(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs
               '(zeru-mode . ("~/zeru-editors/server/target/release/zeru-lsp"))))
(add-hook 'zeru-mode-hook #'eglot-ensure)
```

`zeru-mode` highlights `.zr` files and indents one level per open bracket. The last three
lines start the language server through eglot, built into Emacs 29 and later; without
them the mode works on its own.

## Kakoune

With [plug.kak](https://github.com/andreyorst/plug.kak), in your `kakrc`:

```kak
plug "Miguevrgo/zeru-editors" do %{
    cargo build --release --manifest-path server/Cargo.toml
} config %{
    hook global WinSetOption filetype=zeru %{ lsp-enable-window }
}
```

Run `:plug-install` and open a `.zr` file. It gets highlighting, indentation that
follows `zeru fmt`, and `//` for comment toggling. With
[kakoune-lsp](https://github.com/kakoune-lsp/kakoune-lsp) loaded (`eval %sh{kak-lsp}`),
the `config` line turns on the language server, which the plugin finds where `do` built
it. `:plug-update` updates the plugin and rebuilds the server. Without kakoune-lsp,
leave out the `config` block.

Without plug.kak, clone the repository, build the server, and source the script:

```bash
git clone https://github.com/Miguevrgo/zeru-editors ~/zeru-editors
cargo build --release --manifest-path ~/zeru-editors/server/Cargo.toml
```

```kak
source ~/zeru-editors/kakoune/zeru.kak
hook global WinSetOption filetype=zeru %{ lsp-enable-window }
```

## Vim

Highlighting only. With [vim-plug](https://github.com/junegunn/vim-plug):

```vim
Plug 'Miguevrgo/zeru-editors'
```

Or copy `ftdetect/zeru.vim` and `syntax/zeru.vim` into the same folders under `~/.vim`.

## VS Code

Highlighting, comment toggling and bracket pairs. Copy the `vscode/` folder to
`~/.vscode/extensions/zeru` (`%USERPROFILE%\.vscode\extensions\zeru` on Windows) and
restart VS Code, or package it with `vsce package` inside `vscode/` and install the
`.vsix`.

## Layout

| Path | What it is |
|---|---|
| `server/` | the language server, in Rust |
| `ftdetect/`, `syntax/` | filetype and highlighting for Vim and Neovim |
| `ftplugin/`, `indent/`, `lsp/`, `plugin/` | Neovim settings, indentation and server setup |
| `emacs/` | `zeru-mode` |
| `kakoune/` | highlighting, indentation and server setup for Kakoune |
| `vscode/` | the VS Code extension |
