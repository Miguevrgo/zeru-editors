-- The Zeru language server, built into the compiler: `zeru lsp`.
local zeru = vim.fn.exepath("zeru")
if zeru == "" then
  zeru = vim.fn.expand("~/.zeru/bin/zeru")
end

return {
  cmd = { zeru, "lsp" },
  filetypes = { "zeru" },
  root_markers = { ".git" },
}
