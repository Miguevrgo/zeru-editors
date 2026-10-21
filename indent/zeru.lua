if vim.b.did_indent then
    return
end
vim.b.did_indent = true

vim.bo.indentexpr = "v:lua.ZeruIndent()"
vim.bo.indentkeys = "0{,0},0),0],!^F,o,O,e"
vim.bo.smartindent = false

function ZeruIndent()
    local prev = vim.fn.prevnonblank(vim.v.lnum - 1)
    if prev == 0 then
        return 0
    end
    local indent = vim.fn.indent(prev)
    local before = vim.fn.getline(prev):gsub("%s*//.*$", ""):gsub("%s+$", "")
    if before:match("[{(%[]$") then
        indent = indent + vim.fn.shiftwidth()
    end
    if vim.fn.getline(vim.v.lnum):match("^%s*[})%]]") then
        indent = indent - vim.fn.shiftwidth()
    end
    return math.max(indent, 0)
end
