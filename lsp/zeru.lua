-- The Zeru language server, built from this plugin's server/ directory.
local plugin = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":p:h:h")

return {
  cmd = { plugin .. "/server/target/release/zeru-lsp" },
  filetypes = { "zeru" },
  root_markers = { ".git" },
}
