-- Starts the Zeru language server, and says so when its binary is missing:
-- Neovim skips such a server without a word.
if vim.fn.has("nvim-0.11") == 1 then
  vim.lsp.enable("zeru")

  vim.api.nvim_create_autocmd("FileType", {
    pattern = "zeru",
    once = true,
    callback = function()
      local server = vim.lsp.config.zeru.cmd[1]
      if vim.fn.executable(server) == 0 then
        vim.notify(
          "zeru-lsp is not built, so Zeru files get no language server.\n"
            .. "Build it with :Lazy build zeru-editors, which needs Rust's cargo, or run\n"
            .. "cargo build --release --manifest-path "
            .. vim.fn.fnamemodify(server, ":h:h:h:h")
            .. "/server/Cargo.toml",
          vim.log.levels.WARN
        )
      end
    end,
  })
end
